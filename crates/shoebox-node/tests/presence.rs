//! M1-10: gossip presence — who is online, and "new items" nudges that trigger a sync.

use shoebox_core::clock::ManualClock;
use shoebox_core::ids::EndpointId;
use shoebox_core::model::Role;
use shoebox_core::store::*;
use shoebox_node::{
    from_endpoint_addr, LibraryNode, Node, NodeConfig, NodeEvent, PresenceConfig, SyncConfig,
};
use shoebox_store_mem::{MemBackend, MemLibrary, MemNetwork};
use std::sync::Arc;
use std::time::Duration;

fn fast_presence() -> PresenceConfig {
    PresenceConfig {
        heartbeat: Duration::from_millis(200),
        offline_after: Duration::from_millis(900),
    }
}

/// Sync only when nudged: periodic resync is effectively off, so any prompt sync comes from gossip.
fn nudge_only() -> SyncConfig {
    SyncConfig {
        tick: Duration::from_millis(100),
        resync_interval: Duration::from_secs(3600),
        backoff_min: Duration::from_millis(100),
        backoff_max: Duration::from_millis(500),
        timeout: Duration::from_secs(10),
    }
}

struct Peer {
    ln: LibraryNode<MemLibrary>,
}

async fn spawn(
    dir: &tempfile::TempDir,
    name: &str,
    role: Role,
    net: &MemNetwork,
    cap: Option<&LibraryCapability>,
) -> (Peer, LibraryCapability) {
    let mut cfg = NodeConfig::local_only(dir.path().join(name), name);
    cfg.role = role;
    let node = Node::open(cfg).await.unwrap();
    let device = EndpointId(*node.id().as_bytes());
    let backend = MemBackend::new(net.clone(), device, ManualClock::new(1_000));
    let (lib, cap) = match cap {
        None => backend
            .create_library(&RootPublicKey([3; 32]))
            .await
            .unwrap(),
        Some(c) => (backend.open_library(c).await.unwrap(), c.clone()),
    };
    let ln =
        LibraryNode::with_sync_config(node, Arc::new(lib), ManualClock::new(1_000), nudge_only())
            .with_capability(cap.clone());
    (Peer { ln }, cap)
}

