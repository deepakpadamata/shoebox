//! `AppCore`: everything the desktop UI can ask for, with no UI toolkit in sight.
//!
//! One `AppCore` owns the `Node`, the open library (if any) and the small `app.json` that
//! remembers which library this device belongs to. The Tauri shell and the dev HTTP bridge are
//! thin adapters over its methods, so they behave identically and the logic is tested here.

use crate::keystore::SecretStore;
use crate::types::*;
use anyhow::{anyhow, bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use shoebox_core::clock::SystemClock;
use shoebox_core::identity::KeyPair;
use shoebox_core::ids::{Blake3Hash, EndpointId, Timestamp};
use shoebox_core::merge::Lww;
use shoebox_core::model::{AlbumId, AlbumMeta, BlobKind, PhotoRecord, Role};
use shoebox_core::store::{Cursor, LibraryStore, Page, StoreBackend, TimeRange};
use shoebox_node::{
    DecideFn, FetcherConfig, ImportOptions, LibraryNode, LibraryTicket, MaintenanceConfig, Node,
    NodeConfig, NodeEvent, PairTicket, PathKind, PresenceConfig, RelaySetting,
};
use shoebox_store::{HybridBackend, Library};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{broadcast, oneshot};

const PAIRING_ANSWER_TIMEOUT: Duration = Duration::from_secs(120);

/// How the app is set up; the defaults are what a user's machine gets.
#[derive(Clone, Debug)]
pub struct AppOptions {
    pub relay: RelaySetting,
    pub mdns: bool,
    /// Where the endpoint secret lives. `None` = a file under the data dir (dev/tests).
    pub secrets: Option<Arc<dyn SecretStore>>,
    /// Start the fetcher, maintenance and presence loops (off in some tests).
    pub background: bool,
    /// Where to look for a newer release (R1-07): the site's `release.json`. The placeholder (the
    /// default until the site exists, or `SHOEBOX_UPDATE_FEED`) is never contacted.
    pub update_feed: String,
    /// How often the blob store collects unprotected blobs (`None`: the node's default). Tests
    /// shorten it.
    pub gc_interval: Option<Duration>,
}

fn default_update_feed() -> String {
    std::env::var("SHOEBOX_UPDATE_FEED")
        .unwrap_or_else(|_| shoebox_node::update::PLACEHOLDER_FEED.to_string())
}

/// How often the app looks for a newer release while it runs.
const UPDATE_CHECK_EVERY: Duration = Duration::from_secs(12 * 60 * 60);

impl Default for AppOptions {
    fn default() -> Self {
        Self {
            relay: RelaySetting::Default,
            mdns: true,
            secrets: None,
            background: true,
            update_feed: default_update_feed(),
            // Released blobs (a deleted gallery photo's reference, a copy turned into a
            // reference) are collected on the next pass: keep the wait short.
            gc_interval: Some(Duration::from_secs(20)),
        }
    }
}

impl AppOptions {
    /// No relay, no LAN discovery: loopback only, for tests.
    pub fn local_only() -> Self {
        Self {
            relay: RelaySetting::Disabled,
            mdns: false,
            secrets: None,
            background: true,
            update_feed: shoebox_node::update::PLACEHOLDER_FEED.to_string(),
            gc_interval: None,
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
struct AppFile {
    device_name: String,
    role: Option<String>,
    /// Unacknowledged new-admin alerts (see `SecurityAlert`).
    #[serde(default)]
    alerts: Vec<crate::types::SecurityAlert>,
    /// Look for a newer release now and then (R1-07); `None` = yes.
    #[serde(default)]
    update_checks: Option<bool>,
    /// The release whose "new version" banner was dismissed.
    #[serde(default)]
    dismissed_update: Option<String>,
    /// `LibraryTicket` text: the library capability plus first peers.
    ticket: Option<String>,
    relay: Option<String>,
}

fn parse_role(s: &str) -> Result<Role> {
    Ok(match s.to_ascii_lowercase().as_str() {
        "keeper" => Role::Keeper,
        "companion" | "phone" => Role::Companion,
        "viewer" => Role::Viewer,
        other => bail!("unknown role `{other}`"),
    })
}

fn role_name(r: Role) -> &'static str {
    match r {
        Role::Keeper => "keeper",
        Role::Companion => "companion",
        Role::Viewer => "viewer",
    }
}

pub(crate) fn parse_hash(s: &str) -> Result<Blake3Hash> {
    let bytes: [u8; 32] = hex::decode(s.trim())
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| anyhow!("`{s}` is not a photo id"))?;
    Ok(Blake3Hash(bytes))
}

fn parse_device(s: &str) -> Result<EndpointId> {
    let bytes: [u8; 32] = hex::decode(s.trim())
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| anyhow!("`{s}` is not a device id"))?;
    Ok(EndpointId(bytes))
}

fn parse_album(s: &str) -> Result<AlbumId> {
    let bytes: [u8; 16] = hex::decode(s.trim())
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| anyhow!("`{s}` is not an album id"))?;
    Ok(AlbumId(bytes))
}

fn kind_of(s: &str) -> Result<BlobKind> {
    Ok(match s {
        "thumb" => BlobKind::Thumb,
        "preview" => BlobKind::Preview,
        "original" => BlobKind::Original,
        other => bail!("unknown blob kind `{other}`"),
    })
}

fn kind_name(k: BlobKind) -> &'static str {
    match k {
        BlobKind::Thumb => "thumb",
        BlobKind::Preview => "preview",
        BlobKind::Original => "original",
    }
}

fn relay_setting(s: &str) -> RelaySetting {
    match s {
        "off" => RelaySetting::Disabled,
        "default" | "" => RelaySetting::Default,
        url => RelaySetting::Custom(url.to_string()),
    }
}

fn relay_text(r: &RelaySetting) -> String {
    match r {
        RelaySetting::Disabled => "off".into(),
        RelaySetting::Default => "default".into(),
        RelaySetting::Custom(u) => u.clone(),
    }
}

/// The answer to "do the codes match?": it may arrive before the question is registered (the
/// UI reacts to the event as soon as it is emitted), so an early answer is kept.
#[derive(Default)]
enum PairingSlot {
    #[default]
    Idle,
    Waiting(oneshot::Sender<bool>),
    Early(bool),
}

type PendingAnswer = Mutex<PairingSlot>;

/// A recovery through this device waiting for its person's answer.
struct PendingRecovery {
    question: crate::types::RecoveryQuestion,
    answer: oneshot::Sender<bool>,
}

pub struct AppCore {
    dir: PathBuf,
    opts: AppOptions,
    node: Node,
    library: tokio::sync::RwLock<Option<LibraryNode<Library>>>,
    pub(crate) shared: tokio::sync::RwLock<Option<shoebox_store::SharedAlbums>>,
    /// Following the library's list of shared albums, and sharing what is added to shared
    /// personal albums (A-097): stopped when the library closes.
    followers: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    /// Where the UI loads video originals over HTTP (`media.rs`); `None`: through `shoebox://`.
    media_base: Mutex<Option<String>>,
    file: Mutex<AppFile>,
    events: broadcast::Sender<AppEvent>,
    pending_pairing: PendingAnswer,
    pending_recovery: Mutex<Option<PendingRecovery>>,
    /// Devices this person let in (a recovery approved here) or made admin here: no alert for those.
    approved_recoveries: Mutex<Vec<shoebox_core::ids::EndpointId>>,
    /// A newer release the last check found.
    update: Mutex<Option<crate::types::UpdateInfo>>,
    pub(crate) photo_source: Mutex<Option<Arc<dyn crate::photos_source::PhotoSource>>>,
    pub(crate) backup: tokio::sync::Mutex<crate::backup::BackupState>,
    pub(crate) backup_running: tokio::sync::Mutex<()>,
    /// Which photo each blob belongs to, and the fetch queue's counts, for [`AppCore::transfers`].
    transfer_cache: tokio::sync::Mutex<TransferCache>,
}

/// Rebuilt at most this often: the Devices screen asks every second or two.
const TRANSFER_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(5);

#[derive(Default)]
struct TransferCache {
    blobs: HashMap<Blake3Hash, (Blake3Hash, BlobKind)>,
    blobs_at: Option<std::time::Instant>,
    /// The fetch queue: blob, kind and size.
    waiting: Vec<(Blake3Hash, BlobKind, u64)>,
    waiting_at: Option<std::time::Instant>,
}

