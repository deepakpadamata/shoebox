//! `Node`: owns the iroh endpoint and router, the blob store and the docs/gossip protocols.

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
use iroh_blobs::store::fs::FsStore;
use iroh_blobs::{BlobsProtocol, ALPN as BLOBS_ALPN};
use iroh_docs::protocol::Docs;
use iroh_docs::ALPN as DOCS_ALPN;
use iroh_gossip::net::Gossip;
use iroh_gossip::ALPN as GOSSIP_ALPN;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use tracing::{info, info_span, Instrument};

pub use iroh::EndpointAddr as PeerAddr;

/// ALPN of the liveness/latency protocol.
pub const PING_ALPN: &[u8] = b"shoebox/ping/0";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeEvent {
    Started { id: EndpointId },
    PeerConnected { id: EndpointId },
    PeerDisconnected { id: EndpointId },
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
    conns: Arc<Mutex<HashMap<EndpointId, Vec<WeakConnectionHandle>>>>,
}

impl PeerTracker {
    fn new(tx: broadcast::Sender<NodeEvent>) -> Self {
        Self {
            tx,
            open: Default::default(),
            blocked: Default::default(),
            conns: Default::default(),
        }
    }

    fn is_blocked(&self, id: &EndpointId) -> bool {
        self.blocked.lock().unwrap().contains(id)
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
        _alpn: &'a [u8],
    ) -> BeforeConnectOutcome {
        if self.is_blocked(&remote_addr.id) {
            BeforeConnectOutcome::Reject
        } else {
            BeforeConnectOutcome::Accept
        }
    }

    async fn after_handshake<'a>(&'a self, conn: &'a Connection) -> AfterHandshakeOutcome {
        let id = conn.remote_id();
        if self.is_blocked(&id) {
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
        connection.closed().await;
        Ok(())
    }
}

struct Inner {
    config: NodeConfig,
    id: EndpointId,
    endpoint: Endpoint,
    router: Router,
    blobs: FsStore,
    docs: Docs,
    gossip: Gossip,
    lookup: MemoryLookup,
    events: broadcast::Sender<NodeEvent>,
    tracker: PeerTracker,
}

/// A running Shoebox node. Cheap to clone; all clones share one runtime.
#[derive(Clone)]
pub struct Node {
    inner: Arc<Inner>,
}

impl Node {
    /// Open (creating on first use) the node rooted at `config.data_dir`.
    pub async fn open(config: NodeConfig) -> Result<Node> {
        config.ensure_dirs().context("creating data dir layout")?;
        config.save().context("writing config.toml")?;
        let secret = load_or_create_secret(&config.key_path())?;
        let id = secret.public();
        let span = info_span!("node", id = %id.fmt_short(), name = %config.name);
        async move {
            let (events, _) = broadcast::channel(256);
            let lookup = MemoryLookup::new();
            let tracker = PeerTracker::new(events.clone());
            let builder = match config.relay {
                RelaySetting::Disabled => Endpoint::builder(presets::Minimal),
                RelaySetting::Default => Endpoint::builder(presets::N0),
            };
            let endpoint = builder
                .secret_key(secret)
                .address_lookup(lookup.clone())
                .hooks(tracker.clone())
                .bind()
                .await
                .context("binding endpoint")?;

            let blobs = FsStore::load(config.blobs_dir())
                .await
                .map_err(|e| anyhow!("opening blob store: {e}"))?;
            let gossip = Gossip::builder().spawn(endpoint.clone());
            let docs_dir = config.store_dir().join("docs");
            std::fs::create_dir_all(&docs_dir)?;
            let docs = Docs::persistent(docs_dir)
                .spawn(endpoint.clone(), (*blobs).clone(), gossip.clone())
                .await
                .map_err(|e| anyhow!("starting docs: {e}"))?;
            let router = Router::builder(endpoint.clone())
                .accept(BLOBS_ALPN, BlobsProtocol::new(&blobs, None))
                .accept(GOSSIP_ALPN, gossip.clone())
                .accept(DOCS_ALPN, docs.clone())
                .accept(PING_ALPN, Ping)
                .spawn();
            info!("node started");
            let _ = events.send(NodeEvent::Started { id });
            Ok(Node {
                inner: Arc::new(Inner {
                    config,
                    id,
                    endpoint,
                    router,
                    blobs,
                    docs,
                    gossip,
                    lookup,
                    events,
                    tracker,
                }),
            })
        }
        .instrument(span)
        .await
    }

    pub fn id(&self) -> EndpointId {
        self.inner.id
    }

    pub fn config(&self) -> &NodeConfig {
        &self.inner.config
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
    pub fn blobs(&self) -> &FsStore {
        &self.inner.blobs
    }
    pub fn docs(&self) -> &Docs {
        &self.inner.docs
    }
    pub fn gossip(&self) -> &Gossip {
        &self.inner.gossip
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

    pub fn events(&self) -> NodeEvents {
        NodeEvents {
            started: Some(NodeEvent::Started { id: self.inner.id }),
            rx: self.inner.events.subscribe(),
        }
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
        Ok(rtt)
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
        let _ = self.inner.blobs.shutdown().await;
        Ok(())
    }
}
