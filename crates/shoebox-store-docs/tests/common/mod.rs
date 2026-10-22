#![allow(dead_code)]
use shoebox_core::merge::{Lww, OrSet};
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
pub fn album() -> AlbumMeta {
    let mut items = OrSet::new();
    items.add(hash(1), hlc(2, 0, 1));
    AlbumMeta {
        id: AlbumId([9; 16]),
        name: Lww::new("Summer".into(), hlc(1, 0, 1)),
        cover: Lww::new(None, hlc(1, 0, 1)),
        items,
    }
}
pub fn device_cert() -> DeviceCert {
    DeviceCert {
        device: dev(2),
        role: Role::Keeper,
        name: "Pi".into(),
        added_by: dev(1),
        hlc: hlc(5, 0, 1),
        revoked: false,
        signature: vec![1, 2],
    }
}
pub fn admin_cert() -> AdminCert {
    AdminCert {
        admin: dev(1),
        hlc: hlc(1, 0, 1),
        expires_at: None,
        signature: vec![9],
    }
}
pub fn shared_item(n: u8) -> SharedItem {
    SharedItem {
        id: hash(n),
        original: blob(n),
        shared_hash: None,
        preview: blob(n.wrapping_add(100)),
        thumb: blob(n.wrapping_add(200)),
        added_by: dev(1),
        added_at: Timestamp(10),
        taken_at: Timestamp(5),
    }
}

// ---------- multi-node helpers ----------

use shoebox_core::store::{LibraryStore, Page, RootPublicKey, StoreBackend, TimeRange};
use shoebox_node::{LibraryNode, LibraryTicket, SyncConfig};
use shoebox_store_docs::{DocsBackend, DocsLibrary};
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

/// Node 0 creates the library; the others join through its ticket string.
pub async fn library_nodes(c: &Cluster) -> Vec<LibraryNode<DocsLibrary>> {
    // Set RUST_LOG (e.g. `shoebox_store_docs=debug,shoebox_node=debug`) to see what the sync layers do.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
    let b0 = DocsBackend::for_node(c.node(0).clone(), c.clock().clone());
    let (l0, cap) = b0.create_library(&RootPublicKey([7; 32])).await.unwrap();
    let ln0 =
        LibraryNode::with_sync_config(c.node(0).clone(), Arc::new(l0), c.clock().clone(), fast())
            .with_capability(cap);
    let ticket = ln0.ticket().unwrap().to_string();

    let mut all = vec![ln0];
    for i in 1..c.len() {
        let ticket: LibraryTicket = ticket.parse().unwrap();
        let b = DocsBackend::for_node(c.node(i).clone(), c.clock().clone());
        let lib = b.open_library(&ticket.cap).await.unwrap();
        let ln = LibraryNode::with_sync_config(
            c.node(i).clone(),
            Arc::new(lib),
            c.clock().clone(),
            fast(),
        )
        .with_capability(ticket.cap.clone());
        for peer in ticket.peers {
            ln.add_peer(peer).unwrap();
        }
        all.push(ln);
    }
    all
}

pub async fn converged(nodes: &[LibraryNode<DocsLibrary>]) -> bool {
    let stores: Vec<&DocsLibrary> = nodes.iter().map(|n| n.store().as_ref()).collect();
    let first = stores[0]
        .photos_by_time(TimeRange::ALL, Page::all())
        .await
        .unwrap();
    let first_deleted = stores[0].deleted_photos().await.unwrap();
    for s in &stores[1..] {
        if s.photos_by_time(TimeRange::ALL, Page::all()).await.unwrap() != first
            || s.deleted_photos().await.unwrap() != first_deleted
        {
            return false;
        }
    }
    true
}
