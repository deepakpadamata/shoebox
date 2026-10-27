//! `Node`: owns the iroh endpoint and router, the blob store and the docs/gossip protocols.

use crate::blobs::{iroh_hash, BlobStore, Protection};
use crate::config::{NodeConfig, RelaySetting};
use crate::keys::load_or_create_secret;
use anyhow::{anyhow, Context, Result};
use iroh::address_lookup::memory::MemoryLookup;
use iroh::endpoint::{
    presets, AfterHandshakeOutcome, BeforeConnectOutcome, Connection, EndpointHooks,
    WeakConnectionHandle,
};
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use iroh::{Endpoint, EndpointId};
use iroh_blobs::api::downloader::DownloadProgressItem;
use iroh_blobs::store::fs::{options::Options as BlobOptions, FsStore};
use iroh_blobs::store::GcConfig;
use iroh_blobs::{BlobsProtocol, ALPN as BLOBS_ALPN};
use iroh_docs::engine::ProtectCallbackHandler;
use iroh_docs::protocol::Docs;
use iroh_docs::ALPN as DOCS_ALPN;
use iroh_gossip::net::Gossip;
use iroh_gossip::ALPN as GOSSIP_ALPN;
use iroh_mdns_address_lookup::MdnsAddressLookup;
use shoebox_core::ids::Blake3Hash;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use tracing::{info, info_span, Instrument};

pub use iroh::EndpointAddr as PeerAddr;

/// Minimum bytes between `BlobProgress` events for one download.
const PROGRESS_STEP: u64 = 256 * 1024;
/// Blobs at least this large are fetched from several holders at once (A-099).
pub const SPLIT_MIN: u64 = 8 * 1024 * 1024;
/// One piece of a split fetch, in 1 KiB chunks: 2 MiB, whole 16 KiB chunk groups.
const PIECE_CHUNKS: u64 = 2048;
/// Holders a split fetch draws from at once.
pub const MAX_SOURCES: usize = 4;
/// How long a split fetch waits to connect to a holder before leaving it out.
const SOURCE_CONNECT: Duration = Duration::from_secs(10);

/// Where the bytes of a fetch came from.
#[derive(Clone, Debug, Default)]
pub struct FetchReport {
    /// Payload bytes each holder sent, most first. Empty when the blob was already here.
    pub sources: Vec<(EndpointId, u64)>,
    pub elapsed: Duration,
}

impl FetchReport {
    /// The holder that sent the most.
    pub fn main(&self) -> Option<EndpointId> {
        self.sources.first().map(|(peer, _)| *peer)
    }
}

/// ALPN of the liveness/latency protocol.
pub const PING_ALPN: &[u8] = b"shoebox/ping/0";
/// ALPN of the pairing protocol (M2-04). Open to devices that are not in the device list yet.
pub const PAIR_ALPN: &[u8] = b"shoebox/pair/0";
/// ALPN of the root-key recovery handshake (M2-02/A-060). Open to devices that are not listed.
pub const RECOVER_ALPN: &[u8] = b"shoebox/recover/0";
/// ALPN a shared-album joiner uses to present its invitation to the inviter (M5). Open to
/// devices that are not listed.
pub const ALBUM_ALPN: &[u8] = b"shoebox/album/0";
/// ALPN of the rotation notice push (M2-03b).
pub const ROTATE_ALPN: &[u8] = b"shoebox/rotate/0";
/// ALPN of the Loro library-store sync protocol (handler plugged in by `shoebox-store-loro`).
pub const LORO_ALPN: &[u8] = b"shoebox/loro/0";
/// ALPN of the "do you hold these blobs?" probe used before freeing space (M2-08).
pub const HAS_ALPN: &[u8] = b"shoebox/has/0";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeEvent {
    Started {
        id: EndpointId,
    },
    PeerConnected {
        id: EndpointId,
    },
    PeerDisconnected {
        id: EndpointId,
    },
    /// Bytes received so far for a blob being fetched from `from`.
    BlobProgress {
        hash: Blake3Hash,
        from: EndpointId,
        bytes: u64,
    },
    /// A blob finished downloading, was verified and is now stored and protected.
    BlobFetched {
        hash: Blake3Hash,
        from: EndpointId,
    },
    /// State of the library sync with one peer.
    SyncProgress {
        peer: EndpointId,
        phase: crate::sync::SyncPhase,
    },
    /// The library moved to a new epoch (a device was revoked). Persist `cap`: it replaces the
    /// capability the library was opened with.
    LibraryRotated {
        cap: shoebox_core::store::LibraryCapability,
    },
    /// A pairing attempt reached the point where both people compare `sas`.
    PairingCode {
        sas: String,
        peer_name: String,
        peer_role: shoebox_core::model::Role,
    },
    PairingDone {
        peer: shoebox_core::ids::EndpointId,
    },
    PairingFailed {
        reason: String,
    },
    /// This device's own revocation reached it: it no longer belongs to the library.
    RemovedFromLibrary,
    /// A device rejoined the library with the recovery phrase through this one (after approval,
    /// where this device asks). It is now an admin.
    RecoveryJoined {
        device: shoebox_core::ids::EndpointId,
        name: String,
    },
    /// A device became an admin of the library that this device had not seen as one before
    /// (it can add and remove devices). Raised on every device once it has synced at least
    /// once, so a new admin made with a leaked recovery phrase does not go unnoticed.
    AdminAdded {
        device: shoebox_core::ids::EndpointId,
        name: String,
    },
    /// This device followed a root handover: the library's recovery phrase was replaced on
    /// another device, and the old phrase no longer opens it.
    RecoveryPhraseReplaced,
    /// This device's certified role changed (an admin or the recovery phrase changed it); the
    /// node already follows it. Persist it where the app keeps its settings.
    RoleChanged {
        role: shoebox_core::model::Role,
    },
    /// A device of the library came online or went offline (from the presence topic).
    PresenceChanged {
        device: shoebox_core::ids::EndpointId,
        online: bool,
    },
    /// Progress of an import run (counts of files processed so far, by outcome).
    ImportProgress {
        done: usize,
        total: usize,
        imported: usize,
        duplicates: usize,
        skipped: usize,
        failed: usize,
    },
}