impl AppCore {
    /// Open (creating on first use) the app's data directory and start the node. If this device
    /// already belongs to a library it is opened too.
    pub async fn open(dir: impl Into<PathBuf>, opts: AppOptions) -> Result<Arc<AppCore>> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)?;
        // Older data is upgraded (after a copy), newer data is refused: see `shoebox_node::format`.
        shoebox_node::format::ensure_format(&dir)?;
        let file: AppFile = match std::fs::read(dir.join("app.json")) {
            Ok(b) => serde_json::from_slice(&b).context("reading app.json")?,
            Err(_) => adopt_dev_library(&dir)?.unwrap_or_default(),
        };
        if !dir.join("app.json").exists() && file.ticket.is_some() {
            std::fs::write(dir.join("app.json"), serde_json::to_vec_pretty(&file)?)?;
        }
        let mut config =
            NodeConfig::local_only(dir.join("node"), name_or_default(&file.device_name));
        config.role = file
            .role
            .as_deref()
            .map(parse_role)
            .transpose()?
            .unwrap_or(Role::Keeper);
        config.relay = file
            .relay
            .as_deref()
            .map(relay_setting)
            .unwrap_or_else(|| opts.relay.clone());
        config.mdns = opts.mdns;
        if let Some(every) = opts.gc_interval {
            config = config
                .with_gc_interval(every)
                .with_gc_quiet((every / 2).min(Duration::from_secs(10)));
        }
        let node = match &opts.secrets {
            Some(store) => {
                // A device that already belongs to a library must keep its identity: a missing
                // key is an error to explain, not a reason to make a new device.
                if file.ticket.is_some() && store.load("endpoint-key")?.is_none() {
                    bail!(
                        "This device's key is missing, so it cannot rejoin its library as the same device. \
                         Restore it with the recovery phrase (\"I lost my devices\" when starting fresh)."
                    );
                }
                let secret = store.load_or_create("endpoint-key")?;
                Node::open_with_secret(config, secret).await?
            }
            None => Node::open(config).await?,
        };
        let (events, _) = broadcast::channel(512);
        let backup_state = crate::backup::BackupState::load(&dir);
        let core = Arc::new(AppCore {
            dir,
            opts,
            node,
            library: tokio::sync::RwLock::new(None),
            shared: tokio::sync::RwLock::new(None),
            followers: Mutex::new(vec![]),
            media_base: Mutex::new(None),
            file: Mutex::new(file),
            events,
            pending_pairing: Mutex::new(PairingSlot::Idle),
            pending_recovery: Mutex::new(None),
            approved_recoveries: Mutex::new(vec![]),
            update: Mutex::new(None),
            photo_source: Mutex::new(None),
            backup: tokio::sync::Mutex::new(backup_state),
            backup_running: tokio::sync::Mutex::new(()),
            transfer_cache: tokio::sync::Mutex::new(TransferCache::default()),
        });
        core.forward_node_events();
        core.start_update_checks();
        let ticket = core.file.lock().unwrap().ticket.clone();
        if let Some(text) = ticket {
            let ticket: LibraryTicket = text
                .parse()
                .map_err(|e| anyhow!("corrupt ticket in app.json: {e}"))?;
            core.attach(ticket).await?;
        }
        Ok(core)
    }

    pub fn node(&self) -> &Node {
        &self.node
    }

    pub fn data_dir(&self) -> &Path {
        &self.dir
    }

    /// Subscribe to what happens in the app.
    pub fn events(&self) -> broadcast::Receiver<AppEvent> {
        self.events.subscribe()
    }

    pub(crate) fn emit(&self, e: AppEvent) {
        let _ = self.events.send(e);
    }

    fn save(&self) -> Result<()> {
        let file = self.file.lock().unwrap();
        let tmp = self.dir.join("app.json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&*file)?)?;
        std::fs::rename(tmp, self.dir.join("app.json"))?;
        Ok(())
    }

    /// Stop everything and release the data directory.
    pub async fn shutdown(&self) -> Result<()> {
        for h in self.followers.lock().unwrap().drain(..) {
            h.abort();
        }
        *self.shared.write().await = None;
        *self.library.write().await = None;
        self.node.shutdown().await
    }

    /// Replace the recovery phrase (the library's root key), for when the current one may have
    /// leaked. Every current member follows; the old phrase opens nothing any more. What comes
    /// back is the only copy of the new kit.
    pub async fn replace_recovery_phrase(&self, phrase: &str) -> Result<CreatedLibrary> {
        let ln = self.ln().await?;
        let kit = ln.rotate_root(phrase.trim()).await?;
        Ok(CreatedLibrary {
            phrase: kit.phrase(),
            kit_text: kit.to_text(),
            library_id: hex::encode(ln.library_id().0),
        })
    }

    /// Look for a newer release every 12 hours while the app runs (R1-07), unless turned off or
    /// no feed exists yet.
    fn start_update_checks(self: &Arc<Self>) {
        if self.opts.update_feed == shoebox_node::update::PLACEHOLDER_FEED {
            return;
        }
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            loop {
                let Some(core) = weak.upgrade() else { break };
                if core.update_checks_enabled() {
                    let _ = core.check_for_update().await;
                }
                drop(core);
                tokio::time::sleep(UPDATE_CHECK_EVERY).await;
            }
        });
    }

    fn update_checks_enabled(&self) -> bool {
        self.file.lock().unwrap().update_checks.unwrap_or(true)
    }

    /// Ask the release feed now. `Some` when a newer version than this one is out.
    pub async fn check_for_update(&self) -> Result<Option<crate::types::UpdateInfo>> {
        let found = shoebox_node::update::check(&self.opts.update_feed)
            .await?
            .map(|r| crate::types::UpdateInfo {
                version: r.version,
                page: r.page,
            });
        let changed = {
            let mut current = self.update.lock().unwrap();
            let changed = *current != found;
            *current = found.clone();
            changed
        };
        if changed {
            self.emit(AppEvent::SecurityChanged);
        }
        Ok(found)
    }

    /// Hide the banner for this release (a later one shows again).
    pub fn dismiss_update(&self) -> Result<()> {
        let version = self
            .update
            .lock()
            .unwrap()
            .as_ref()
            .map(|u| u.version.clone());
        self.file.lock().unwrap().dismissed_update = version;
        self.save()?;
        self.emit(AppEvent::SecurityChanged);
        Ok(())
    }

    pub fn set_update_checks(&self, on: bool) -> Result<()> {
        self.file.lock().unwrap().update_checks = Some(on);
        if !on {
            *self.update.lock().unwrap() = None;
        }
        self.save()?;
        self.emit(AppEvent::SecurityChanged);
        Ok(())
    }

    /// Version, build and where the source is (Settings → About).
    pub fn about(&self) -> crate::types::AboutInfo {
        crate::types::AboutInfo {
            version: env!("CARGO_PKG_VERSION").into(),
            commit: env!("SHOEBOX_COMMIT").into(),
            source_url: env!("CARGO_PKG_REPOSITORY").into(),
        }
    }

    /// The third-party licence notices bundled with the app (generated by `scripts/licences.py`).
    pub fn third_party_licences(&self) -> &'static str {
        include_str!("../../THIRD-PARTY-LICENSES.txt")
    }

    /// The link another device needs to recover through this one (`shoebox-peer1…`).
    pub fn device_link(&self) -> String {
        shoebox_node::PeerLink(shoebox_node::from_endpoint_addr(&self.node.addr())).to_string()
    }

    pub(crate) fn device_name(&self) -> String {
        self.file.lock().unwrap().device_name.clone()
    }

    pub(crate) async fn ln(&self) -> Result<LibraryNode<Library>> {
        self.library
            .read()
            .await
            .clone()
            .ok_or_else(|| anyhow!("no library yet"))
    }

    // ---------- events ----------

    fn forward_node_events(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        let mut events = self.node.events();
        tokio::spawn(async move {
            loop {
                let ev = match events.recv().await {
                    Ok(ev) => ev,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                };
                let Some(core) = weak.upgrade() else { break };
                let out = match ev {
                    NodeEvent::PeerConnected { id } => Some(AppEvent::PeerConnected {
                        id: hex::encode(id.as_bytes()),
                    }),
                    NodeEvent::PeerDisconnected { id } => Some(AppEvent::PeerDisconnected {
                        id: hex::encode(id.as_bytes()),
                    }),
                    NodeEvent::SyncProgress { peer, phase } => Some(AppEvent::Sync {
                        peer: hex::encode(peer.as_bytes()),
                        phase: format!("{phase:?}"),
                    }),
                    NodeEvent::BlobFetched { hash, .. } => Some(AppEvent::Fetched {
                        photo: Some(hash.to_string()),
                        kind: "blob".into(),
                    }),
                    NodeEvent::ImportProgress { done, total, .. } => {
                        Some(AppEvent::ImportProgress {
                            done: done as u32,
                            total: total as u32,
                        })
                    }
                    NodeEvent::PairingCode {
                        sas,
                        peer_name,
                        peer_role,
                    } => Some(AppEvent::PairingCode {
                        sas,
                        peer_name,
                        peer_role: role_name(peer_role).into(),
                    }),
                    NodeEvent::PairingDone { .. } => {
                        *core.pending_pairing.lock().unwrap() = PairingSlot::Idle;
                        Some(AppEvent::PairingDone)
                    }
                    NodeEvent::PairingFailed { reason } => {
                        // Whoever was asked to compare the code need not answer any more.
                        *core.pending_pairing.lock().unwrap() = PairingSlot::Idle;
                        Some(AppEvent::PairingFailed { reason })
                    }
                    NodeEvent::RemovedFromLibrary => Some(AppEvent::RemovedFromLibrary),
                    NodeEvent::RecoveryPhraseReplaced => {
                        {
                            let mut f = core.file.lock().unwrap();
                            f.alerts.retain(|a| a.kind != "phrase_replaced");
                            f.alerts.push(crate::types::SecurityAlert {
                                kind: "phrase_replaced".into(),
                                device_id: String::new(),
                                device_name: String::new(),
                                at_ms: now_ms() as f64,
                                notified: false,
                            });
                        }
                        let _ = core.save();
                        Some(AppEvent::SecurityChanged)
                    }
                    NodeEvent::AdminAdded { device, name } => {
                        let approved_here =
                            core.approved_recoveries.lock().unwrap().contains(&device);
                        if approved_here {
                            None
                        } else {
                            let id = hex::encode(device.0);
                            {
                                let mut f = core.file.lock().unwrap();
                                f.alerts.retain(|a| a.device_id != id);
                                f.alerts.push(crate::types::SecurityAlert {
                                    kind: "admin_added".into(),
                                    device_id: id,
                                    device_name: name,
                                    at_ms: now_ms() as f64,
                                    notified: false,
                                });
                            }
                            let _ = core.save();
                            Some(AppEvent::SecurityChanged)
                        }
                    }
                    NodeEvent::RoleChanged { role } => {
                        // The node already follows it; keep it for the next start.
                        core.file.lock().unwrap().role = Some(role_name(role).into());
                        let _ = core.save();
                        Some(AppEvent::DevicesChanged)
                    }
                    NodeEvent::PresenceChanged { .. } => Some(AppEvent::DevicesChanged),
                    NodeEvent::LibraryRotated { cap } => {
                        // Remember the new capability so a restart rejoins the new epoch.
                        let peers = core
                            .file
                            .lock()
                            .unwrap()
                            .ticket
                            .as_deref()
                            .and_then(|t| t.parse::<LibraryTicket>().ok())
                            .map(|t| t.peers)
                            .unwrap_or_default();
                        core.file.lock().unwrap().ticket =
                            Some(LibraryTicket { cap, peers }.to_string());
                        let _ = core.save();
                        Some(AppEvent::LibraryRotated)
                    }
                    _ => None,
                };
                if let Some(out) = out {
                    core.emit(out);
                }
            }
        });
    }

    fn forward_library_changes(self: &Arc<Self>, ln: &LibraryNode<Library>) {
        let weak = Arc::downgrade(self);
        let mut changes = ln.store().subscribe();
        tokio::spawn(async move {
            use shoebox_core::store::Change;
            while let Some(change) = futures::StreamExt::next(&mut changes).await {
                let Some(core) = weak.upgrade() else { break };
                core.emit(match change {
                    Change::DeviceListChanged => AppEvent::DevicesChanged,
                    _ => AppEvent::LibraryChanged,
                });
            }
        });
    }

    fn forward_shared_changes(self: &Arc<Self>, shared: &shoebox_store::SharedAlbums) {
        let weak = Arc::downgrade(self);
        let mut notices = shared.subscribe();
        tokio::spawn(async move {
            while let Ok(shoebox_store::AlbumNotice(id)) = notices.recv().await {
                let Some(core) = weak.upgrade() else { break };
                core.emit(AppEvent::SharedChanged {
                    album: hex::encode(id.0),
                });
            }
        });
    }

    // ---------- library lifecycle ----------

    async fn attach(self: &Arc<Self>, ticket: LibraryTicket) -> Result<()> {
        let backend = HybridBackend::for_node(self.node.clone(), SystemClock);
        let library = backend.open_library(&ticket.cap).await?;
        let ln = LibraryNode::new(self.node.clone(), Arc::new(library), SystemClock)
            .with_capability(ticket.cap.clone());
        for peer in &ticket.peers {
            ln.add_peer(peer.clone())?;
        }
        ln.enforce_membership().await?;
        // Ask before letting a recovery through. With nobody to answer (the headless Android
        // worker) the question times out and the recovery is refused.
        ln.set_recovery_approval(Some(self.recovery_decider()));
        if self.opts.background {
            ln.start_fetcher(FetcherConfig::default());
            ln.start_maintenance(MaintenanceConfig::default());
            ln.start_presence(PresenceConfig::default()).await?;
        }
        self.forward_library_changes(&ln);
        let shared =
            shoebox_store::SharedAlbums::open(self.node.clone(), &self.dir, self.opts.background)
                .await?;
        self.forward_shared_changes(&shared);
        if self.opts.background {
            let weak = Arc::downgrade(self);
            let follow =
                shoebox_store::library_link::follow(shared.clone(), ln.clone(), move || {
                    weak.upgrade().map(|c| c.device_name()).unwrap_or_default()
                });
            let weak = Arc::downgrade(self);
            let linked = tokio::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                    let Some(core) = weak.upgrade() else { break };
                    if let Err(e) = core.sync_linked_albums().await {
                        tracing::debug!("sharing new photos of shared albums: {e:#}");
                    }
                }
            });
            self.followers.lock().unwrap().extend([follow, linked]);
        } else {
            shared.set_person(ln.person_id());
        }
        *self.shared.write().await = Some(shared);
        *self.library.write().await = Some(ln);
        self.adopt_local_posters();
        Ok(())
    }

    fn remember(&self, ticket: &LibraryTicket, name: &str, role: Role) -> Result<()> {
        {
            let mut f = self.file.lock().unwrap();
            f.device_name = name.to_string();
            f.role = Some(role_name(role).into());
            f.ticket = Some(ticket.to_string());
        }
        self.node.update_config(|c| {
            c.name = name.to_string();
            c.role = role;
        })?;
        self.save()
    }

    /// Create a new library on this device. The root key exists only for the duration of this
    /// call: what comes back (phrase and printable kit) is the only copy.
    pub async fn create_library(
        self: &Arc<Self>,
        device_name: &str,
        role: &str,
    ) -> Result<CreatedLibrary> {
        ensure!(
            self.library.read().await.is_none(),
            "this device already has a library"
        );
        let role = parse_role(role)?;
        let root = KeyPair::generate();
        let backend = HybridBackend::for_node(self.node.clone(), SystemClock);
        let (library, cap) = backend.create_library(&root.root_public()).await?;
        let ln = LibraryNode::new(self.node.clone(), Arc::new(library), SystemClock)
            .with_capability(cap.clone());
        let kit = ln.create_identity(&root, role, device_name).await?;
        drop(root);
        let ticket = LibraryTicket {
            cap: cap.clone(),
            peers: vec![],
        };
        self.remember(&ticket, device_name, role)?;
        drop(ln);
        self.attach(ticket).await?;
        Ok(CreatedLibrary {
            phrase: kit.phrase(),
            kit_text: kit.to_text(),
            library_id: hex::encode(cap.library.0),
        })
    }

    /// Join a library with a pairing ticket another device is showing. `confirm_pairing` must be
    /// called with the answer once the six-digit code was compared (see `AppEvent::PairingCode`).
    pub async fn join(self: &Arc<Self>, ticket: &str, device_name: &str) -> Result<()> {
        ensure!(
            self.library.read().await.is_none(),
            "this device already has a library"
        );
        let ticket: PairTicket = ticket.parse().map_err(|e| anyhow!("{e}"))?;
        let role = ticket.role;
        let grant = self
            .node
            .pair_join(&ticket, device_name, self.pairing_decider())
            .await?;
        let library_ticket = LibraryTicket {
            cap: grant.cap,
            peers: grant.peers,
        };
        self.remember(&library_ticket, device_name, role)?;
        self.attach(library_ticket).await?;
        Ok(())
    }

    /// Rejoin a library with the recovery phrase through a surviving device (`shoebox-peer1…`), as
    /// `role` (a phone rejoins as a companion, not a keeper that fetches every original).
    pub async fn recover(
        self: &Arc<Self>,
        link: &str,
        phrase: &str,
        device_name: &str,
        role: &str,
    ) -> Result<()> {
        ensure!(
            self.library.read().await.is_none(),
            "this device already has a library"
        );
        let role = parse_role(role)?;
        let link: shoebox_node::PeerLink = link.parse().map_err(|e| anyhow!("{e}"))?;
        let now = now_ms();
        let grant = self
            .node
            .recover(&link, phrase, role, device_name, now)
            .await?;
        let ticket = LibraryTicket {
            cap: grant.cap,
            peers: grant.peers,
        };
        self.remember(&ticket, device_name, role)?;
        self.attach(ticket).await?;
        Ok(())
    }

    fn pairing_decider(self: &Arc<Self>) -> DecideFn {
        let weak = Arc::downgrade(self);
        Arc::new(move |_code| {
            let weak = weak.clone();
            Box::pin(async move {
                let Some(core) = weak.upgrade() else {
                    return false;
                };
                let (tx, rx) = oneshot::channel();
                {
                    let mut slot = core.pending_pairing.lock().unwrap();
                    if let PairingSlot::Early(answer) = *slot {
                        *slot = PairingSlot::Idle;
                        return answer;
                    }
                    *slot = PairingSlot::Waiting(tx);
                }
                // The code itself reaches the UI through `AppEvent::PairingCode`.
                matches!(
                    tokio::time::timeout(PAIRING_ANSWER_TIMEOUT, rx).await,
                    Ok(Ok(true))
                )
            })
        })
    }

    fn recovery_decider(self: &Arc<Self>) -> shoebox_node::RecoveryDecideFn {
        let weak = Arc::downgrade(self);
        Arc::new(move |req: shoebox_node::RecoveryRequest| {
            let weak = weak.clone();
            Box::pin(async move {
                let Some(core) = weak.upgrade() else {
                    return false;
                };
                let (tx, rx) = oneshot::channel();
                *core.pending_recovery.lock().unwrap() = Some(PendingRecovery {
                    question: crate::types::RecoveryQuestion {
                        device_id: hex::encode(req.device.0),
                        device_name: req.name.clone(),
                        role: role_name(req.role).into(),
                    },
                    answer: tx,
                });
                core.emit(AppEvent::SecurityChanged);
                // The node gives up after two minutes; so does the question.
                let allowed = matches!(rx.await, Ok(true));
                if allowed {
                    core.approved_recoveries.lock().unwrap().push(req.device);
                }
                if core
                    .pending_recovery
                    .lock()
                    .unwrap()
                    .as_ref()
                    .is_some_and(|p| p.question.device_id == hex::encode(req.device.0))
                {
                    *core.pending_recovery.lock().unwrap() = None;
                }
                core.emit(AppEvent::SecurityChanged);
                allowed
            })
        })
    }

    /// Answer the recovery waiting on this device (`accept`: let that device in as an admin).
    pub fn answer_recovery(&self, accept: bool) -> Result<()> {
        let pending = self.pending_recovery.lock().unwrap().take();
        let pending = pending.filter(|p| !p.answer.is_closed()).ok_or_else(|| {
            anyhow!("no recovery is waiting for an answer (it may have timed out)")
        })?;
        let _ = pending.answer.send(accept);
        Ok(())
    }

    /// Alerts no system notification was shown for yet, marked as shown now (R1-06): the caller
    /// shows them (Android's background worker, the desktop tray app with its window hidden).
    pub fn take_unnotified_alerts(&self) -> Vec<crate::types::SecurityAlert> {
        let taken: Vec<_> = {
            let mut f = self.file.lock().unwrap();
            let taken = f.alerts.iter().filter(|a| !a.notified).cloned().collect();
            for a in f.alerts.iter_mut() {
                a.notified = true;
            }
            taken
        };
        if !taken.is_empty() {
            let _ = self.save();
        }
        taken
    }

    /// The person has seen the alert about this new admin device.
    pub fn dismiss_alert(&self, device_id: &str) -> Result<()> {
        self.file
            .lock()
            .unwrap()
            .alerts
            .retain(|a| a.device_id != device_id);
        self.save()?;
        self.emit(AppEvent::SecurityChanged);
        Ok(())
    }

    /// Answer the code comparison of the pairing in progress.
    pub fn confirm_pairing(&self, accept: bool) -> Result<()> {
        let mut slot = self.pending_pairing.lock().unwrap();
        match std::mem::take(&mut *slot) {
            PairingSlot::Waiting(tx) => {
                let _ = tx.send(accept);
            }
            // Answered before the question was registered: remember it for the decider.
            _ => *slot = PairingSlot::Early(accept),
        }
        Ok(())
    }

    /// Offer one new device the `role`; show the returned ticket (text or QR).
    pub async fn pair_start(self: &Arc<Self>, role: &str) -> Result<PairStart> {
        let ln = self.ln().await?;
        let role = parse_role(role)?;
        let ticket = ln
            .start_pairing(role, Duration::from_secs(300), self.pairing_decider())
            .await?;
        let text = ticket.to_string();
        let qr = qrcode::QrCode::new(text.as_bytes()).context("encoding the QR code")?;
        let svg = qr
            .render::<qrcode::render::svg::Color>()
            .min_dimensions(200, 200)
            .quiet_zone(true)
            .build();
        Ok(PairStart {
            ticket: text,
            qr_svg: svg,
            expires_at: ticket.expires.0 as f64,
        })
    }

    // ---------- state ----------

    pub async fn app_state(&self) -> Result<AppState> {
        let device_id = hex::encode(self.node.id().as_bytes());
        let (name, role, alerts) = {
            let f = self.file.lock().unwrap();
            (
                f.device_name.clone(),
                // Files from before 0.1.0 say "phone".
                f.role
                    .as_deref()
                    .map(|r| parse_role(r).map(role_name).unwrap_or("keeper"))
                    .unwrap_or("keeper")
                    .to_string(),
                f.alerts.clone(),
            )
        };
        let update = {
            let dismissed = self.file.lock().unwrap().dismissed_update.clone();
            self.update
                .lock()
                .unwrap()
                .clone()
                .filter(|u| Some(&u.version) != dismissed.as_ref())
        };
        // A question nobody waits for any more (the node gave up) is not shown.
        let recovery_request = self
            .pending_recovery
            .lock()
            .unwrap()
            .as_ref()
            .filter(|p| !p.answer.is_closed())
            .map(|p| p.question.clone());
        let guard = self.library.read().await;
        Ok(match guard.as_ref() {
            None => AppState {
                has_library: false,
                device_name: name,
                device_id,
                role,
                membership: "unknown".into(),
                recovery_kit_saved: false,
                library_id: None,
                alerts: vec![],
                recovery_request: None,
                update,
            },
            Some(ln) => AppState {
                has_library: true,
                device_name: name,
                device_id,
                role,
                membership: format!("{:?}", ln.membership().await?).to_lowercase(),
                recovery_kit_saved: ln.recovery_kit_saved().await?,
                library_id: Some(hex::encode(ln.library_id().0)),
                alerts,
                recovery_request,
                update,
            },
        })
    }

    pub async fn mark_recovery_kit_saved(&self) -> Result<()> {
        self.ln().await?.set_recovery_kit_saved(true).await
    }

    // ---------- timeline ----------

    /// The grid's view of `recs`. What every photo shares (the copies target, the keepers, which
    /// blobs are kept, the poster frames) is looked up once, and the blob store is asked about
    /// many photos at a time: one by one, a phone spent a second per 2,000 photos waiting on it.
    async fn summaries(
        &self,
        ln: &LibraryNode<Library>,
        recs: &[PhotoRecord],
    ) -> Result<Vec<PhotoSummary>> {
        use futures::{StreamExt, TryStreamExt};
        if recs.is_empty() {
            return Ok(vec![]);
        }
        let target = ln.redundancy_target().await? as usize;
        let keepers: HashSet<EndpointId> = ln
            .store()
            .device_list()
            .await?
            .devices
            .iter()
            .filter(|d| d.role == Role::Keeper && !d.revoked)
            .map(|d| d.device)
            .collect();
        let blobs = self.node.blobs().clone();
        let protected = Arc::new(blobs.protected().await?);
        // (thumbnail stored, original kept), many requests in flight. Owned values only: a
        // borrowing closure here would make every command's future not `Send`.
        let wanted: Vec<(Blake3Hash, Blake3Hash)> = recs
            .iter()
            .map(|r| (r.thumb.hash, r.original.hash))
            .collect();
        let stored: Vec<(bool, bool)> = futures::stream::iter(wanted)
            .map(move |(thumb, original)| {
                let (blobs, protected) = (blobs.clone(), protected.clone());
                async move {
                    let has_thumb = blobs.has(&thumb).await?;
                    let kept = protected.contains(&original) && blobs.has(&original).await?;
                    anyhow::Ok((has_thumb, kept))
                }
            })
            .buffered(32)
            .try_collect()
            .await?;
        let mut out = Vec::with_capacity(recs.len());
        for (rec, (has_thumb, has_original)) in recs.iter().zip(stored) {
            let holders = ln.store().holders(&rec.original.hash).await?;
            let originals = || holders.iter().filter(|h| h.kind == BlobKind::Original);
            let copies = originals().count();
            let is_video = rec.original.mime.starts_with("video/");
            let placeholder = rec.thumb.hash == rec.original.hash;
            out.push(PhotoSummary {
                id: rec.id.to_string(),
                taken_at: rec.taken_at.0 as f64,
                favorite: rec.favorite.value,
                is_video,
                copies: copies as u32,
                // Safe once enough devices hold it, or once any keeper does.
                backed_up: copies >= target || originals().any(|h| keepers.contains(&h.device)),
                has_thumb,
                has_poster: is_video && !placeholder && has_thumb,
                needs_poster: is_video && placeholder,
                has_original,
                width: rec.exif.width,
                height: rec.exif.height,
            });
        }
        Ok(out)
    }

    /// A page of the timeline, newest first.
    pub async fn library_page(&self, cursor: Option<PageCursor>, limit: u32) -> Result<PhotoPage> {
        let ln = self.ln().await?;
        let after = cursor
            .map(|c| -> Result<Cursor> {
                Ok(Cursor {
                    taken_at: Timestamp(c.taken_at as i64),
                    id: parse_hash(&c.id)?,
                })
            })
            .transpose()?;
        let limit = limit.clamp(1, 2000) as usize;
        let recs = ln
            .store()
            .photos_by_time(TimeRange::ALL, Page { after, limit })
            .await?;
        let items = self.summaries(&ln, &recs).await?;
        let next = (recs.len() == limit)
            .then(|| recs.last())
            .flatten()
            .map(|r| PageCursor {
                taken_at: r.taken_at.0 as f64,
                id: r.id.to_string(),
            });
        Ok(PhotoPage { items, next })
    }

    pub async fn counts(&self) -> Result<LibraryCounts> {
        let ln = self.ln().await?;
        let photos = ln
            .store()
            .photos_by_time(TimeRange::ALL, Page::all())
            .await?;
        let deleted = ln.store().deleted_photos().await?.len();
        Ok(LibraryCounts {
            photos: photos.len() as u32,
            favorites: photos.iter().filter(|p| p.favorite.value).count() as u32,
            deleted: deleted as u32,
            at_risk: ln.at_risk(Page::all()).await?.len() as u32,
            redundancy_target: ln.redundancy_target().await? as u32,
        })
    }

    pub async fn photo_detail(&self, id: &str) -> Result<PhotoDetail> {
        let ln = self.ln().await?;
        let id = parse_hash(id)?;
        let rec = ln
            .store()
            .photo(&id)
            .await?
            .ok_or_else(|| anyhow!("no such photo"))?;
        let holders = ln.store().holders(&rec.original.hash).await?;
        let list = ln.store().device_list().await?;
        let presence = ln.devices();
        let me = ln.device();
        let mut stored_on: Vec<StoredOn> = vec![];
        for blob in [&rec.original, &rec.preview, &rec.thumb] {
            for h in ln.store().holders(&blob.hash).await? {
                let kind = kind_name(h.kind).to_string();
                match stored_on
                    .iter_mut()
                    .find(|s| s.device_id == hex::encode(h.device.0))
                {
                    Some(s) => {
                        if !s.kinds.contains(&kind) {
                            s.kinds.push(kind);
                        }
                    }
                    None => {
                        let cert = list.devices.iter().find(|d| d.device == h.device);
                        stored_on.push(StoredOn {
                            device_id: hex::encode(h.device.0),
                            name: self.shown_name(
                                &h.device,
                                cert.map(|c| c.name.as_str()),
                                &presence,
                            ),
                            role: cert
                                .map(|c| role_name(c.role).to_string())
                                .unwrap_or_default(),
                            online: h.device == me
                                || presence.iter().any(|p| p.id == h.device && p.online),
                            is_this_device: h.device == me,
                            kinds: vec![kind],
                        });
                    }
                }
            }
        }
        let albums = ln
            .store()
            .albums()
            .await?
            .into_iter()
            .map(|a| a.id)
            .collect::<Vec<_>>();
        let mut in_albums = vec![];
        for a in albums {
            let items = ln.store().album_items(&a, Page::all()).await?;
            if items.contains(&id) {
                in_albums.push(hex::encode(a.0));
            }
        }
        let copies = holders
            .iter()
            .filter(|h| h.kind == BlobKind::Original)
            .count() as u32;
        let camera = match (&rec.exif.camera_make, &rec.exif.camera_model) {
            (Some(a), Some(b)) if b.starts_with(a.as_str()) => Some(b.clone()),
            (Some(a), Some(b)) => Some(format!("{a} {b}")),
            (None, Some(b)) => Some(b.clone()),
            (Some(a), None) => Some(a.clone()),
            _ => None,
        };
        Ok(PhotoDetail {
            id: rec.id.to_string(),
            taken_at: rec.taken_at.0 as f64,
            favorite: rec.favorite.value,
            deleted: rec.deleted_at.value.is_some(),
            is_video: rec.original.mime.starts_with("video/"),
            mime: rec.original.mime.clone(),
            size: rec.original.size as f64,
            width: rec.exif.width,
            height: rec.exif.height,
            camera,
            lens: rec.exif.lens.clone(),
            gps: rec
                .exif
                .gps
                .map(|g| (g.lat_e7 as f64 / 1e7, g.lon_e7 as f64 / 1e7)),
            imported_by: hex::encode(rec.imported_by.0),
            pinned: ln.is_pinned(&rec.id),
            copies,
            redundancy_target: ln.redundancy_target().await? as u32,
            has_thumb: self.node.blobs().has(&rec.thumb.hash).await?,
            has_preview: self.node.blobs().has(&rec.preview.hash).await?,
            has_original: self.node.blobs().kept(&rec.original.hash).await?,
            stored_on,
            albums: in_albums,
        })
    }

    // ---------- edits ----------

    pub async fn set_favorite(&self, ids: &[String], favorite: bool) -> Result<()> {
        let ln = self.ln().await?;
        for id in ids {
            ln.store().set_favorite(&parse_hash(id)?, favorite).await?;
        }
        Ok(())
    }

    pub async fn delete(&self, ids: &[String]) -> Result<()> {
        let ln = self.ln().await?;
        let ids: Vec<_> = ids.iter().map(|i| parse_hash(i)).collect::<Result<_>>()?;
        ln.delete_photos(&ids).await
    }

    pub async fn restore(&self, ids: &[String]) -> Result<()> {
        let ln = self.ln().await?;
        let ids: Vec<_> = ids.iter().map(|i| parse_hash(i)).collect::<Result<_>>()?;
        ln.restore_photos(&ids).await
    }

    /// "Delete now" in Recently Deleted: gone from every device at once instead of after 30 days.
    /// Returns how many were deleted (photos no longer in Recently Deleted are skipped).
    pub async fn delete_permanently(&self, ids: &[String]) -> Result<u32> {
        let ln = self.ln().await?;
        let ids: Vec<_> = ids.iter().map(|i| parse_hash(i)).collect::<Result<_>>()?;
        Ok(ln.purge_deleted(&ids).await?.len() as u32)
    }

    /// Recently Deleted, most recently deleted first.
    pub async fn recently_deleted(&self) -> Result<Vec<PhotoSummary>> {
        let ln = self.ln().await?;
        let recs = ln.recently_deleted().await?;
        self.summaries(&ln, &recs).await
    }

    pub async fn import_paths(&self, paths: &[String]) -> Result<ImportSummary> {
        let ln = self.ln().await?;
        let paths: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
        let report = ln.import_paths(&paths, ImportOptions::default()).await?;
        Ok(ImportSummary {
            imported: report.imported.len() as u32,
            duplicates: report.duplicates.len() as u32,
            skipped: report.skipped.len() as u32,
            failed: report
                .failed
                .iter()
                .map(|(p, why)| format!("{}: {why}", p.display()))
                .collect(),
        })
    }

    pub async fn pin_original(&self, id: &str, pin: bool) -> Result<()> {
        let ln = self.ln().await?;
        let id = parse_hash(id)?;
        if pin {
            ln.pin_original(&id).await
        } else {
            ln.unpin_original(&id).await
        }
    }

    /// Get a blob of a photo now (the viewer asks for the original on demand).
    pub async fn fetch_now(&self, id: &str, kind: &str) -> Result<()> {
        let ln = self.ln().await?;
        ln.fetch_now(&parse_hash(id)?, kind_of(kind)?).await?;
        self.emit(AppEvent::Fetched {
            photo: Some(id.to_string()),
            kind: kind.to_string(),
        });
        Ok(())
    }

    // ---------- albums ----------

    pub async fn albums(&self) -> Result<Vec<AlbumInfo>> {
        let ln = self.ln().await?;
        let links = self.shared_links().await;
        Ok(ln
            .store()
            .albums()
            .await?
            .into_iter()
            .map(|a| {
                let id = hex::encode(a.id.0);
                AlbumInfo {
                    shared_as: links.iter().find(|(p, _)| *p == id).map(|(_, s)| s.clone()),
                    id,
                    name: a.name,
                    cover: a.cover.map(|c| c.to_string()),
                    item_count: a.item_count as u32,
                }
            })
            .collect())
    }

    pub async fn create_album(&self, name: &str) -> Result<AlbumInfo> {
        let ln = self.ln().await?;
        let name = name.trim();
        ensure!(!name.is_empty(), "an album needs a name");
        let mut id = [0u8; 16];
        getrandom::fill(&mut id).expect("operating system randomness");
        let id = AlbumId(id);
        let hlc = shoebox_core::Hlc {
            physical_ms: now_ms(),
            counter: 0,
            device: ln.device(),
        };
        ln.store()
            .upsert_album(AlbumMeta {
                id,
                name: Lww::new(name.to_string(), hlc),
                cover: Lww::new(None, hlc),
                items: shoebox_core::merge::OrSet::new(),
            })
            .await?;
        Ok(AlbumInfo {
            id: hex::encode(id.0),
            name: name.to_string(),
            cover: None,
            item_count: 0,
            shared_as: None,
        })
    }

    pub async fn album_add(&self, album: &str, ids: &[String]) -> Result<()> {
        let ln = self.ln().await?;
        let ids: Vec<_> = ids.iter().map(|i| parse_hash(i)).collect::<Result<_>>()?;
        ln.store().album_add(&parse_album(album)?, &ids).await?;
        // A shared album: share what was added (or later, from another device, if this one is
        // not in the shared album).
        if let Err(e) = self.sync_linked_albums().await {
            tracing::debug!("sharing new photos of shared albums: {e:#}");
        }
        Ok(())
    }

    pub async fn album_remove(&self, album: &str, ids: &[String]) -> Result<()> {
        let ln = self.ln().await?;
        let ids: Vec<_> = ids.iter().map(|i| parse_hash(i)).collect::<Result<_>>()?;
        ln.store().album_remove(&parse_album(album)?, &ids).await?;
        Ok(())
    }

    /// Give an album a new name (the latest rename wins on every device).
    pub async fn rename_album(&self, album: &str, name: &str) -> Result<()> {
        let ln = self.ln().await?;
        let name = name.trim();
        ensure!(!name.is_empty(), "an album needs a name");
        let id = parse_album(album)?;
        ensure!(
            ln.store().albums().await?.iter().any(|a| a.id == id),
            "no such album"
        );
        let hlc = shoebox_core::Hlc {
            physical_ms: now_ms(),
            counter: 0,
            device: ln.device(),
        };
        // An update that only carries the new name: the cover and items lose to what exists.
        ln.store()
            .upsert_album(AlbumMeta {
                id,
                name: Lww::new(name.to_string(), hlc),
                cover: Lww::new(None, shoebox_core::Hlc::GENESIS),
                items: shoebox_core::merge::OrSet::new(),
            })
            .await?;
        Ok(())
    }

    pub async fn album_page(
        &self,
        album: &str,
        cursor: Option<PageCursor>,
        limit: u32,
    ) -> Result<PhotoPage> {
        let ln = self.ln().await?;
        let after = cursor
            .map(|c| parse_hash(&c.id).map(Cursor::after_id))
            .transpose()?;
        let limit = limit.clamp(1, 2000) as usize;
        let ids = ln
            .store()
            .album_items(&parse_album(album)?, Page { after, limit })
            .await?;
        let mut recs = vec![];
        for id in &ids {
            if let Some(rec) = ln.store().photo(id).await? {
                if rec.deleted_at.value.is_none() {
                    recs.push(rec);
                }
            }
        }
        let items = self.summaries(&ln, &recs).await?;
        let next = (ids.len() == limit)
            .then(|| ids.last())
            .flatten()
            .map(|id| PageCursor {
                taken_at: 0.0,
                id: id.to_string(),
            });
        Ok(PhotoPage { items, next })
    }

    // ---------- devices ----------

    /// What each device holds of the library (devices holding nothing are left out).
    pub async fn device_storage(&self) -> Result<Vec<crate::types::DeviceStorageInfo>> {
        let ln = self.ln().await?;
        Ok(ln
            .storage_by_device()
            .await?
            .into_iter()
            .map(|(id, s)| crate::types::DeviceStorageInfo {
                id: hex::encode(id.0),
                originals: s.originals,
                original_bytes: s.original_bytes as f64,
                other_bytes: s.other_bytes as f64,
            })
            .collect())
    }

    /// What is moving right now and what this device still has to fetch (Devices screen).
    pub async fn transfers(&self) -> Result<crate::types::Transfers> {
        use crate::types::{TransferItem, Transfers};
        let ln = self.ln().await?;
        let snap = self.node.transfers();
        let mut cache = self.transfer_cache.lock().await;
        let stale =
            |at: Option<std::time::Instant>| at.is_none_or(|t| t.elapsed() > TRANSFER_CACHE_TTL);
        let unknown = snap
            .downloads
            .iter()
            .chain(&snap.uploads)
            .any(|t| !cache.blobs.contains_key(&t.hash));
        if stale(cache.blobs_at)
            || (unknown && cache.blobs_at.is_none_or(|t| t.elapsed().as_secs() >= 1))
        {
            let mut blobs = HashMap::new();
            for rec in ln
                .store()
                .photos_by_time(TimeRange::ALL, Page::all())
                .await?
            {
                blobs.insert(rec.thumb.hash, (rec.id, BlobKind::Thumb));
                blobs.insert(rec.preview.hash, (rec.id, BlobKind::Preview));
                // Last: a video's thumbnail and preview are its original.
                blobs.insert(rec.original.hash, (rec.id, BlobKind::Original));
            }
            cache.blobs = blobs;
            cache.blobs_at = Some(std::time::Instant::now());
        }
        if stale(cache.waiting_at) {
            cache.waiting = ln
                .missing_blobs()
                .await?
                .into_iter()
                .map(|q| (q.item.blob.hash, q.item.kind, q.item.blob.size))
                .collect();
            cache.waiting_at = Some(std::time::Instant::now());
        }
        let item = |t: &shoebox_node::Transfer| {
            let known = cache.blobs.get(&t.hash);
            TransferItem {
                photo: known.map(|(p, _)| p.to_string()),
                kind: known
                    .map_or("other", |(_, k)| match k {
                        BlobKind::Thumb => "thumb",
                        BlobKind::Preview => "preview",
                        BlobKind::Original => "original",
                    })
                    .into(),
                peer: t.peer.map(|p| hex::encode(p.0)),
                size: t.size.map(|s| s as f64),
                done: t.done as f64,
            }
        };
        let downloading: Vec<TransferItem> = snap.downloads.iter().map(item).collect();
        let sending: Vec<TransferItem> = snap.uploads.iter().map(item).collect();
        let in_flight: HashSet<Blake3Hash> = snap.downloads.iter().map(|t| t.hash).collect();
        let (mut thumbs, mut previews, mut originals, mut bytes) = (0u32, 0u32, 0u32, 0u64);
        for (hash, kind, size) in &cache.waiting {
            if in_flight.contains(hash) || self.node.blobs().has(hash).await? {
                continue;
            }
            match kind {
                BlobKind::Thumb => thumbs += 1,
                BlobKind::Preview => previews += 1,
                BlobKind::Original => originals += 1,
            }
            bytes += size;
        }
        drop(cache);
        let paused = if ln.fetcher_paused() {
            let policy = self.backup.lock().await.policy.clone();
            let source = self.photo_source.lock().unwrap().clone();
            let conditions = match source {
                Some(src) => src.conditions().await.ok(),
                None => None,
            };
            Some(
                conditions
                    .and_then(|c| crate::backup::pause_reason(&policy, &c))
                    .unwrap_or("paused")
                    .to_string(),
            )
        } else {
            None
        };
        Ok(Transfers {
            downloading,
            sending,
            download_rate: snap.download_rate as f64,
            upload_rate: snap.upload_rate as f64,
            waiting_thumbs: thumbs,
            waiting_previews: previews,
            waiting_originals: originals,
            waiting_bytes: bytes as f64,
            paused,
        })
    }

    /// How a device is named everywhere in the app. The name a device reports about itself is the
    /// freshest; the certificate's name (set when it joined, updated by an admin) is the fallback.
    fn shown_name(
        &self,
        device: &EndpointId,
        cert_name: Option<&str>,
        presence: &[shoebox_node::DeviceStatus],
    ) -> String {
        if device.0 == *self.node.id().as_bytes() {
            return self.device_name();
        }
        presence
            .iter()
            .find(|p| p.id == *device)
            .and_then(|p| p.name.clone())
            .or_else(|| cert_name.map(str::to_string))
            .unwrap_or_else(|| "unknown device".into())
    }

    pub async fn devices(&self) -> Result<Vec<DeviceInfo>> {
        let ln = self.ln().await?;
        let list = ln.store().device_list().await?;
        let presence = ln.devices();
        let me = ln.device();
        let mut out = vec![];
        for d in &list.devices {
            let (path, latency) = if d.device == me {
                ("none", None)
            } else {
                match shoebox_node::to_iroh_id(&d.device) {
                    Ok(id) => match self.node.peer_info(id).await {
                        Some(info) => (
                            match info.path {
                                PathKind::Lan => "lan",
                                PathKind::Direct => "direct",
                                PathKind::Relay { .. } => "relay",
                                PathKind::Unknown => "none",
                            },
                            info.latency.map(|l| l.as_millis() as u32),
                        ),
                        None => ("none", None),
                    },
                    Err(_) => ("none", None),
                }
            };
            out.push(DeviceInfo {
                id: hex::encode(d.device.0),
                name: self.shown_name(&d.device, Some(&d.name), &presence),
                role: role_name(d.role).into(),
                online: d.device == me || presence.iter().any(|p| p.id == d.device && p.online),
                is_this_device: d.device == me,
                admin: list.admins.iter().any(|a| a.admin == d.device),
                revoked: d.revoked,
                path: path.into(),
                latency_ms: latency,
            });
        }
        out.sort_by(|a, b| (b.is_this_device, &a.name).cmp(&(a.is_this_device, &b.name)));
        Ok(out)
    }

    /// Make a device an admin with the recovery phrase (R1-04). No alert here: it was done here.
    pub async fn make_admin(&self, id: &str, phrase: &str) -> Result<()> {
        let ln = self.ln().await?;
        let device = parse_device(id)?;
        self.approved_recoveries.lock().unwrap().push(device);
        ln.make_admin_with_phrase(phrase.trim(), &device).await?;
        self.emit(AppEvent::DevicesChanged);
        Ok(())
    }

    /// Give a device another role. Without a recovery phrase this device must be an admin.
    pub async fn set_device_role(&self, id: &str, role: &str, phrase: Option<&str>) -> Result<()> {
        let ln = self.ln().await?;
        let (device, role) = (parse_device(id)?, parse_role(role)?);
        match phrase.map(str::trim).filter(|p| !p.is_empty()) {
            Some(phrase) => {
                ln.set_device_role_with_phrase(phrase, &device, role)
                    .await?
            }
            None => ln.set_device_role(&device, role).await?,
        }
        self.emit(AppEvent::DevicesChanged);
        Ok(())
    }

    /// Revoke a device and move the library to a new epoch. Without a recovery phrase this
    /// device must be an admin and the other device must not be one; with the phrase any device
    /// can remove any other (A-085).
    pub async fn revoke_device(&self, id: &str, phrase: Option<&str>) -> Result<()> {
        let ln = self.ln().await?;
        let device = parse_device(id)?;
        match phrase.map(str::trim).filter(|p| !p.is_empty()) {
            Some(phrase) => {
                ln.revoke_and_rotate_with_phrase(phrase, &device).await?;
            }
            None => {
                ln.revoke_and_rotate(&device).await?;
            }
        }
        Ok(())
    }

    // ---------- health & storage ----------

    pub async fn health(&self) -> Result<Health> {
        let ln = self.ln().await?;
        let photos = ln
            .store()
            .photos_by_time(TimeRange::ALL, Page::all())
            .await?;
        let target = ln.redundancy_target().await? as usize;
        let mut hist = vec![0u32; target.max(2) + 2];
        let (mut here, mut here_bytes) = (0u32, 0f64);
        for rec in &photos {
            let copies = ln
                .store()
                .holders(&rec.original.hash)
                .await?
                .iter()
                .filter(|h| h.kind == BlobKind::Original)
                .count();
            let last = hist.len() - 1;
            hist[copies.min(last)] += 1;
            if self.node.blobs().has(&rec.original.hash).await? {
                here += 1;
                here_bytes += rec.original.size as f64;
            }
        }
        let list = ln.store().device_list().await?;
        let online = self
            .devices()
            .await?
            .iter()
            .filter(|d| d.online && !d.revoked)
            .count();
        let (free_photos, free_bytes) = match ln.free_up_space(true).await {
            Ok(r) => (r.photos.len() as u32, r.bytes as f64),
            Err(_) => (0, 0.0), // keepers never free space
        };
        Ok(Health {
            photos: photos.len() as u32,
            copies_histogram: hist,
            at_risk: ln.at_risk(Page::all()).await?.len() as u32,
            redundancy_target: target as u32,
            originals_here: here,
            originals_here_bytes: here_bytes,
            devices_online: online as u32,
            devices_total: list.devices.iter().filter(|d| !d.revoked).count() as u32,
            can_free_bytes: free_bytes,
            can_free_photos: free_photos,
        })
    }

    pub async fn free_up_space(&self, dry_run: bool) -> Result<FreeUpReport> {
        let ln = self.ln().await?;
        let r = ln.free_up_space(dry_run).await?;
        Ok(FreeUpReport {
            photos: r.photos.len() as u32,
            bytes: r.bytes as f64,
        })
    }

    // ---------- poster frames ----------

    /// A video's poster frame becomes its thumb in the library (`LibraryNode::set_video_poster`),
    /// made once by a device that can play it and then synced like any thumbnail. Earlier builds
    /// kept a poster per device in this folder; [`AppCore::adopt_local_posters`] moves them over.
    pub(crate) fn poster_path(&self, id: &Blake3Hash) -> PathBuf {
        self.dir.join("posters").join(format!("{id}.jpg"))
    }

    /// Hand the posters an earlier build kept in `posters/` to the library, once.
    fn adopt_local_posters(self: &Arc<Self>) {
        let dir = self.dir.join("posters");
        if !dir.is_dir() {
            return;
        }
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                return;
            };
            let mut adopted = 0u32;
            for path in entries.flatten().map(|e| e.path()) {
                let Some(core) = weak.upgrade() else { return };
                let Ok(ln) = core.ln().await else { return };
                let id = path.file_stem().and_then(|s| s.to_str()).map(parse_hash);
                if let (Some(Ok(id)), Ok(jpeg)) = (id, std::fs::read(&path)) {
                    match ln.set_video_poster(&id, jpeg).await {
                        Ok(true) => adopted += 1,
                        Ok(false) => {}
                        // A photo no longer in the library, or not a poster: nothing to keep.
                        Err(e) => tracing::debug!("poster {}: {e:#}", path.display()),
                    }
                }
                let _ = std::fs::remove_file(&path);
            }
            let _ = std::fs::remove_dir(&dir);
            if let Some(core) = weak.upgrade() {
                tracing::info!(adopted, "posters moved into the library");
                if adopted > 0 {
                    core.emit(AppEvent::LibraryChanged);
                }
            }
        });
    }

    pub async fn set_poster(&self, id: &str, base64_jpeg: &str) -> Result<()> {
        use base64::Engine;
        let id = parse_hash(id)?;
        let rec = self
            .record(&id)
            .await?
            .ok_or_else(|| anyhow!("no such photo"))?;
        ensure!(
            rec.original.mime.starts_with("video/"),
            "only videos have poster frames"
        );
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(base64_jpeg.trim())
            .map_err(|_| anyhow!("the poster is not valid base64"))?;
        if self.ln().await?.set_video_poster(&id, bytes).await? {
            self.emit(AppEvent::LibraryChanged);
        }
        Ok(())
    }

    // ---------- leaving ----------

    /// Leave this device's library, to create a new one or join another: the data folder is set
    /// aside next to itself as `<folder>.left-<library>-<seconds>` (nothing is deleted, so it can
    /// be put back) and the app starts at first run once it is restarted, which the shell does.
    /// The device key is kept. The other devices list this one until an admin removes it.
    pub async fn leave_library(&self) -> Result<String> {
        let id = match self.library.read().await.as_ref() {
            Some(ln) => hex::encode(ln.library_id().0),
            None => bail!("this device has no library to leave"),
        };
        let name = self
            .dir
            .file_name()
            .ok_or_else(|| anyhow!("the data folder has no name"))?
            .to_string_lossy()
            .into_owned();
        let aside =
            self.dir
                .with_file_name(format!("{name}.left-{}-{}", &id[..8], now_ms() / 1000));
        std::fs::rename(&self.dir, &aside)
            .with_context(|| format!("setting {} aside", self.dir.display()))?;
        tracing::info!(kept = %aside.display(), "left the library");
        Ok(aside.display().to_string())
    }

    // ---------- export ----------

    /// Copy every original that is on this device into `dest/<year>/<month>/` as
    /// `<date>_<time>_<id>.<ext>` (plus `library.json` with the metadata), checking each file
    /// against its content hash. Nothing in the library changes.
    pub async fn export_library(&self, dest: &str) -> Result<ExportReport> {
        let ln = self.ln().await?;
        let dest = PathBuf::from(dest);
        ensure!(
            !dest.as_os_str().is_empty(),
            "choose a folder to export into"
        );
        std::fs::create_dir_all(&dest).with_context(|| format!("creating {}", dest.display()))?;
        let photos = ln
            .store()
            .photos_by_time(TimeRange::ALL, Page::all())
            .await?;
        let albums = ln.store().albums().await?;
        let mut membership: std::collections::HashMap<Blake3Hash, Vec<String>> = Default::default();
        for a in &albums {
            for id in ln.store().album_items(&a.id, Page::all()).await? {
                membership.entry(id).or_default().push(a.name.clone());
            }
        }
        let (mut written, mut missing, mut bytes) = (0u32, 0u32, 0u64);
        let mut manifest = vec![];
        for rec in &photos {
            let Some(size) = self.node.blobs().size(&rec.original.hash).await? else {
                missing += 1;
                continue;
            };
            let when = crate::export::civil(rec.taken_at.0);
            let ext = crate::export::extension(&rec.original.mime);
            let dir = dest
                .join(format!("{:04}", when.0))
                .join(format!("{:02}", when.1));
            std::fs::create_dir_all(&dir)?;
            let name = format!(
                "{:04}-{:02}-{:02}_{:02}{:02}{:02}_{}.{ext}",
                when.0,
                when.1,
                when.2,
                when.3,
                when.4,
                when.5,
                &rec.id.to_string()[..8]
            );
            let path = dir.join(&name);
            if path.metadata().map(|m| m.len() == size).unwrap_or(false) {
                // An earlier export already wrote this file.
            } else {
                let data = self.node.blobs().get(&rec.original.hash).await?;
                ensure!(
                    Blake3Hash::of_bytes(&data) == rec.original.hash,
                    "{name} does not match its content hash; not exported"
                );
                std::fs::write(&path, &data)?;
            }
            written += 1;
            bytes += size;
            manifest.push(serde_json::json!({
                "id": rec.id.to_string(),
                "file": path.strip_prefix(&dest).unwrap_or(&path).display().to_string(),
                "taken_at": rec.taken_at.0,
                "favorite": rec.favorite.value,
                "deleted": rec.deleted_at.value.is_some(),
                "albums": membership.get(&rec.id).cloned().unwrap_or_default(),
            }));
        }
        std::fs::write(
            dest.join("library.json"),
            serde_json::to_vec_pretty(&serde_json::json!({ "photos": manifest }))?,
        )?;
        Ok(ExportReport {
            written,
            missing,
            bytes: bytes as f64,
            folder: dest.display().to_string(),
        })
    }

    /// Write the originals of `ids` into `dest` (emptied first) as `<date>_<time>_<id>.<ext>`, for
    /// handing to another app. An original that is not on this device is fetched first; each file
    /// is checked against its content hash. Returns the paths, in the order of `ids`.
    pub async fn share_files(&self, ids: &[String], dest: &Path) -> Result<Vec<PathBuf>> {
        let ln = self.ln().await?;
        ensure!(!ids.is_empty(), "choose photos to share");
        let _ = std::fs::remove_dir_all(dest);
        std::fs::create_dir_all(dest).with_context(|| format!("creating {}", dest.display()))?;
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            let hash = parse_hash(id)?;
            let rec = ln
                .store()
                .photo(&hash)
                .await?
                .ok_or_else(|| anyhow!("that photo is no longer in the library"))?;
            if self.node.blobs().size(&rec.original.hash).await?.is_none() {
                ln.fetch_now(&hash, kind_of("original")?).await.context(
                    "a photo is not on this device and no device that has it is reachable",
                )?;
            }
            let data = self.node.blobs().get(&rec.original.hash).await?;
            ensure!(
                Blake3Hash::of_bytes(&data) == rec.original.hash,
                "a photo does not match its content hash; not shared"
            );
            let when = crate::export::civil(rec.taken_at.0);
            let path = dest.join(format!(
                "{:04}-{:02}-{:02}_{:02}{:02}{:02}_{}.{}",
                when.0,
                when.1,
                when.2,
                when.3,
                when.4,
                when.5,
                &rec.id.to_string()[..8],
                crate::export::extension(&rec.original.mime)
            ));
            std::fs::write(&path, &data)?;
            out.push(path);
        }
        Ok(out)
    }

    // ---------- settings ----------

    pub async fn settings(&self) -> Result<Settings> {
        let cfg = self.node.config();
        let (name, relay, role) = {
            let f = self.file.lock().unwrap();
            (
                f.device_name.clone(),
                f.relay.clone().unwrap_or_else(|| relay_text(&cfg.relay)),
                f.role.clone().unwrap_or_else(|| "keeper".into()),
            )
        };
        let target = match self.library.read().await.as_ref() {
            Some(ln) => ln.redundancy_target().await? as u32,
            None => shoebox_node::DEFAULT_REDUNDANCY as u32,
        };
        Ok(Settings {
            device_name: name,
            redundancy_target: target,
            relay,
            watched_folders: cfg
                .watched_folders
                .iter()
                .map(|p| p.display().to_string())
                .collect(),
            role,
            update_checks: self.update_checks_enabled(),
            keep_recent_days: cfg.keep_recent_days,
            keep_favorites: cfg.keep_favorites,
        })
    }

    /// What a Companion keeps on this device: the originals of photos taken in the last
    /// `recent_days` (0: none) and of favourites. Pins are always kept.
    pub async fn set_keep_rules(&self, recent_days: u32, favorites: bool) -> Result<()> {
        ensure!(
            recent_days <= 3650,
            "the recent window is at most ten years"
        );
        self.ln().await?.set_keep_rules(recent_days, favorites)
    }

    pub async fn set_redundancy_target(&self, copies: u32) -> Result<()> {
        ensure!(
            (1..=9).contains(&copies),
            "the redundancy target must be between 1 and 9"
        );
        self.ln().await?.set_redundancy_target(copies as u8).await
    }

    /// Takes effect the next time the app starts.
    pub async fn set_relay(&self, relay: &str) -> Result<()> {
        let relay = relay.trim();
        if !matches!(relay, "default" | "off") {
            ensure!(
                relay.starts_with("https://"),
                "a relay is `default`, `off` or an https:// URL"
            );
        }
        self.file.lock().unwrap().relay = Some(relay.to_string());
        self.save()
    }

    pub async fn rename_device(&self, name: &str) -> Result<()> {
        let name = name.trim();
        ensure!(!name.is_empty(), "the device needs a name");
        self.file.lock().unwrap().device_name = name.to_string();
        self.node.update_config(|c| c.name = name.to_string())?;
        self.save()?;
        if let Ok(ln) = self.ln().await {
            ln.announce_rename(name).await?;
        }
        if let Some(shared) = self.shared.read().await.clone() {
            shared.rename_me(name).await;
        }
        self.emit(AppEvent::DevicesChanged);
        Ok(())
    }

    pub async fn watch_folder(&self, dir: &str) -> Result<()> {
        self.ln().await?.watch_folder(Path::new(dir)).await
    }

    // ---------- blobs ----------

    /// The bytes of one rendition of a photo (or a byte range of it). `None` = not on this
    /// device; for previews and originals the fetch starts in the background so a retry works.
    pub fn set_media_base(&self, base: String) {
        *self.media_base.lock().unwrap() = Some(base);
    }

    pub fn media_base(&self) -> Option<String> {
        self.media_base.lock().unwrap().clone()
    }

    pub async fn blob(
        self: &Arc<Self>,
        id: &str,
        kind: &str,
        range: Option<(u64, Option<u64>)>,
    ) -> Result<Option<crate::blob::BlobBytes>> {
        crate::blob::serve(self, id, kind, range).await
    }

    pub(crate) async fn record(&self, id: &Blake3Hash) -> Result<Option<PhotoRecord>> {
        Ok(self.ln().await?.store().photo(id).await?)
    }

    pub(crate) fn blobs(&self) -> &shoebox_node::BlobStore {
        self.node.blobs()
    }

    /// Ask for a missing blob without waiting for it.
    pub(crate) fn fetch_in_background(self: &Arc<Self>, id: Blake3Hash, kind: BlobKind) {
        let core = self.clone();
        tokio::spawn(async move {
            if let Ok(ln) = core.ln().await {
                let _ = ln.fetch_now(&id, kind).await;
                core.emit(AppEvent::Fetched {
                    photo: Some(id.to_string()),
                    kind: kind_name(kind).into(),
                });
            }
        });
    }
}

/// A directory made by `shoebox-dev` holds the same node layout plus a `dev.json`. When the
/// app is pointed at one that has no `app.json` yet, take over its library (same device key,
/// same data); the dev tool's root seed, if any, is left where it is and not used.
fn adopt_dev_library(dir: &Path) -> Result<Option<AppFile>> {
    #[derive(Deserialize)]
    struct DevFile {
        name: String,
        role: String,
        ticket: String,
    }
    let Ok(bytes) = std::fs::read(dir.join("dev.json")) else {
        return Ok(None);
    };
    let dev: DevFile = serde_json::from_slice(&bytes).context("reading dev.json")?;
    let _: LibraryTicket = dev
        .ticket
        .parse()
        .map_err(|e| anyhow!("corrupt ticket in dev.json: {e}"))?;
    Ok(Some(AppFile {
        device_name: dev.name,
        role: Some(role_name(parse_role(&dev.role)?).to_string()),
        ticket: Some(dev.ticket),
        relay: None,
        alerts: vec![],
        update_checks: None,
        dismissed_update: None,
    }))
}

fn name_or_default(name: &str) -> String {
    if name.is_empty() {
        "This device".into()
    } else {
        name.to_string()
    }
}

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
