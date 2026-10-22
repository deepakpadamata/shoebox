//! M5-01/02: an album survives a restart, and the member/item limits are enforced.

mod common;

use common::*;
use shoebox_core::clock::ManualClock;
use shoebox_core::ids::EndpointId;
use shoebox_core::model::*;
use shoebox_core::store::*;
use shoebox_core::Timestamp;
use shoebox_node::{Node, NodeConfig};
use shoebox_store_docs::DocsBackend;

fn item(n: u16) -> SharedItem {
    let mut h = [0u8; 32];
    h[..2].copy_from_slice(&n.to_le_bytes());
    let b = BlobRef {
        hash: shoebox_core::Blake3Hash(h),
        size: 10,
        mime: "image/jpeg".into(),
    };
    SharedItem {
        id: b.hash,
        original: b.clone(),
        shared_hash: None,
        preview: b.clone(),
        thumb: b,
        added_by: dev(1),
        added_at: Timestamp(n as i64),
        taken_at: Timestamp(0),
    }
}

fn meta(by: EndpointId) -> SharedAlbumMeta {
    SharedAlbumMeta {
        id: AlbumId([4; 16]),
        name: "Trip".into(),
        cover: None,
        created_by: by,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_album_reopens_after_a_restart_with_its_items_and_members() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = || NodeConfig::local_only(dir.path().join("n"), "n");
    let clock = ManualClock::new(5_000);
    let node = Node::open(cfg()).await.unwrap();
    let backend = DocsBackend::for_node(node.clone(), clock.clone());
    let me = backend.device();
    let album = backend.create_album(meta(me)).await.unwrap();
    album.add_items(vec![item(1), item(2)]).await.unwrap();
    let cap = album.capability().await.unwrap();
    drop(album);
    drop(backend);
    node.shutdown().await.unwrap();
    drop(node);

    let node = Node::open(cfg()).await.unwrap();
    let backend = DocsBackend::for_node(node.clone(), clock.clone());
    let album = backend.open_album(&cap).await.unwrap();
    assert_eq!(album.meta().await.unwrap().name, "Trip");
    assert_eq!(album.items(Page::all()).await.unwrap().len(), 2);
    assert_eq!(album.members().await.unwrap().len(), 1);
    album
        .add_items(vec![item(3)])
        .await
        .expect("still writable");
    assert_eq!(album.items(Page::all()).await.unwrap().len(), 3);
    node.shutdown().await.unwrap();
}

// 50,000 signed writes take about a minute: `cargo test -- --ignored`.
#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_item_limit_is_enforced_with_a_clear_message() {
    let dir = tempfile::tempdir().unwrap();
    let node = Node::open(NodeConfig::local_only(dir.path().join("n"), "n"))
        .await
        .unwrap();
    let backend = DocsBackend::for_node(node.clone(), ManualClock::new(1_000));
    let album = backend.create_album(meta(backend.device())).await.unwrap();
    let many = |from: u32, to: u32| -> Vec<SharedItem> {
        (from..to)
            .map(|n| {
                let mut i = item(0);
                let mut h = [0u8; 32];
                h[..4].copy_from_slice(&n.to_le_bytes());
                i.id = shoebox_core::Blake3Hash(h);
                i.original.hash = i.id;
                i.preview.hash = i.id;
                i.thumb.hash = i.id;
                i.added_at = Timestamp(n as i64);
                i
            })
            .collect()
    };
    album.add_items(many(0, 20_000)).await.unwrap();
    album.add_items(many(20_000, 50_000)).await.unwrap();
    let err = album.add_items(many(50_000, 50_001)).await.unwrap_err();
    assert!(err.to_string().contains("at most 50000"), "{err}");
    // Re-adding existing items is not "more".
    album.add_items(many(0, 10)).await.unwrap();
    node.shutdown().await.unwrap();
}