/// The kind of network path currently used to reach a peer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathKind {
    /// Same local network (private, link-local or loopback address).
    Lan,
    /// Direct over the internet (hole-punched or publicly reachable).
    Direct,
    /// Through a relay server.
    Relay { url: String },
    /// No path in use (not connected, or not known yet).
    Unknown,
}

#[derive(Clone, Debug)]
pub struct PeerInfo {
    pub id: EndpointId,
    pub path: PathKind,
    pub latency: Option<Duration>,
    pub last_seen: Option<Instant>,
}

fn is_local_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => v4.is_private() || v4.is_loopback() || v4.is_link_local(),
        std::net::IpAddr::V6(v6) => {
            let first = v6.segments()[0];
            v6.is_loopback() || (first & 0xffc0) == 0xfe80 || (first & 0xfe00) == 0xfc00
        }
    }
}

/// Summarise the addresses in use for a peer: a local-network path beats a direct one, which
/// beats a relay.
pub fn classify_addrs(addrs: &[iroh::TransportAddr]) -> PathKind {
    let mut direct = false;
    let mut relay = None;
    for addr in addrs {
        match addr {
            iroh::TransportAddr::Ip(sock) if is_local_ip(sock.ip()) => return PathKind::Lan,
            iroh::TransportAddr::Ip(_) => direct = true,
            iroh::TransportAddr::Relay(url) => relay = Some(url.to_string()),
            _ => {}
        }
    }
    if direct {
        PathKind::Direct
    } else if let Some(url) = relay {
        PathKind::Relay { url }
    } else {
        PathKind::Unknown
    }
}

/// A subscription to node events. A new subscriber always sees [`NodeEvent::Started`] first.
pub struct NodeEvents {
    started: Option<NodeEvent>,
    rx: broadcast::Receiver<NodeEvent>,
}

impl NodeEvents {
    pub async fn recv(&mut self) -> Result<NodeEvent, broadcast::error::RecvError> {
        if let Some(ev) = self.started.take() {
            return Ok(ev);
        }
        self.rx.recv().await
    }
}

/// Emits peer events from connection handshakes (both directions, every ALPN) and collapses
/// the several connections a peer holds into one connected/disconnected pair.
#[derive(Debug, Clone)]
struct PeerTracker {
    tx: broadcast::Sender<NodeEvent>,
    open: Arc<Mutex<HashMap<EndpointId, usize>>>,
    /// Peers we refuse to talk to (used to simulate partitions and, later, revoked devices).
    blocked: Arc<Mutex<HashSet<EndpointId>>>,
    /// When set, only these peers may connect (the library's valid device list, M2-03);
    /// [`PAIR_ALPN`] stays open so new devices can be paired.
    allowed: Arc<Mutex<Option<HashSet<EndpointId>>>>,
    /// Members of shared albums (M5): outside the device list, so the only library-side thing
    /// they may do is fetch blobs (whose hashes they know from the album).
    album_peers: Arc<Mutex<HashSet<EndpointId>>>,
    conns: Arc<Mutex<HashMap<EndpointId, Vec<WeakConnectionHandle>>>>,
    /// Last time a connection with the peer was established or closed.
    last_seen: Arc<Mutex<HashMap<EndpointId, Instant>>>,
}

impl PeerTracker {
    fn new(tx: broadcast::Sender<NodeEvent>) -> Self {
        Self {
            tx,
            open: Default::default(),
            blocked: Default::default(),
            allowed: Default::default(),
            album_peers: Default::default(),
            conns: Default::default(),
            last_seen: Default::default(),
        }
    }

    fn is_blocked(&self, id: &EndpointId) -> bool {
        self.blocked.lock().unwrap().contains(id)
    }

    /// Blocked, or not on the allow-list (when there is one), for a connection speaking `alpn`.
    fn refuses(&self, id: &EndpointId, alpn: &[u8]) -> bool {
        if self.is_blocked(id) {
            return true;
        }
        match &*self.allowed.lock().unwrap() {
            Some(allowed) => !allowed.contains(id) && !self.open_to_outsiders(id, alpn),
            None => false,
        }
    }

