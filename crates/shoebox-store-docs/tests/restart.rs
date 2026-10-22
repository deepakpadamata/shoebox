//! The library is rebuilt from iroh-docs entries when a node restarts.

mod common;

use common::*;
use shoebox_core::clock::ManualClock;
use shoebox_core::store::*;
use shoebox_node::{Node, NodeConfig};
use shoebox_store_docs::DocsBackend;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn library_state_survives_a_node_restart() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = || NodeConfig::local_only(dir.path().join("n"), "n");
    let clock = ManualClock::new(5_000);

    let node = Node::open(cfg()).await.unwrap();
    let backend = DocsBackend::for_node(node.clone(), clock.clone());
    let (lib, cap) = backend
        .create_library(&RootPublicKey([1; 32]))
        .await
        .unwrap();
    for n in [1u8, 2, 3] {
        lib.upsert_photo(photo(n, n as i64)).await.unwrap();
    }
    lib.set_favorite(&hash(2), true).await.unwrap();
    lib.mark_deleted(&hash(3)).await.unwrap();
    lib.upsert_album(album()).await.unwrap();
    lib.album_add(&shoebox_core::model::AlbumId([9; 16]), &[hash(1)])
        .await
        .unwrap();
    let before_2 = lib.photo(&hash(2)).await.unwrap().unwrap();
    drop(lib);
    drop(backend);
    node.shutdown().await.unwrap();
    drop(node);

    let node = Node::open(cfg()).await.unwrap();
    let backend = DocsBackend::for_node(node.clone(), clock.clone());
    let lib = backend.open_library(&cap).await.unwrap();
    assert_eq!(lib.photo(&hash(2)).await.unwrap().unwrap(), before_2);
    assert!(lib.photo(&hash(3)).await.unwrap().unwrap().is_deleted());
    let live: Vec<_> = lib
        .photos_by_time(TimeRange::ALL, Page::all())
        .await
        .unwrap()
        .into_iter()
        .map(|p| p.id)
        .collect();
    assert_eq!(live, vec![hash(2), hash(1)]);
    assert_eq!(
        lib.album_items(&shoebox_core::model::AlbumId([9; 16]), Page::all())
            .await
            .unwrap(),
        vec![hash(1)]
    );

    // A write after the restart still orders after everything that was stored (HLC restored).
    clock.set(1); // wall clock went backwards across the restart
    lib.set_favorite(&hash(2), false).await.unwrap();
    assert!(!lib.photo(&hash(2)).await.unwrap().unwrap().favorite.value);
    node.shutdown().await.unwrap();
}
