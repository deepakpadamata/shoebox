//! M5-01 acceptance: an album with 50k items reaches a new member in under two minutes on a
//! LAN (metadata only). Slow (the owner signs 50,000 writes first): `cargo test -- --ignored`.

use shoebox_core::ids::Blake3Hash;
use shoebox_core::model::*;
use shoebox_core::store::*;
use shoebox_core::Timestamp;
use shoebox_node::{Node, NodeConfig};
use shoebox_store::SharedAlbums;
use std::time::{Duration, Instant};

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn album_sync_of_50k_items() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
    // Production-like nodes: blob GC every five minutes (the testkit's 200 ms GC walks every
    // docs record each time, which is not what a real device does).
    let dirs: Vec<_> = (0..2).map(|_| tempfile::tempdir().unwrap()).collect();
    let n0 = Node::open(NodeConfig::local_only(dirs[0].path().join("node"), "owner"))
        .await
        .unwrap();
    let n1 = Node::open(NodeConfig::local_only(
        dirs[1].path().join("node"),
        "friend",
    ))
    .await
    .unwrap();
    n0.add_peer_addr(n1.addr());
    n1.add_peer_addr(n0.addr());
    let owner = SharedAlbums::open(n0.clone(), dirs[0].path(), false)
        .await
        .unwrap();
    let friend = SharedAlbums::open(n1.clone(), dirs[1].path(), false)
        .await
        .unwrap();
    let id = owner.create("Big", "owner").await.unwrap();
    let items: Vec<SharedItem> = (0..50_000u32)
        .map(|n| {
            let mut h = [0u8; 32];
            h[..4].copy_from_slice(&n.to_le_bytes());
            let b = BlobRef {
                hash: Blake3Hash(h),
                size: 10,
                mime: "image/jpeg".into(),
            };
            SharedItem {
                id: b.hash,
                original: b.clone(),
                shared_hash: None,
                preview: b.clone(),
                thumb: b,
                added_by: shoebox_node::from_iroh_id(&n0.id()),
                added_at: Timestamp(n as i64),
                taken_at: Timestamp(0),
            }
        })
        .collect();
    let started = Instant::now();
    owner.album(&id).unwrap().add_items(items).await.unwrap();
    eprintln!("owner wrote 50k items in {:?}", started.elapsed());

    let text = owner.invite(&id, MemberRole::Contributor, 1).await.unwrap();
    let joined = Instant::now();
    friend.join(&text, "friend").await.unwrap();
    let album = friend.album(&id).unwrap();
    while album.item_count().await < 50_000 {
        assert!(
            joined.elapsed() < Duration::from_secs(120),
            "{} items after 2 min",
            album.item_count().await
        );
        friend.sync_now(&id).await.ok();
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let took = joined.elapsed();
    eprintln!("new member has 50,000 items after {took:?}");
    assert!(took < Duration::from_secs(120));
}