    /// What a peer outside the device list may still do. Pairing and recovery are how devices
    /// get in. Namespace sync and gossip are guarded by the namespace capability, which only
    /// shared albums use. Blob transfer is open only to members of a shared album.
    fn open_to_outsiders(&self, id: &EndpointId, alpn: &[u8]) -> bool {
        alpn == PAIR_ALPN
            || alpn == RECOVER_ALPN
            || alpn == ALBUM_ALPN
            || alpn == crate::doc_digest::DOC_DIGEST_ALPN
            || alpn == DOCS_ALPN
            || alpn == GOSSIP_ALPN
            || (alpn == BLOBS_ALPN && self.album_peers.lock().unwrap().contains(id))
    }

    fn set_allowed(&self, allowed: Option<HashSet<EndpointId>>) {
        *self.allowed.lock().unwrap() = allowed.clone();
        let Some(allowed) = allowed else { return };
        let mut conns = self.conns.lock().unwrap();
        conns.retain(|id, list| {
            if allowed.contains(id) {
                return true;
            }
            let mut keep = false;
            for conn in list.iter().filter_map(WeakConnectionHandle::upgrade) {
                // Pairing connections of not-yet-listed devices, and what shared albums need,
                // are left alone.
                if self.open_to_outsiders(id, conn.alpn()) {
                    keep = true;
                } else {
                    conn.close(1u32.into(), b"not in the device list");
                }
            }
            keep
        });
    }

    fn set_album_peers(&self, ids: HashSet<EndpointId>) {
        let dropped: Vec<EndpointId> = {
            let mut peers = self.album_peers.lock().unwrap();
            let dropped = peers.difference(&ids).copied().collect();
            *peers = ids;
            dropped
        };
        // Someone left the albums: end what they had open that only the albums justified.
        let listed = self.allowed.lock().unwrap().clone();
        let mut conns = self.conns.lock().unwrap();
        for id in dropped {
            if listed.as_ref().is_none_or(|l| l.contains(&id)) {
                continue;
            }
            if let Some(list) = conns.get_mut(&id) {
                for conn in list.iter().filter_map(WeakConnectionHandle::upgrade) {
                    if !self.open_to_outsiders(&id, conn.alpn()) {
                        conn.close(1u32.into(), b"no longer in a shared album");
                    }
                }
            }
        }
    }

    fn block(&self, id: EndpointId) {
        self.blocked.lock().unwrap().insert(id);
        let conns = self.conns.lock().unwrap().remove(&id).unwrap_or_default();
        for conn in conns.iter().filter_map(WeakConnectionHandle::upgrade) {
            conn.close(1u32.into(), b"blocked");
        }
    }

    fn unblock(&self, id: &EndpointId) {
        self.blocked.lock().unwrap().remove(id);
    }
}

impl EndpointHooks for PeerTracker {
    async fn before_connect<'a>(
        &'a self,
        remote_addr: &'a iroh::EndpointAddr,
        alpn: &'a [u8],
    ) -> BeforeConnectOutcome {
        if self.refuses(&remote_addr.id, alpn) {
            BeforeConnectOutcome::Reject
        } else {
            BeforeConnectOutcome::Accept
        }
    }

    async fn after_handshake<'a>(&'a self, conn: &'a Connection) -> AfterHandshakeOutcome {
        let id = conn.remote_id();
        if self.refuses(&id, conn.alpn()) {
            return AfterHandshakeOutcome::Reject {
                error_code: 1u32.into(),
                reason: b"blocked".to_vec(),
            };
        }
        self.conns
            .lock()
            .unwrap()
            .entry(id)
            .or_default()
            .push(conn.weak_handle());
        self.last_seen.lock().unwrap().insert(id, Instant::now());
        let first = {
            let mut open = self.open.lock().unwrap();
            let n = open.entry(id).or_default();
            *n += 1;
            *n == 1
        };
        if first {
            let _ = self.tx.send(NodeEvent::PeerConnected { id });
        }
        let closed = conn.weak_handle().closed();
        let this = self.clone();
        tokio::spawn(async move {
            closed.await;
            this.last_seen.lock().unwrap().insert(id, Instant::now());
            let last = {
                let mut open = this.open.lock().unwrap();
                let n = open.get_mut(&id).expect("counted on handshake");
                *n -= 1;
                let last = *n == 0;
                if last {
                    open.remove(&id);
                }
                last
            };
            if last {
                let _ = this.tx.send(NodeEvent::PeerDisconnected { id });
            }
        });
        AfterHandshakeOutcome::accept()
    }
}

/// Echoes one short message back; used for reachability and round-trip time.
#[derive(Debug, Clone)]
struct Ping;

impl ProtocolHandler for Ping {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let (mut send, mut recv) = connection.accept_bi().await?;
        let msg = recv.read_to_end(64).await.map_err(AcceptError::from_err)?;
        send.write_all(&msg).await.map_err(AcceptError::from_err)?;
        send.finish()?;
        // The peer closes once it has read the reply; do not wait on it for longer than that.
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), connection.closed()).await;
        Ok(())
    }
}

