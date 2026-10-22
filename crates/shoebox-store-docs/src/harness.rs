//! Plugs the iroh-docs backend into the shoebox-core conformance suite: every device gets its
//! own real node (own iroh endpoint, blob store and docs engine) in a temp dir.

use crate::{from_endpoint_addr, DocsBackend};
use async_trait::async_trait;
use shoebox_core::clock::ManualClock;
use shoebox_core::conformance::Harness;
use shoebox_core::ids::EndpointId;
use shoebox_core::store::PeerAddr;
use shoebox_node::{Node, NodeConfig};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Shared by every world a harness creates, so `shutdown` can stop all nodes at the end.
#[derive(Default)]
struct Registry {
    nodes: Mutex<Vec<Node>>,
    dirs: Mutex<Vec<tempfile::TempDir>>,
}

pub struct DocsHarness {
    registry: Arc<Registry>,
    dir: Arc<tempfile::TempDir>,
    peers: Arc<Mutex<HashMap<EndpointId, PeerAddr>>>,
    world_nodes: Arc<Mutex<Vec<Node>>>,
}

impl Default for DocsHarness {
    fn default() -> Self {
        Self::with_registry(Arc::new(Registry::default()))
    }
}

impl DocsHarness {
    fn with_registry(registry: Arc<Registry>) -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        Self {
            registry,
            dir: Arc::new(dir),
            peers: Default::default(),
            world_nodes: Default::default(),
        }
    }

    /// Stop every node created through this harness (including `fresh` worlds).
    pub async fn shutdown(&self) {
        let nodes: Vec<Node> = self.registry.nodes.lock().unwrap().drain(..).collect();
        for node in nodes {
            let _ = node.shutdown().await;
        }
        self.registry.dirs.lock().unwrap().clear();
    }
}

#[async_trait]
impl Harness for DocsHarness {
    type Backend = DocsBackend;

    fn fresh(&self) -> Self {
        let world = Self::with_registry(self.registry.clone());
        // Keep the previous world's files until `shutdown` (its nodes may still be winding down).
        if let Ok(old) = Arc::try_unwrap(Arc::clone(&self.dir)) {
            self.registry.dirs.lock().unwrap().push(old);
        }
        world
    }

    async fn backend(&self, device: EndpointId, clock: ManualClock) -> DocsBackend {
        let name = format!("device-{}", hex::encode(&device.0[..2]));
        let cfg = NodeConfig::local_only(self.dir.path().join(&name), name);
        let node = Node::open(cfg).await.expect("node opens");
        // Everyone in the world can reach everyone else (no discovery service needed).
        for other in self.world_nodes.lock().unwrap().iter() {
            node.add_peer_addr(other.addr());
            other.add_peer_addr(node.addr());
        }
        self.world_nodes.lock().unwrap().push(node.clone());
        self.registry.nodes.lock().unwrap().push(node.clone());
        self.peers
            .lock()
            .unwrap()
            .insert(device, from_endpoint_addr(&node.addr()));
        DocsBackend::new(node, device, clock)
    }

    fn peer_addr(&self, device: EndpointId) -> PeerAddr {
        self.peers
            .lock()
            .unwrap()
            .get(&device)
            .cloned()
            .expect("device was created by this harness")
    }
}