async fn wait_for<F, Fut>(timeout: Duration, mut f: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if f().await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

fn connect(a: &Peer, b: &Peer) {
    a.ln.node().add_peer_addr(b.ln.node().addr());
    b.ln.node().add_peer_addr(a.ln.node().addr());
    a.ln.add_peer(from_endpoint_addr(&b.ln.node().addr()))
        .unwrap();
    b.ln.add_peer(from_endpoint_addr(&a.ln.node().addr()))
        .unwrap();
}

#[tokio::test]
async fn hello_fills_the_device_table_and_shutdown_shows_offline() {
    let dir = tempfile::tempdir().unwrap();
    let net = MemNetwork::new();
    let (a, cap) = spawn(&dir, "laptop", Role::Viewer, &net, None).await;
    let (b, _) = spawn(&dir, "pi", Role::Keeper, &net, Some(&cap)).await;
    connect(&a, &b);
    a.ln.start_presence(fast_presence()).await.unwrap();
    b.ln.start_presence(fast_presence()).await.unwrap();
    let b_id = EndpointId(*b.ln.node().id().as_bytes());

    assert!(
        wait_for(Duration::from_secs(10), || async {
            a.ln.devices().iter().any(|d| d.id == b_id && d.online)
        })
        .await,
        "b shows up online"
    );
    let seen = a.ln.devices().into_iter().find(|d| d.id == b_id).unwrap();
    assert_eq!(seen.name.as_deref(), Some("pi"));
    assert_eq!(seen.role, Some(Role::Keeper));
    assert_eq!(seen.app_version.as_deref(), Some(env!("CARGO_PKG_VERSION")));
    assert!(
        a.ln.devices()
            .iter()
            .all(|d| d.id != EndpointId(*a.ln.node().id().as_bytes())),
        "not itself"
    );

    // b disappears: a notices well within the 60 s budget (here: ~1 s with the fast config).
    let mut events = a.ln.node().events();
    b.ln.node().shutdown().await.unwrap();
    assert!(
        wait_for(Duration::from_secs(10), || async {
            a.ln.devices().iter().any(|d| d.id == b_id && !d.online)
        })
        .await,
        "b shows offline"
    );
    let mut saw_offline_event = false;
    while let Ok(Ok(ev)) = tokio::time::timeout(Duration::from_millis(300), events.recv()).await {
        saw_offline_event |= ev
            == NodeEvent::PresenceChanged {
                device: b_id,
                online: false,
            };
    }
    assert!(saw_offline_event, "PresenceChanged(offline) was emitted");
    a.ln.node().shutdown().await.unwrap();
}

#[tokio::test]
async fn other_libraries_do_not_appear_in_the_table() {
    let dir = tempfile::tempdir().unwrap();
    let (a, _) = spawn(&dir, "a", Role::Keeper, &MemNetwork::new(), None).await;
    let (stranger, _) = spawn(&dir, "stranger", Role::Keeper, &MemNetwork::new(), None).await;
    connect(&a, &stranger); // they can reach each other, but they are not in the same library
    a.ln.start_presence(fast_presence()).await.unwrap();
    stranger.ln.start_presence(fast_presence()).await.unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(a.ln.devices().is_empty());
    assert!(stranger.ln.devices().is_empty());
    a.ln.node().shutdown().await.unwrap();
    stranger.ln.node().shutdown().await.unwrap();
}

#[tokio::test]
async fn new_items_nudge_triggers_a_sync_within_two_seconds() {
    let dir = tempfile::tempdir().unwrap();
    let net = MemNetwork::new();
    let (a, cap) = spawn(&dir, "a", Role::Keeper, &net, None).await;
    let (b, _) = spawn(&dir, "b", Role::Keeper, &net, Some(&cap)).await;
    connect(&a, &b);
    a.ln.start_presence(fast_presence()).await.unwrap();
    b.ln.start_presence(fast_presence()).await.unwrap();
    let b_id = EndpointId(*b.ln.node().id().as_bytes());
    // Both know each other and finished their first sync, so only a nudge can cause the next one.
    assert!(
        wait_for(Duration::from_secs(10), || async {
            a.ln.devices().iter().any(|d| d.id == b_id && d.online)
        })
        .await
    );
    tokio::time::sleep(Duration::from_millis(1500)).await;

    let mut p = photo();
    p.imported_by = EndpointId(*a.ln.node().id().as_bytes());
    a.ln.store().upsert_photo(p.clone()).await.unwrap();
    assert!(
        b.ln.store().photo(&p.id).await.unwrap().is_none(),
        "no sync has happened yet"
    );

    let start = std::time::Instant::now();
    a.ln.announce_new_items(1).await.unwrap();
    assert!(
        wait_for(Duration::from_secs(2), || async {
            b.ln.store().photo(&p.id).await.unwrap().is_some()
        })
        .await,
        "b synced after the nudge (took {:?})",
        start.elapsed()
    );
    a.ln.node().shutdown().await.unwrap();
    b.ln.node().shutdown().await.unwrap();
}

fn photo() -> shoebox_core::model::PhotoRecord {
    use shoebox_core::merge::Lww;
    use shoebox_core::model::*;
    use shoebox_core::{Blake3Hash, Hlc, Timestamp};
    let blob = |b: u8| BlobRef {
        hash: Blake3Hash([b; 32]),
        size: 1,
        mime: "image/jpeg".into(),
    };
    let h = Hlc::GENESIS;
    PhotoRecord {
        id: Blake3Hash([1; 32]),
        original: blob(1),
        preview: blob(2),
        thumb: blob(3),
        taken_at: Timestamp(1),
        exif: ExifSummary::default(),
        imported_by: EndpointId([0; 32]),
        favorite: Lww::new(false, h),
        deleted_at: Lww::new(None, h),
    }
}