struct Inner {
    config: Mutex<NodeConfig>,
    id: EndpointId,
    endpoint: Endpoint,
    router: Router,
    blobs: BlobStore,
    activity: crate::activity::Activity,
    docs: Docs,
    gossip: Gossip,
    lookup: MemoryLookup,
    events: broadcast::Sender<NodeEvent>,
    tracker: PeerTracker,
    /// The endpoint secret, kept to sign device certificates as an admin (M2-04).
    secret_seed: [u8; 32],
    pub(crate) pair_slot: crate::pairing::PairSlot,
    loro_slot: crate::ext::ExtSlot,
    rotate_slot: crate::ext::ExtSlot,
    recover_slot: crate::ext::ExtSlot,
    album_slot: crate::ext::ExtSlot,
    /// Most recent measured round-trip time per peer.
    rtts: Mutex<HashMap<EndpointId, Duration>>,
}

/// A running Shoebox node. Cheap to clone; all clones share one runtime.
#[derive(Clone)]
pub struct Node {
    inner: Arc<Inner>,
}

impl Node {
    /// Open (creating on first use) the node rooted at `config.data_dir`.
    pub async fn open(config: NodeConfig) -> Result<Node> {
        Self::open_inner(config, None).await
    }

    /// Like [`Node::open`], but with the endpoint secret supplied by the caller (for example
    /// from the platform keystore) instead of the key file under `keys/`.
    pub async fn open_with_secret(config: NodeConfig, secret: [u8; 32]) -> Result<Node> {
        Self::open_inner(config, Some(secret)).await
    }

    async fn open_inner(mut config: NodeConfig, secret: Option<[u8; 32]>) -> Result<Node> {
        config.ensure_dirs().context("creating data dir layout")?;
        // Watched folders, pins and the keep rules are state the person builds up at runtime, not
        // something callers pass in: keep what an earlier run saved. (Pins were once lost here, so
        // a restart forgot them: free-up could then release a pinned original.)
        if let Ok(saved) = NodeConfig::load(&config.data_dir) {
            if config.watched_folders.is_empty() {
                config.watched_folders = saved.watched_folders;
            }
            if config.pinned_originals.is_empty() {
                config.pinned_originals = saved.pinned_originals;
            }
            config.keep_recent_days = saved.keep_recent_days;
            config.keep_favorites = saved.keep_favorites;
        }
        config.save().context("writing config.toml")?;
        let secret = match secret {
            Some(bytes) => iroh::SecretKey::from_bytes(&bytes),
            None => load_or_create_secret(&config.key_path())?,
        };
        let secret_seed = secret.to_bytes();
        let pair_slot = crate::pairing::PairSlot::default();
        let loro_slot = crate::ext::ExtSlot::default();
        let rotate_slot = crate::ext::ExtSlot::default();
        let recover_slot = crate::ext::ExtSlot::default();
        let album_slot = crate::ext::ExtSlot::default();
        let id = secret.public();
        let span = info_span!("node", id = %id.fmt_short(), name = %config.name);
        async move {
            let (events, _) = broadcast::channel(256);
            let lookup = MemoryLookup::new();
            let tracker = PeerTracker::new(events.clone());
            let mut builder = match &config.relay {
                RelaySetting::Disabled => Endpoint::builder(presets::Minimal),
                RelaySetting::Default => Endpoint::builder(presets::N0),
                RelaySetting::Custom(url) => {
                    let url: iroh::RelayUrl = url
                        .parse()
                        .with_context(|| format!("invalid relay URL `{url}`"))?;
                    Endpoint::builder(presets::Minimal)
                        .relay_mode(iroh::RelayMode::Custom(iroh::RelayMap::from(url)))
                }
            };
            if config.mdns {
                // Find devices on the same network without any server (works offline).
                builder = builder.address_lookup(MdnsAddressLookup::builder());
            }
            let endpoint = builder
                .secret_key(secret)
                .address_lookup(lookup.clone())
                .hooks(tracker.clone())
                .bind()
                .await
                .context("binding endpoint")?;

            // GC reclaims every untagged blob; docs content hashes are protected through the
            // protect handler so metadata entries are never collected from under the library.
            let mut blob_opts = BlobOptions::new(&config.blobs_dir());
            let (protect_handler, docs_cb) = ProtectCallbackHandler::new();
            let activity = crate::activity::Activity::default();
            // Skip a GC pass while the node is busy: GC can race with in-flight data.
            let quiet = Duration::from_millis(config.gc_quiet_ms);
            let guard_activity = activity.clone();
            let protect_cb: iroh_blobs::store::ProtectCb = Arc::new(move |live| {
                let docs_cb = docs_cb.clone();
                let busy = guard_activity.busy_within(quiet);
                Box::pin(async move {
                    if busy {
                        iroh_blobs::store::ProtectOutcome::Abort
                    } else {
                        docs_cb(live).await
                    }
                })
            });
            blob_opts.gc = config.gc_interval_ms.map(|ms| GcConfig {
                interval: Duration::from_millis(ms),
                add_protected: Some(protect_cb),
            });
            let raw_blobs = FsStore::load_with_opts(config.blobs_dir().join("blobs.db"), blob_opts)
                .await
                .map_err(|e| anyhow!("opening blob store: {e}"))?;
            let blobs = BlobStore::new(raw_blobs, activity.clone());
            let gossip = Gossip::builder().spawn(endpoint.clone());
            let docs_dir = config.store_dir().join("docs");
            std::fs::create_dir_all(&docs_dir)?;
            let docs = Docs::persistent(docs_dir)
                .protect_handler(protect_handler)
                .spawn(endpoint.clone(), (**blobs.raw()).clone(), gossip.clone())
                .await
                .map_err(|e| anyhow!("starting docs: {e}"))?;
            let router = Router::builder(endpoint.clone())
                .accept(BLOBS_ALPN, BlobsProtocol::new(blobs.raw(), None))
                .accept(GOSSIP_ALPN, gossip.clone())
                .accept(DOCS_ALPN, docs.clone())
                .accept(PING_ALPN, Ping)
                .accept(RECOVER_ALPN, crate::ext::ExtProtocol(recover_slot.clone()))
                .accept(ALBUM_ALPN, crate::ext::ExtProtocol(album_slot.clone()))
                .accept(ROTATE_ALPN, crate::ext::ExtProtocol(rotate_slot.clone()))
                .accept(LORO_ALPN, crate::ext::ExtProtocol(loro_slot.clone()))
                .accept(
                    HAS_ALPN,
                    crate::freeup::HasProtocol {
                        blobs: blobs.clone(),
                    },
                )
                .accept(PAIR_ALPN, crate::pairing::PairProtocol(pair_slot.clone()))
                .accept(
                    crate::doc_digest::DOC_DIGEST_ALPN,
                    crate::doc_digest::DigestProtocol { docs: docs.clone() },
                )
                .spawn();
            info!("node started");
            let _ = events.send(NodeEvent::Started { id });
            Ok(Node {
                inner: Arc::new(Inner {
                    config: Mutex::new(config),
                    id,
                    endpoint,
                    router,
                    blobs,
                    activity,
                    docs,
                    gossip,
                    lookup,
                    events,
                    tracker,
                    secret_seed,
                    pair_slot,
                    loro_slot,
                    rotate_slot,
                    recover_slot,
                    album_slot,
                    rtts: Mutex::new(HashMap::new()),
                }),
            })
        }
        .instrument(span)
        .await
    }

