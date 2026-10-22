//! M1-13: delete, Recently Deleted, purge and blob GC across nodes (fake clock).

mod common;

use common::*;
use shoebox_core::clock::Clock;
use shoebox_core::ids::Blake3Hash;
use shoebox_core::model::{AlbumId, Role};
use shoebox_core::store::*;
use shoebox_node::{FetcherConfig, ImportOptions, LibraryNode, MaintenanceConfig, Protection};
use shoebox_store_docs::DocsLibrary;
use shoebox_testkit::{wait_until, Cluster};
use std::time::Duration;

fn fast() -> FetcherConfig {
    FetcherConfig {
        concurrency: 4,
        tick: Duration::from_millis(300),
        retry: Duration::from_millis(300),
    }
}

async fn holds_all(n: &LibraryNode<DocsLibrary>, id: &Blake3Hash) -> bool {
    let Some(rec) = n.store().photo(id).await.unwrap() else {
        return false;
    };
    for blob in [&rec.original, &rec.preview, &rec.thumb] {
        if !n.node().blobs().has(&blob.hash).await.unwrap() {
            return false;
        }
    }
    true
}

/// Three keepers that all hold one imported photo. Returns the nodes and the photo id.
async fn trio_with_photo(c: &Cluster) -> (Vec<LibraryNode<DocsLibrary>>, Blake3Hash) {
    let nodes = library_nodes(c).await;
    for n in &nodes {
        n.start_maintenance(MaintenanceConfig {
            interval: Duration::from_secs(3600),
            react_after: Duration::from_millis(200),
        });
    }
    for n in &nodes[1..] {
        n.start_fetcher(fast());
    }
    let samples = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/samples");
    let id = nodes[0]
        .import_paths(&[samples.join("photo-gps.jpg")], ImportOptions::default())
        .await
        .unwrap()
        .imported[0];
    wait_until(Duration::from_secs(30), || async {
        holds_all(&nodes[1], &id).await && holds_all(&nodes[2], &id).await
    })
    .await
    .expect("every keeper holds the photo");
    (nodes, id)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn delete_and_restore_round_trip_on_every_node() {
    let c = Cluster::with_roles(&[Role::Keeper; 3]).await.unwrap();
    let (nodes, id) = trio_with_photo(&c).await;

    c.clock().advance(1_000);
    nodes[1].delete_photos(&[id]).await.unwrap();
    wait_until(Duration::from_secs(20), || async {
        let mut all = true;
        for n in &nodes {
            all &= n
                .store()
                .photo(&id)
                .await
                .unwrap()
                .is_some_and(|p| p.is_deleted())
                && n.recently_deleted()
                    .await
                    .unwrap()
                    .iter()
                    .any(|p| p.id == id)
                && n.store()
                    .photos_by_time(TimeRange::ALL, Page::all())
                    .await
                    .unwrap()
                    .is_empty();
        }
        all
    })
    .await
    .expect("the delete reaches every device and the photo leaves the timeline");
    // Deleted photos keep their bytes (they are recoverable for 30 days).
    for n in &nodes {
        assert!(holds_all(n, &id).await);
    }

    c.clock().advance(1_000);
    nodes[2].restore_photos(&[id]).await.unwrap();
    wait_until(Duration::from_secs(20), || async {
        let mut all = true;
        for n in &nodes {
            all &= n
                .store()
                .photo(&id)
                .await
                .unwrap()
                .is_some_and(|p| !p.is_deleted())
                && n.recently_deleted().await.unwrap().is_empty();
        }
        all
    })
    .await
    .expect("the restore reaches every device");
    c.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn after_purge_and_gc_no_node_still_stores_the_photo() {
    let c = Cluster::with_roles(&[Role::Keeper; 3]).await.unwrap();
    let (nodes, id) = trio_with_photo(&c).await;
    let rec = nodes[0].store().photo(&id).await.unwrap().unwrap();
    let blobs = [rec.original.hash, rec.preview.hash, rec.thumb.hash];

    c.clock().advance(1_000);
    nodes[0].delete_photos(&[id]).await.unwrap();
    wait_until(Duration::from_secs(20), || async {
        nodes[1]
            .store()
            .photo(&id)
            .await
            .unwrap()
            .is_some_and(|p| p.is_deleted())
            && nodes[2]
                .store()
                .photo(&id)
                .await
                .unwrap()
                .is_some_and(|p| p.is_deleted())
    })
    .await
    .unwrap();

    // Before the retention window ends nothing is purged.
    c.clock()
        .advance(shoebox_core::store::RETENTION_MS as u64 - 10_000);
    assert!(nodes[0].purge_expired().await.unwrap().is_empty());
    assert!(holds_all(&nodes[0], &id).await);

    // After it, one device purges; the purge replicates and every node releases and collects.
    c.clock().advance(20_000);
    let purged = nodes[0].purge_expired().await.unwrap();
    assert_eq!(purged, vec![id]);
    wait_until(Duration::from_secs(30), || async {
        for n in &nodes {
            for b in &blobs {
                if n.node().blobs().has(b).await.unwrap() {
                    return false;
                }
            }
        }
        true
    })
    .await
    .expect(
        "after the purge replicates and GC runs, no node stores the original, preview or thumbnail",
    );

    // Maintenance on the other nodes notices the purge on its own and retracts their holdings.
    for n in &nodes[1..] {
        n.reconcile_blobs().await.unwrap();
    }
    wait_until(Duration::from_secs(30), || async {
        nodes[0].holders(&blobs[0]).await.unwrap().is_empty()
    })
    .await
    .expect("holdings of the purged original are retracted");
    for n in &nodes {
        assert!(
            n.store().photo(&id).await.unwrap().is_none(),
            "the record itself is gone"
        );
        assert!(n.recently_deleted().await.unwrap().is_empty());
    }
    c.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_shared_album_reference_keeps_the_blob_through_a_purge() {
    let c = Cluster::with_roles(&[Role::Keeper; 3]).await.unwrap();
    let (nodes, id) = trio_with_photo(&c).await;
    let rec = nodes[1].store().photo(&id).await.unwrap().unwrap();
    // Node 1 is a member of a shared album that uses this photo's preview (the M5 hook).
    nodes[1]
        .node()
        .blobs()
        .protect(&rec.preview.hash, Protection::Album(AlbumId([7; 16])))
        .await
        .unwrap();

    c.clock().advance(1_000);
    nodes[0].delete_photos(&[id]).await.unwrap();
    c.clock()
        .advance(shoebox_core::store::RETENTION_MS as u64 + 10_000);
    wait_until(Duration::from_secs(20), || async {
        nodes[1]
            .store()
            .photo(&id)
            .await
            .unwrap()
            .is_some_and(|p| p.is_deleted())
    })
    .await
    .unwrap();
    nodes[0].purge_expired().await.unwrap();

    wait_until(Duration::from_secs(30), || async {
        !nodes[0]
            .node()
            .blobs()
            .has(&rec.original.hash)
            .await
            .unwrap()
            && !nodes[2]
                .node()
                .blobs()
                .has(&rec.original.hash)
                .await
                .unwrap()
    })
    .await
    .expect("nodes without a reference collect the blobs");
    wait_until(Duration::from_secs(30), || async {
        !nodes[1]
            .node()
            .blobs()
            .has(&rec.original.hash)
            .await
            .unwrap()
    })
    .await
    .expect("the original is not referenced by the album, so it goes too");
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(
        nodes[1]
            .node()
            .blobs()
            .has(&rec.preview.hash)
            .await
            .unwrap(),
        "the album-referenced preview survives the purge and GC"
    );
    assert!(!nodes[0]
        .node()
        .blobs()
        .has(&rec.preview.hash)
        .await
        .unwrap());
    let _ = c.clock().now_ms();
    c.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reconcile_keeps_everything_a_record_still_references() {
    let c = Cluster::with_roles(&[Role::Keeper]).await.unwrap();
    let nodes = library_nodes(&c).await;
    let samples = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/samples");
    let ids = nodes[0]
        .import_paths(
            &[
                samples.join("photo-gps.jpg"),
                samples.join("screenshot.png"),
            ],
            ImportOptions::default(),
        )
        .await
        .unwrap()
        .imported;
    // A soft-deleted photo must keep its bytes; live ones too.
    nodes[0].delete_photos(&ids[..1]).await.unwrap();
    let released = nodes[0].reconcile_blobs().await.unwrap();
    assert!(released.is_empty(), "nothing is unreferenced: {released:?}");
    tokio::time::sleep(Duration::from_secs(1)).await;
    for id in &ids {
        assert!(holds_all(&nodes[0], id).await);
    }
    // An orphan blob (library-protected but referenced by no record) is released.
    let orphan = nodes[0]
        .node()
        .blobs()
        .add_bytes(b"orphan".to_vec(), Protection::Library)
        .await
        .unwrap();
    let released = nodes[0].reconcile_blobs().await.unwrap();
    assert_eq!(released, vec![orphan]);
    wait_until(Duration::from_secs(10), || async {
        !nodes[0].node().blobs().has(&orphan).await.unwrap()
    })
    .await
    .expect("the orphan is collected");
    c.shutdown().await;
}
