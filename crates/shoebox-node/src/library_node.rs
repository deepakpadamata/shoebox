//! `LibraryNode`: a [`Node`] bound to a library store. It adds what needs both the network/blob
//! layer and the replicated library: importing, publishing what this device holds, fetching by
//! role, and so on. Generic over the store backend, so the in-memory and iroh-docs backends are
//! interchangeable.

use crate::node::Node;
use anyhow::Result;
use notify::RecommendedWatcher;
use shoebox_core::clock::Clock;
use shoebox_core::hlc::{Hlc, HlcClock};
use shoebox_core::ids::{Blake3Hash, EndpointId};
use shoebox_core::model::BlobKind;
use shoebox_core::store::{HoldingsDelta, LibraryStore};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub(crate) struct Inner<L: LibraryStore> {
    pub node: Node,
    pub store: Arc<L>,
    pub device: EndpointId,
    pub clock: Arc<dyn Clock>,
    /// Orders this device's own holdings writes (it is their only writer).
    holdings_clock: Mutex<HlcClock>,
    pub watchers: Mutex<HashMap<PathBuf, RecommendedWatcher>>,
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
        let device = EndpointId(*node.id().as_bytes());
        let ln = Self {
            inner: Arc::new(Inner {
                node,
                store,
                device,
                clock: Arc::new(clock),
                holdings_clock: Mutex::new(HlcClock::new(device)),
                watchers: Mutex::new(HashMap::new()),
            }),
        };
        ln.restore_watches();
        ln
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
