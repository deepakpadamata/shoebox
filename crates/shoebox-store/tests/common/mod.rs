#![allow(dead_code)]
use shoebox_core::clock::Clock;
use shoebox_core::merge::Lww;
use shoebox_core::model::*;
use shoebox_core::{Blake3Hash, EndpointId, Hlc, Timestamp};

pub fn dev(n: u8) -> EndpointId {
    EndpointId([n; 32])
}
pub fn hash(n: u8) -> Blake3Hash {
    Blake3Hash([n; 32])
}
pub fn hlc(ms: u64, counter: u32, d: u8) -> Hlc {
    Hlc {
        physical_ms: ms,
        counter,
        device: dev(d),
    }
}
pub fn blob(n: u8) -> BlobRef {
    BlobRef {
        hash: hash(n),
        size: 100 + n as u64,
        mime: "image/jpeg".into(),
    }
}
pub fn photo(n: u8, taken: i64) -> PhotoRecord {
    PhotoRecord {
        id: hash(n),
        original: blob(n),
        preview: blob(n.wrapping_add(100)),
        thumb: blob(n.wrapping_add(200)),
        taken_at: Timestamp(taken),
        exif: ExifSummary::default(),
        imported_by: dev(1),
        favorite: Lww::new(false, hlc(1, 0, 1)),
        deleted_at: Lww::new(None, hlc(1, 0, 1)),
    }
}

// ---------- multi-node helpers ----------

use shoebox_core::identity::{issue_admin_cert, issue_device_cert, KeyPair};
use shoebox_core::store::{
    LibraryCapability, LibraryStore, PeerAddr, SignedDeviceChange, StoreBackend,
};
use shoebox_node::{LibraryNode, SyncConfig};
use shoebox_store::{HybridBackend, Library};
use shoebox_testkit::Cluster;
use std::sync::Arc;
use std::time::Duration;

pub fn fast() -> SyncConfig {
    SyncConfig {
        tick: Duration::from_millis(100),
        resync_interval: Duration::from_secs(2),
        backoff_min: Duration::from_millis(200),
        backoff_max: Duration::from_millis(800),
        timeout: Duration::from_secs(10),
    }
}

pub fn peer_addr(c: &Cluster, i: usize) -> PeerAddr {
    shoebox_node::from_endpoint_addr(&c.node(i).addr())
}

pub fn core_id(c: &Cluster, i: usize) -> EndpointId {
    EndpointId(*c.node(i).id().as_bytes())
}

pub struct Rig {
    pub root: KeyPair,
    pub cap: LibraryCapability,
    pub backends: Vec<HybridBackend>,
}

/// Node 0 creates a library whose root key the test holds; every node is certified as `roles[i]`
/// (node 0 also becomes an admin), and the certificates are replicated to the others.
pub async fn rig(c: &Cluster, roles: &[Role]) -> (Rig, Vec<Library>) {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
    let root = KeyPair::generate();
    let backends: Vec<HybridBackend> = (0..c.len())
        .map(|i| HybridBackend::for_node(c.node(i).clone(), c.clock().clone()))
        .collect();
    let (l0, cap) = backends[0]
        .create_library(&root.root_public())
        .await
        .unwrap();
    let hlc_at = |ms: u64| Hlc {
        physical_ms: ms,
        counter: 0,
        device: core_id(c, 0),
    };
    let t0 = c.clock().now_ms();
    l0.apply_device_change(SignedDeviceChange::Admin(issue_admin_cert(
        &root,
        cap.library,
        core_id(c, 0),
        hlc_at(t0),
        None,
    )))
    .await
    .unwrap();
    for (i, role) in roles.iter().enumerate() {
        let cert = issue_device_cert(
            &root,
            cap.library,
            core_id(c, i),
            *role,
            &format!("n{i}"),
            hlc_at(t0 + 1),
            false,
        );
        l0.apply_device_change(SignedDeviceChange::Device(cert))
            .await
            .unwrap();
    }
    let mut libs = vec![l0];
    for b in &backends[1..] {
        libs.push(b.open_library(&cap).await.unwrap());
    }
    // Everyone learns the device list from node 0.
    for (i, l) in libs.iter().enumerate().skip(1) {
        l.sync_with(&peer_addr(c, 0)).await.unwrap();
        assert_eq!(
            l.device_list().await.unwrap().devices.len(),
            roles.len(),
            "node {i}"
        );
    }
    (
        Rig {
            root,
            cap,
            backends,
        },
        libs,
    )
}

/// Like [`rig`], wrapped in `LibraryNode`s that sync on their own.
pub async fn library_nodes(c: &Cluster, roles: &[Role]) -> (Rig, Vec<LibraryNode<Library>>) {
    let (rig, libs) = rig(c, roles).await;
    let mut nodes = vec![];
    for (i, l) in libs.into_iter().enumerate() {
        let ln = LibraryNode::with_sync_config(
            c.node(i).clone(),
            Arc::new(l),
            c.clock().clone(),
            fast(),
        )
        .with_capability(rig.cap.clone());
        nodes.push(ln);
    }
    for i in 0..nodes.len() {
        for j in 0..nodes.len() {
            if i != j {
                nodes[i].add_peer(peer_addr(c, j)).unwrap();
            }
        }
    }
    (rig, nodes)
}

pub async fn photos_of(l: &impl LibraryStore) -> Vec<shoebox_core::model::PhotoRecord> {
    l.photos_by_time(
        shoebox_core::store::TimeRange::ALL,
        shoebox_core::store::Page::all(),
    )
    .await
    .unwrap()
}

pub async fn converged(nodes: &[LibraryNode<Library>]) -> bool {
    let first = photos_of(nodes[0].store().as_ref()).await;
    for n in &nodes[1..] {
        if photos_of(n.store().as_ref()).await != first {
            return false;
        }
    }
    true
}
