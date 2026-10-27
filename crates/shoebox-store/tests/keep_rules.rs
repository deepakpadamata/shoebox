//! A Companion keeps the originals of recent photos (a window of days), of favourites (a toggle)
//! and of pins; everything else lives on keepers. Free-up never touches what it keeps.

mod common;

use common::*;
use shoebox_core::ids::Blake3Hash;
use shoebox_core::model::Role;
use shoebox_core::store::LibraryStore;
use shoebox_node::{FetcherConfig, ImportOptions};
use shoebox_testkit::{wait_until, Cluster};
use std::path::PathBuf;
use std::time::Duration;

const DAY: u64 = 86_400_000;
/// 2024-06-01 12:00 UTC, the date in the sample photo's EXIF.
const SAMPLE_TAKEN: u64 = 1_717_243_200_000;

fn fast() -> FetcherConfig {
    FetcherConfig {
        concurrency: 6,
        tick: Duration::from_millis(300),
        retry: Duration::from_millis(300),
    }
}

/// Two photos: `recent` taken 2024-06-01, `old` taken 2023-01-01 (the same JPEG with another date).
fn photos(dir: &std::path::Path) -> (PathBuf, PathBuf) {
    let samples = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/samples");
    let recent = dir.join("recent.jpg");
    std::fs::copy(samples.join("photo-gps.jpg"), &recent).unwrap();
    let mut bytes = std::fs::read(samples.join("IMG_0001.jpg")).unwrap();
    let at = bytes
        .windows(19)
        .position(|w| w == b"2024:06:01 12:00:00")
        .expect("the sample carries its date");
    bytes[at..at + 19].copy_from_slice(b"2023:01:01 12:00:00");
    let old = dir.join("old.jpg");
    std::fs::write(&old, bytes).unwrap();
    (recent, old)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_companion_keeps_recent_and_favourite_originals_and_free_up_leaves_them() {
    let c = Cluster::with_roles(&[Role::Keeper, Role::Companion])
        .await
        .unwrap();
    let nodes = library_nodes_auto(&c).await;
    let dir = tempfile::tempdir().unwrap();
    let (recent_file, old_file) = photos(dir.path());
    c.clock().set(SAMPLE_TAKEN + 10 * DAY);
    let report = nodes[0]
        .import_paths(
            &[recent_file.clone(), old_file.clone()],
            ImportOptions::default(),
        )
        .await
        .unwrap();
    let id_of =
        |f: &PathBuf| -> Blake3Hash { report.by_path.iter().find(|(p, _)| p == f).unwrap().1 };
    let (recent, old) = (id_of(&recent_file), id_of(&old_file));

    // 30-day window, no favourites: the companion gets the recent original only.
    nodes[1].set_keep_rules(30, false).unwrap();
    nodes[1].start_fetcher(fast());
    wait_until(Duration::from_secs(30), || async {
        nodes[1].node().blobs().has(&recent).await.unwrap()
    })
    .await
    .expect("the recent original is fetched");
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(
        !nodes[1].node().blobs().has(&old).await.unwrap(),
        "an old photo stays on the keeper"
    );

    // A favourite is kept once the toggle is on.
    nodes[0].store().set_favorite(&old, true).await.unwrap();
    nodes[1].set_keep_rules(30, true).unwrap();
    wait_until(Duration::from_secs(30), || async {
        nodes[1].node().blobs().has(&old).await.unwrap()
    })
    .await
    .expect("the favourite's original is fetched");

    // Free-up: the keeper holds both, but the recent photo and the favourite are kept.
    wait_until(Duration::from_secs(30), || async {
        nodes[1].copy_count(&old).await.unwrap() == 2
            && nodes[1].copy_count(&recent).await.unwrap() == 2
    })
    .await
    .expect("holdings settle");
    assert!(nodes[1]
        .free_up_space(true)
        .await
        .unwrap()
        .photos
        .is_empty());
    // Favourites off: the old photo can go, the recent one cannot.
    nodes[1].set_keep_rules(30, false).unwrap();
    assert_eq!(
        nodes[1].free_up_space(true).await.unwrap().photos,
        vec![old]
    );
    // Pinned: never.
    nodes[1].pin_original(&old).await.unwrap();
    assert!(nodes[1]
        .free_up_space(true)
        .await
        .unwrap()
        .photos
        .is_empty());
    nodes[1].unpin_original(&old).await.unwrap();
    // Window gone (a year later, or switched off): the recent one is freeable too.
    c.clock().advance(200 * DAY);
    let mut freeable = nodes[1].free_up_space(true).await.unwrap().photos;
    freeable.sort();
    let mut both = vec![recent, old];
    both.sort();
    assert_eq!(freeable, both);
    nodes[1].set_keep_rules(0, false).unwrap();
    assert_eq!(nodes[1].keep_rules(), (0, false));
    c.shutdown().await;
}