    pub fn id(&self) -> EndpointId {
        self.inner.id
    }

    /// This device's signing key (the endpoint key), for certificates it issues as an admin.
    pub fn device_key(&self) -> shoebox_core::identity::KeyPair {
        shoebox_core::identity::KeyPair::from_seed(self.inner.secret_seed)
    }

    pub(crate) fn recover_slot(&self) -> &crate::ext::ExtSlot {
        &self.inner.recover_slot
    }

    /// Where a library node registers its rotation-notice handler.
    pub(crate) fn rotate_slot(&self) -> &crate::ext::ExtSlot {
        &self.inner.rotate_slot
    }

    /// Where the shared-album manager registers its invitation handler.
    pub fn album_slot(&self) -> &crate::ext::ExtSlot {
        &self.inner.album_slot
    }

    /// Where the Loro backend registers its connection handler.
    pub fn loro_slot(&self) -> &crate::ext::ExtSlot {
        &self.inner.loro_slot
    }

    pub(crate) fn inner_pair_slot(&self) -> &crate::pairing::PairSlot {
        &self.inner.pair_slot
    }

    /// A snapshot of the current configuration.
    pub fn config(&self) -> NodeConfig {
        self.inner.config.lock().unwrap().clone()
    }

    /// Change the configuration and persist it to `config.toml`.
    pub fn update_config(&self, f: impl FnOnce(&mut NodeConfig)) -> Result<()> {
        let mut config = self.inner.config.lock().unwrap();
        f(&mut config);
        config.save()
    }

    /// Record that real work is happening, so blob GC steps aside (see [`crate::Activity`]).
    pub fn note_activity(&self) {
        self.inner.activity.touch();
    }

    pub(crate) fn emit(&self, event: NodeEvent) {
        let _ = self.inner.events.send(event);
    }

    /// How other nodes can reach this one right now.
    pub fn addr(&self) -> PeerAddr {
        self.inner.endpoint.addr()
    }

    /// Teach this node how to reach a peer (no discovery service needed).
    pub fn add_peer_addr(&self, addr: PeerAddr) {
        self.inner.lookup.add_endpoint_info(addr);
    }

    pub fn endpoint(&self) -> &Endpoint {
        &self.inner.endpoint
    }
    pub fn blobs(&self) -> &BlobStore {
        &self.inner.blobs
    }
    pub fn docs(&self) -> &Docs {
        &self.inner.docs
    }
    pub fn gossip(&self) -> &Gossip {
        &self.inner.gossip
    }

