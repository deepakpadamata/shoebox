//! The keeper daemon: one process per storage directory.
//!
//! Layout under `--storage`: `keeper.lock` (held while running), `control.sock` (0600),
//! `keeper.json` (the paired library: ticket string = capability + peers, device name),
//! plus the node's own `keys/`, `blobs/`, `store/`.

use crate::control;
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use shoebox_core::clock::SystemClock;
use shoebox_core::model::Role;
use shoebox_core::store::StoreBackend;
use shoebox_node::{
    FetcherConfig, LibraryNode, LibraryTicket, MaintenanceConfig, Node, NodeConfig, PresenceConfig,
    RelaySetting,
};
use shoebox_store_docs::{DocsBackend, DocsLibrary};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{watch, Mutex};
use tracing::{info, warn};

#[derive(Clone, Debug)]
pub struct KeeperOptions {
    pub storage: PathBuf,
    pub name: String,
    pub relay: RelaySetting,
    pub mdns: bool,
}

impl KeeperOptions {
    /// No relay and no LAN discovery: for tests.
    pub fn local_only(storage: impl Into<PathBuf>, name: impl Into<String>) -> Self {
        Self {
            storage: storage.into(),
            name: name.into(),
            relay: RelaySetting::Disabled,
            mdns: false,
        }
    }

    pub fn new(storage: impl Into<PathBuf>, name: impl Into<String>) -> Self {
        Self {
            storage: storage.into(),
            name: name.into(),
            relay: RelaySetting::Default,
            mdns: true,
        }
    }
}

/// `$XDG_DATA_HOME/shoebox`, else `~/.local/share/shoebox` (`~/Library/Application Support/shoebox` on macOS).
pub fn default_storage() -> PathBuf {
    if let Some(x) = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        return PathBuf::from(x).join("shoebox");
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/shoebox")
    } else {
        home.join(".local/share/shoebox")
    }
}

#[derive(Serialize, Deserialize)]
struct KeeperState {
    name: String,
    /// A `LibraryTicket` string: the library capability and the devices to sync with.
    ticket: String,
}

pub(crate) struct Shared {
    pub node: Node,
    pub opts: KeeperOptions,
    pub started: Instant,
    pub library: Mutex<Option<LibraryNode<DocsLibrary>>>,
    pub shutdown: watch::Sender<bool>,
}

pub struct Daemon {
    shared: Arc<Shared>,
    control_path: PathBuf,
    // Held for the daemon's lifetime; the OS releases the lock if the process dies.
    _lock: File,
    server: tokio::task::JoinHandle<()>,
    shutdown_rx: watch::Receiver<bool>,
}

fn state_path(storage: &Path) -> PathBuf {
    storage.join("keeper.json")
}

impl Shared {
    /// Open the library from a pairing grant or from `keeper.json`, start the background work
    /// and remember it.
    pub(crate) async fn attach(&self, ticket: LibraryTicket) -> Result<()> {
        let backend = DocsBackend::for_node(self.node.clone(), SystemClock);
        let library = backend.open_library(&ticket.cap).await?;
        let ln = LibraryNode::new(self.node.clone(), Arc::new(library), SystemClock)
            .with_capability(ticket.cap.clone());
        for peer in &ticket.peers {
            ln.add_peer(peer.clone())?;
        }
        ln.enforce_membership().await?;
        ln.start_fetcher(FetcherConfig::default());
        ln.start_maintenance(MaintenanceConfig::default());
        ln.start_presence(PresenceConfig::default()).await?;
        *self.library.lock().await = Some(ln);
        Ok(())
    }

    /// Persist the pairing so the next start rejoins the library.
    pub(crate) fn save(&self, ticket: &LibraryTicket) -> Result<()> {
        let state = KeeperState {
            name: self.opts.name.clone(),
            ticket: ticket.to_string(),
        };
        let path = state_path(&self.opts.storage);
        std::fs::write(&path, serde_json::to_vec_pretty(&state)?)?;
        Ok(())
    }
}

impl Daemon {
    pub async fn start(opts: KeeperOptions) -> Result<Daemon> {
        std::fs::create_dir_all(&opts.storage)
            .with_context(|| format!("creating {}", opts.storage.display()))?;
        let lock = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(opts.storage.join("keeper.lock"))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => bail!(
                "a keeper is already running on {} (it holds keeper.lock)",
                opts.storage.display()
            ),
            Err(std::fs::TryLockError::Error(e)) => return Err(e.into()),
        }

        let mut config = NodeConfig::local_only(&opts.storage, &opts.name);
        config.role = Role::Keeper;
        config.relay = opts.relay.clone();
        config.mdns = opts.mdns;
        let node = Node::open(config).await?;
        let (shutdown, shutdown_rx) = watch::channel(false);
        let shared = Arc::new(Shared {
            node,
            opts: opts.clone(),
            started: Instant::now(),
            library: Mutex::new(None),
            shutdown,
        });

        let saved = state_path(&opts.storage);
        if saved.exists() {
            let state: KeeperState =
                serde_json::from_slice(&std::fs::read(&saved)?).context("reading keeper.json")?;
            let ticket: LibraryTicket = state
                .ticket
                .parse()
                .map_err(|e| anyhow!("corrupt ticket in keeper.json: {e}"))?;
            shared.attach(ticket).await?;
            info!("rejoined the library");
        } else {
            info!("not paired yet: run `shoebox-keeper pair <ticket>`");
        }

        let control_path = opts.storage.join("control.sock");
        let server = control::serve(shared.clone(), &control_path).await?;
        info!(storage = %opts.storage.display(), "keeper running");
        Ok(Daemon {
            shared,
            control_path,
            _lock: lock,
            server,
            shutdown_rx,
        })
    }

    pub fn control_path(&self) -> &Path {
        &self.control_path
    }

    /// Resolves when a shutdown was requested (control socket or [`Daemon::shutdown`]).
    pub async fn wait_for_shutdown(&self) {
        let mut rx = self.shutdown_rx.clone();
        while !*rx.borrow() {
            if rx.changed().await.is_err() {
                break;
            }
        }
    }

    pub async fn shutdown(self) -> Result<()> {
        let _ = self.shared.shutdown.send(true);
        self.server.abort();
        let _ = self.server.await;
        let _ = std::fs::remove_file(&self.control_path);
        if let Some(ln) = self.shared.library.lock().await.take() {
            drop(ln);
        }
        if let Err(e) = self.shared.node.shutdown().await {
            warn!("node shutdown: {e:#}");
        }
        Ok(())
    }
}
