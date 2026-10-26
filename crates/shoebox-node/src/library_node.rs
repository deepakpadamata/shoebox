//! `LibraryNode`: a [`Node`] bound to a library store. It adds what needs both the network/blob
//! layer and the replicated library: importing, publishing what this device holds, fetching by
//! role, and so on. Generic over the store backend, so the in-memory and iroh-docs backends are
//! interchangeable.

use crate::node::Node;
use crate::sync::{PeerState, SyncConfig};
use crate::ticket::LibraryTicket;
use anyhow::Result;
use notify::RecommendedWatcher;
use shoebox_core::clock::Clock;
use shoebox_core::hlc::{Hlc, HlcClock};
use shoebox_core::ids::{Blake3Hash, EndpointId};
use shoebox_core::model::BlobKind;
use shoebox_core::store::{HoldingsDelta, LibraryCapability, LibraryStore};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

pub(crate) struct Inner<L: LibraryStore> {
    pub node: Node,
    pub store: Arc<L>,
    pub device: EndpointId,
    pub clock: Arc<dyn Clock>,
    /// Orders this device's own holdings writes (it is their only writer).
    holdings_clock: Mutex<HlcClock>,
    pub watchers: Mutex<HashMap<PathBuf, RecommendedWatcher>>,
    pub peers: Mutex<HashMap<EndpointId, PeerState>>,
    pub nudge: mpsc::UnboundedSender<EndpointId>,
    pub sync_config: SyncConfig,
    pub(crate) cap: Mutex<Option<LibraryCapability>>,
    pub(crate) presence: Mutex<Option<crate::presence::PresenceState>>,
    pub(crate) devices: Mutex<HashMap<EndpointId, crate::presence::DeviceRecord>>,
    pub(crate) fetcher: crate::fetcher::FetcherState,
    /// Held (shared) while a file is ingested and (exclusively) while blobs are reconciled, so a
    /// freshly tagged blob whose record is not written yet is never released.
    pub(crate) ingest: tokio::sync::RwLock<()>,
    pub(crate) membership: Mutex<crate::identity::Membership>,
    pub(crate) confirmations: Mutex<crate::freeup::Confirmations>,
    pub(crate) notice: Mutex<Option<shoebox_core::identity::RotationNotice>>,
    pub(crate) notice_acked: Mutex<std::collections::HashSet<EndpointId>>,
    pub(crate) invites: Arc<Mutex<crate::pairing::Invites>>,
}

pub struct LibraryNode<L: LibraryStore> {
    pub(crate) inner: Arc<Inner<L>>,
}

impl<L: LibraryStore> Clone for LibraryNode<L> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<L: LibraryStore> LibraryNode<L> {
    /// Bind `node` to `store`. The device identity is the node's endpoint id. Folders that were
    /// being watched when the node last ran are watched again (and scanned for files that arrived
    /// while it was closed).
    pub fn new(node: Node, store: Arc<L>, clock: impl Clock) -> Self {
        Self::with_sync_config(node, store, clock, SyncConfig::default())
    }

    pub fn with_sync_config(
        node: Node,
        store: Arc<L>,
        clock: impl Clock,
        sync: SyncConfig,
    ) -> Self {
        let device = EndpointId(*node.id().as_bytes());
        let data_dir = node.config().data_dir;
        let (nudge, nudge_rx) = mpsc::unbounded_channel();
        let ln = Self {
            inner: Arc::new(Inner {
                node,
                store,
                device,
                clock: Arc::new(clock),
                holdings_clock: Mutex::new(HlcClock::new(device)),
                watchers: Mutex::new(HashMap::new()),
                peers: Mutex::new(HashMap::new()),
                nudge,
                sync_config: sync,
                cap: Mutex::new(None),
                presence: Mutex::new(None),
                devices: Mutex::new(HashMap::new()),
                fetcher: Default::default(),
                ingest: tokio::sync::RwLock::new(()),
                membership: Mutex::new(crate::identity::Membership::Unknown),
                invites: Default::default(),
                notice: Mutex::new(crate::rotation::load_notice(&data_dir)),
                notice_acked: Default::default(),
                confirmations: Mutex::new(crate::freeup::Confirmations::load(&data_dir)),
            }),
        };
        ln.start_sync_manager(nudge_rx);
        ln.start_rotation_support();
        ln.restore_watches();
        ln
    }

    /// Remember the capability this library was opened with, so [`LibraryNode::ticket`] can share it.
    pub fn with_capability(self, cap: LibraryCapability) -> Self {
        *self.inner.cap.lock().unwrap() = Some(cap);
        self
    }

    /// A dev-only ticket another device can join with (needs [`LibraryNode::with_capability`]).
    pub fn ticket(&self) -> Result<LibraryTicket> {
        let cap = self.inner.cap.lock().unwrap().clone().ok_or_else(|| {
            anyhow::anyhow!("this library node was not given a capability to share")
        })?;
        Ok(LibraryTicket {
            cap,
            peers: vec![crate::addr::from_endpoint_addr(&self.node().addr())],
        })
    }

    pub(crate) fn capability(&self) -> Result<LibraryCapability> {
        self.inner.cap.lock().unwrap().clone().ok_or_else(|| {
            anyhow::anyhow!("this library node was not given a capability (see with_capability)")
        })
    }

    pub fn library_id(&self) -> shoebox_core::store::LibraryId {
        self.capability()
            .expect("library node has a capability")
            .library
    }

    pub fn node(&self) -> &Node {
        &self.inner.node
    }

    pub fn store(&self) -> &Arc<L> {
        &self.inner.store
    }

    pub fn device(&self) -> EndpointId {
        self.inner.device
    }

    pub(crate) fn now_ms(&self) -> u64 {
        self.inner.clock.now_ms()
    }

    /// An HLC for a cert that replaces one dated `prev`: later than it even when the device
    /// that wrote `prev` has a clock ahead of ours (per device, the latest cert wins).
    pub(crate) fn cert_hlc_after(&self, prev: &Hlc) -> Hlc {
        let now = self.holdings_hlc();
        if now > *prev {
            now
        } else {
            Hlc {
                physical_ms: prev.physical_ms,
                counter: prev.counter + 1,
                device: self.device(),
            }
        }
    }

    /// A fresh HLC for this device's own holdings writes.
    pub(crate) fn holdings_hlc(&self) -> Hlc {
        let now = self.now_ms();
        self.inner.holdings_clock.lock().unwrap().tick(now)
    }

    /// Tell the library that this device now holds `blobs`.
    pub async fn publish_holdings(&self, blobs: Vec<(Blake3Hash, BlobKind)>) -> Result<()> {
        if blobs.is_empty() {
            return Ok(());
        }
        let delta = HoldingsDelta {
            hlc: self.holdings_hlc(),
            added: blobs,
            removed: vec![],
        };
        self.inner
            .store
            .put_holdings(self.inner.device, delta)
            .await?;
        Ok(())
    }
}