    /// Open a gossip connection to `peer` ourselves and hand it to gossip. iroh-gossip 0.101 dials
    /// a peer once: if that dial fails, the peer stays "pending" and later messages to it queue
    /// forever. A connection handed in this way takes over and sends what was queued.
    pub async fn gossip_dial(&self, peer: iroh::EndpointId) -> Result<()> {
        let conn = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            self.inner.endpoint.connect(peer, GOSSIP_ALPN),
        )
        .await
        .map_err(|_| anyhow!("timed out"))??;
        self.inner.gossip.handle_connection(conn).await?;
        Ok(())
    }

    /// Refuse all connections to and from `id` and drop the current ones.
    pub fn block_peer(&self, id: EndpointId) {
        self.inner.tracker.block(id);
    }

    pub fn unblock_peer(&self, id: EndpointId) {
        self.inner.tracker.unblock(&id);
    }

    pub fn is_blocked(&self, id: EndpointId) -> bool {
        self.inner.tracker.is_blocked(&id)
    }

    /// Only let `allowed` connect (and close other connections), or lift the restriction with
    /// `None`. The pairing protocol stays reachable by everyone.
    /// The members of this device's shared albums: they may fetch blobs although they are not
    /// in the library's device list. Replaces the previous set; takes effect on new connections.
    pub fn set_album_peers(&self, ids: HashSet<EndpointId>) {
        self.inner.tracker.set_album_peers(ids);
    }

    pub fn set_allowed_peers(&self, allowed: Option<HashSet<EndpointId>>) {
        self.inner.tracker.set_allowed(allowed);
    }

    /// Would a (non-pairing) connection with `id` be refused right now?
    pub fn is_refused(&self, id: EndpointId) -> bool {
        self.inner.tracker.refuses(&id, b"")
    }

    pub fn events(&self) -> NodeEvents {
        NodeEvents {
            started: Some(NodeEvent::Started { id: self.inner.id }),
            rx: self.inner.events.subscribe(),
        }
    }

    /// Ask `peer` for the digest of one of its iroh-docs namespaces (see [`crate::doc_digest`]).
    /// `Ok(None)` if the peer does not have the namespace open.
    pub async fn peer_doc_digest(
        &self,
        peer: EndpointId,
        namespace: iroh_docs::NamespaceId,
    ) -> Result<Option<[u8; 32]>> {
        let conn = self
            .inner
            .endpoint
            .connect(peer, crate::doc_digest::DOC_DIGEST_ALPN)
            .await
            .context("connecting")?;
        let (mut send, mut recv) = conn.open_bi().await?;
        send.write_all(&namespace.to_bytes()).await?;
        send.finish()?;
        let reply = recv.read_to_end(64).await?;
        conn.close(0u32.into(), b"done");
        match reply.as_slice() {
            [1, digest @ ..] if digest.len() == 32 => {
                Ok(Some(digest.try_into().expect("32 bytes")))
            }
            [0] => Ok(None),
            _ => Err(anyhow!("malformed digest reply")),
        }
    }

    /// What this node knows about the connection to `id`: the kind of path in use, the last
    /// measured latency and when the peer was last seen. `None` if the peer is unknown.
    pub async fn peer_info(&self, id: EndpointId) -> Option<PeerInfo> {
        let remote = self.inner.endpoint.remote_info(id).await;
        let last_seen = self
            .inner
            .tracker
            .last_seen
            .lock()
            .unwrap()
            .get(&id)
            .copied();
        let latency = self.inner.rtts.lock().unwrap().get(&id).copied();
        if remote.is_none() && last_seen.is_none() {
            return None;
        }
        let active: Vec<iroh::TransportAddr> = remote
            .map(|r| {
                r.into_addrs()
                    .filter(|a| matches!(a.usage(), iroh::endpoint::TransportAddrUsage::Active))
                    .map(|a| a.into_addr())
                    .collect()
            })
            .unwrap_or_default();
        Some(PeerInfo {
            id,
            path: classify_addrs(&active),
            latency,
            last_seen,
        })
    }

    /// Round-trip time to `id` over the ping protocol (also proves reachability).
    pub async fn ping(&self, id: EndpointId) -> Result<Duration> {
        let conn = self
            .inner
            .endpoint
            .connect(id, PING_ALPN)
            .await
            .context("connecting")?;
        let start = Instant::now();
        let (mut send, mut recv) = conn.open_bi().await?;
        send.write_all(b"ping").await?;
        send.finish()?;
        let echo = recv.read_to_end(64).await?;
        anyhow::ensure!(echo == b"ping", "unexpected ping reply");
        let rtt = start.elapsed();
        conn.close(0u32.into(), b"done");
        self.inner.rtts.lock().unwrap().insert(id, rtt);
        Ok(rtt)
    }

    /// Download `hash` from `from`, verifying it as it streams in, then protect it with `why`.
    /// Emits [`NodeEvent::BlobProgress`] while receiving and [`NodeEvent::BlobFetched`] at the end.
    pub async fn fetch_blob(
        &self,
        from: EndpointId,
        hash: &Blake3Hash,
        why: Protection,
    ) -> Result<()> {
        self.fetch_blob_from(&[from], hash, None, why).await?;
        Ok(())
    }

    /// [`Self::download_blob_from`], then protect the blob with `why`.
    pub async fn fetch_blob_from(
        &self,
        holders: &[EndpointId],
        hash: &Blake3Hash,
        size: Option<u64>,
        why: Protection,
    ) -> Result<FetchReport> {
        let report = self.download_blob_from(holders, hash, size).await?;
        self.inner.blobs.protect(hash, why).await?;
        Ok(report)
    }

    /// Download `hash` from `from` and verify it, without keeping it under any protection of its
    /// own (for blobs something else already keeps alive, such as iroh-docs entry content).
    pub async fn download_blob(&self, from: EndpointId, hash: &Blake3Hash) -> Result<()> {
        self.download_blob_from(&[from], hash, None).await?;
        Ok(())
    }

    /// Download `hash` from any of `holders` (best first) and verify it, without protecting it.
    ///
    /// A blob of known `size` of at least [`SPLIT_MIN`] with more than one holder is fetched in
    /// pieces from up to [`MAX_SOURCES`] holders at once (A-099): each holder takes the next
    /// missing piece, so a faster one takes more, and a piece a holder fails on goes back for
    /// another. Whatever is still missing after that, and every smaller blob, comes from one
    /// holder at a time, each taking over where the last stopped. Every piece is verified against
    /// the hash as it arrives, whichever holder sent it.
    pub async fn download_blob_from(
        &self,
        holders: &[EndpointId],
        hash: &Blake3Hash,
        size: Option<u64>,
    ) -> Result<FetchReport> {
        anyhow::ensure!(!holders.is_empty(), "no holder to fetch {hash} from");
        self.note_activity();
        if self.inner.blobs.has(hash).await? {
            return Ok(FetchReport::default());
        }
        // An in-flight download has no tag yet; hold a temp tag so GC cannot reap the partial or
        // freshly completed blob before the permanent protection exists.
        let _in_flight = self
            .inner
            .blobs
            .raw()
            .tags()
            .temp_tag(iroh_blobs::HashAndFormat {
                hash: iroh_hash(hash),
                format: iroh_blobs::BlobFormat::Raw,
            })
            .await
            .map_err(|e| anyhow!("protecting in-flight download: {e}"))?;
        let started = Instant::now();
        let mut received: HashMap<EndpointId, u64> = HashMap::new();
        if let Some(size) = size.filter(|s| holders.len() > 1 && *s >= SPLIT_MIN) {
            for (peer, bytes) in self.download_split(holders, hash, size).await {
                *received.entry(peer).or_default() += bytes;
            }
        }
        if !self.inner.blobs.has(hash).await? {
            let (peer, bytes) = self.download_one_at_a_time(holders, hash).await?;
            *received.entry(peer).or_default() += bytes;
        }
        let mut sources: Vec<(EndpointId, u64)> = received.into_iter().collect();
        sources.sort_by_key(|(_, bytes)| std::cmp::Reverse(*bytes));
        let report = FetchReport {
            sources,
            elapsed: started.elapsed(),
        };
        if report.sources.len() > 1 {
            info!(
                %hash,
                sources = ?report.sources.iter().map(|(p, b)| (p.fmt_short().to_string(), *b)).collect::<Vec<_>>(),
                ms = report.elapsed.as_millis() as u64,
                "fetched from several holders"
            );
        }
        if let Some(from) = report.main() {
            let _ = self
                .inner
                .events
                .send(NodeEvent::BlobFetched { hash: *hash, from });
        }
        Ok(report)
    }

    /// The pieces of a split fetch, each holder taking the next one. Returns the payload bytes
    /// each holder sent; leaves anything not received for the caller.
    async fn download_split(
        &self,
        holders: &[EndpointId],
        hash: &Blake3Hash,
        size: u64,
    ) -> Vec<(EndpointId, u64)> {
        use iroh_blobs::protocol::{ChunkRanges, ChunkRangesExt, GetRequest};
        let root = iroh_hash(hash);
        let chunks = size.div_ceil(1024);
        // The last piece is open-ended: the holder's proof of the size ends it.
        let pieces: std::collections::VecDeque<(u64, Option<u64>)> = (0..chunks)
            .step_by(PIECE_CHUNKS as usize)
            .map(|a| (a, (a + PIECE_CHUNKS < chunks).then_some(a + PIECE_CHUNKS)))
            .collect();
        // Pieces waiting, and how many are being fetched: when both are zero the split is over,
        // even if a holder is still trying to connect.
        let queue = Arc::new(Mutex::new((pieces, 0usize)));
        let finished = Arc::new(tokio::sync::Notify::new());
        let total = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let got_by: Arc<Mutex<HashMap<EndpointId, u64>>> = Arc::default();
        let workers = holders.iter().take(MAX_SOURCES).map(|&peer| {
            let (node, queue, total) = (self.clone(), queue.clone(), total.clone());
            let (finished, got_by) = (finished.clone(), got_by.clone());
            async move {
                let conn = match tokio::time::timeout(
                    SOURCE_CONNECT,
                    node.inner.endpoint.connect(peer, BLOBS_ALPN),
                )
                .await
                {
                    Ok(Ok(conn)) => conn,
                    _ => return,
                };
                let remote = node.inner.blobs.raw().remote().clone();
                // Hand a piece back (`retry`) or mark it done; the last one ends the split.
                let settle = |piece: (u64, Option<u64>), retry: bool| {
                    let mut q = queue.lock().unwrap();
                    q.1 -= 1;
                    if retry {
                        q.0.push_back(piece);
                    } else if q.0.is_empty() && q.1 == 0 {
                        finished.notify_one();
                    }
                };
                loop {
                    let next = {
                        let mut q = queue.lock().unwrap();
                        let next = q.0.pop_front();
                        if next.is_some() {
                            q.1 += 1;
                        }
                        next
                    };
                    let Some((a, b)) = next else {
                        break;
                    };
                    let ranges = match b {
                        Some(b) => ChunkRanges::chunks(a..b),
                        None => ChunkRanges::chunks(a..),
                    };
                    let piece = Arc::new(GetRequest::builder().root(ranges).build(root));
                    let missing = match remote.local_for_request(piece.clone()).await {
                        Ok(local) if local.is_complete() => {
                            settle((a, b), false);
                            continue;
                        }
                        Ok(local) => local.missing(),
                        Err(_) => {
                            settle((a, b), true);
                            break;
                        }
                    };
                    let done = match remote.execute_get(conn.clone(), missing).complete().await {
                        Ok(stats) => {
                            *got_by.lock().unwrap().entry(peer).or_default() +=
                                stats.payload_bytes_read;
                            let bytes = total.fetch_add(stats.payload_bytes_read, Ordering::Relaxed)
                                + stats.payload_bytes_read;
                            node.note_activity();
                            let _ = node.inner.events.send(NodeEvent::BlobProgress {
                                hash: *hash,
                                from: peer,
                                bytes,
                            });
                            // A holder with only part of the blob sends what it has: not done.
                            matches!(remote.local_for_request(piece).await, Ok(l) if l.is_complete())
                        }
                        Err(e) => {
                            tracing::debug!(peer = %peer.fmt_short(), "piece of {hash} failed: {e:#}");
                            false
                        }
                    };
                    settle((a, b), !done);
                    if !done {
                        // Another holder takes it; this one has stopped being useful.
                        break;
                    }
                }
                conn.close(0u32.into(), b"done");
            }
        });
        tokio::select! {
            _ = futures::future::join_all(workers) => {}
            _ = finished.notified() => {}
        }
        let got = got_by.lock().unwrap().clone();
        got.into_iter().filter(|(_, n)| *n > 0).collect()
    }

    /// One holder at a time (iroh-blobs' downloader), each continuing from what is already here.
    /// Returns the holder that finished it and the bytes received in this call.
    async fn download_one_at_a_time(
        &self,
        holders: &[EndpointId],
        hash: &Blake3Hash,
    ) -> Result<(EndpointId, u64)> {
        let remote = self.inner.blobs.raw().remote().clone();
        let before = remote
            .local(iroh_hash(hash))
            .await
            .map(|l| l.local_bytes())
            .unwrap_or(0);
        let downloader = self.inner.blobs.raw().downloader(&self.inner.endpoint);
        let mut progress = downloader
            .download(iroh_hash(hash), holders.to_vec())
            .stream()
            .await
            .map_err(|e| anyhow!("starting download of {hash}: {e}"))?;
        let mut current = holders[0];
        let mut failure: Option<String> = None;
        let mut failed: Vec<String> = vec![];
        let mut last_reported = 0u64;
        while let Some(item) = futures::StreamExt::next(&mut progress).await {
            match item {
                DownloadProgressItem::TryProvider { id, .. } => current = id,
                DownloadProgressItem::Progress(bytes) => {
                    self.note_activity();
                    // Throttle: a large blob reports thousands of chunks; a UI wants a few updates.
                    if bytes >= last_reported + PROGRESS_STEP {
                        last_reported = bytes;
                        let _ = self.inner.events.send(NodeEvent::BlobProgress {
                            hash: *hash,
                            from: current,
                            bytes,
                        });
                    }
                }
                DownloadProgressItem::Error(e) => failure = Some(e.to_string()),
                DownloadProgressItem::DownloadError => {
                    failure.get_or_insert_with(|| "download failed".into());
                }
                DownloadProgressItem::ProviderFailed { id, .. } => {
                    failed.push(id.fmt_short().to_string());
                }
                _ => {}
            }
        }
        let who = if holders.len() == 1 {
            holders[0].fmt_short().to_string()
        } else {
            format!("{} holders", holders.len())
        };
        if !self.inner.blobs.has(hash).await? {
            let reason = match (failure, failed.is_empty()) {
                (Some(reason), _) => reason,
                (None, false) => format!("{} could not provide the blob", failed.join(", ")),
                (None, true) => "download did not complete".into(),
            };
            anyhow::bail!("fetching {hash} from {who}: {reason}");
        }
        let after = remote
            .local(iroh_hash(hash))
            .await
            .map(|l| l.local_bytes())
            .unwrap_or(before);
        Ok((current, after.saturating_sub(before)))
    }

    /// Stop all protocols, close the endpoint and release the on-disk stores.
    pub async fn shutdown(&self) -> Result<()> {
        self.inner
            .router
            .shutdown()
            .await
            .map_err(|e| anyhow!("router shutdown: {e}"))?;
        // The blobs protocol shuts its store down with the router; a second call finds the actor
        // already gone, which is fine (the restart test proves the store is released).
        let _ = self.inner.blobs.raw().shutdown().await;
        Ok(())
    }
}
