//! A phone's gallery photos are referenced where they are, never copied (one copy on the phone),
//! and a deletion in the gallery deletes them from the library.

mod common;

use common::*;
use shoebox_core::ids::Blake3Hash;
use shoebox_core::model::Role;
use shoebox_core::store::{LibraryStore, Page, TimeRange};
use shoebox_node::{FetcherConfig, ImportMode, ImportOptions, LibraryNode, Protection};
use shoebox_store::Library;
use shoebox_testkit::{wait_until, Cluster};
use std::path::{Path, PathBuf};
use std::time::Duration;

fn fast() -> FetcherConfig {
    FetcherConfig {
        concurrency: 6,
        tick: Duration::from_millis(300),
        retry: Duration::from_millis(300),
    }
}

/// A "gallery" folder holding copies of the sample photos.
fn gallery(dir: &Path) -> Vec<PathBuf> {
    let samples = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/samples");
    std::fs::create_dir_all(dir).unwrap();
    ["photo-gps.jpg", "IMG_0001.jpg"]
        .iter()
        .map(|f| {
            let to = dir.join(f);
            std::fs::copy(samples.join(f), &to).unwrap();
            to
        })
        .collect()
}

fn in_place() -> ImportOptions {
    ImportOptions {
        mode: ImportMode::Reference,
        ..ImportOptions::default()
    }
}

/// Bytes the phone's blob store keeps on disk (data files only; the database is small).
fn store_bytes(dir: &Path) -> u64 {
    fn walk(d: &Path) -> u64 {
        let Ok(entries) = std::fs::read_dir(d) else {
            return 0;
        };
        entries
            .flatten()
            .map(|e| {
                let p = e.path();
                if p.is_dir() {
                    walk(&p)
                } else if p.extension().is_some_and(|x| x == "data") {
                    e.metadata().map(|m| m.len()).unwrap_or(0)
                } else {
                    0
                }
            })
            .sum()
    }
    walk(dir)
}

async fn phone_and_keeper() -> (Cluster, Vec<LibraryNode<Library>>) {
    let c = Cluster::with_roles(&[Role::Companion, Role::Keeper])
        .await
        .unwrap();
    let nodes = library_nodes_auto(&c).await;
    nodes[1].start_fetcher(fast());
    (c, nodes)
}

async fn keeper_holds(nodes: &[LibraryNode<Library>], ids: &[Blake3Hash]) {
    wait_until(Duration::from_secs(40), || async {
        for id in ids {
            if nodes[0].copy_count(id).await.unwrap() != 2 {
                return false;
            }
        }
        true
    })
    .await
    .expect("the keeper fetches the originals and says so");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn gallery_photos_are_referenced_not_copied_and_never_freed() {
    let (c, nodes) = phone_and_keeper().await;
    let dir = tempfile::tempdir().unwrap();
    let files = gallery(dir.path());
    let phone_blobs = c.node(0).config().data_dir.join("blobs");
    let before = store_bytes(&phone_blobs);
    let report = nodes[0].import_paths(&files, in_place()).await.unwrap();
    assert_eq!(report.imported.len(), 2);
    assert_eq!(report.by_path.len(), 2);
    let originals: u64 = files.iter().map(|f| f.metadata().unwrap().len()).sum();
    // Previews and thumbnails are the store's own; the originals are not copied in.
    let mut renditions = 0;
    for id in &report.imported {
        let rec = nodes[0].store().photo(id).await.unwrap().unwrap();
        renditions += rec.preview.size + rec.thumb.size;
    }
    let grew = store_bytes(&phone_blobs) - before;
    assert!(
        grew <= renditions,
        "originals were copied into the store: grew {grew}, renditions {renditions}, originals {originals}"
    );
    for id in &report.imported {
        assert!(nodes[0].is_in_place(id).await.unwrap());
        assert!(nodes[0]
            .node()
            .blobs()
            .protections(id)
            .await
            .unwrap()
            .contains(&Protection::InPlace));
    }
    // Served from the gallery file: the keeper gets the real bytes.
    keeper_holds(&nodes, &report.imported).await;
    let (path, id) = &report.by_path[0];
    assert!(
        nodes[1].node().blobs().get(id).await.unwrap() == std::fs::read(path).unwrap(),
        "the keeper has the gallery file's bytes"
    );
    // Backed up on a keeper, yet nothing to free: the file is the gallery's.
    assert!(nodes[0]
        .free_up_space(true)
        .await
        .unwrap()
        .photos
        .is_empty());
    c.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_gallery_deletion_deletes_the_photo_everywhere() {
    let (c, nodes) = phone_and_keeper().await;
    let dir = tempfile::tempdir().unwrap();
    let files = gallery(dir.path());
    let report = nodes[0].import_paths(&files, in_place()).await.unwrap();
    // In `files` order (the importer sorts paths).
    let ids: Vec<Blake3Hash> = files
        .iter()
        .map(|f| report.by_path.iter().find(|(p, _)| p == f).unwrap().1)
        .collect();
    keeper_holds(&nodes, &ids).await;

    std::fs::remove_file(&files[0]).unwrap();
    nodes[0].gallery_removed(&ids[..1]).await.unwrap();

    // In Recently Deleted on both devices; the other photo is untouched.
    wait_until(Duration::from_secs(20), || async {
        let live = nodes[1]
            .store()
            .photos_by_time(TimeRange::ALL, Page::all())
            .await
            .unwrap();
        live.len() == 1
            && live[0].id == ids[1]
            && nodes[1].recently_deleted().await.unwrap().len() == 1
    })
    .await
    .expect("the deletion reaches the keeper");
    // The phone no longer claims the original; the keeper keeps its copy until the purge.
    wait_until(Duration::from_secs(20), || async {
        nodes[1].copy_count(&ids[0]).await.unwrap() == 1
    })
    .await
    .expect("the phone's holding is retracted");
    assert!(nodes[1].node().blobs().has(&ids[0]).await.unwrap());
    wait_until(Duration::from_secs(10), || async {
        !nodes[0].node().blobs().has(&ids[0]).await.unwrap()
    })
    .await
    .expect("the phone drops its reference");
    // The other gallery file is never touched by the store's garbage collection.
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(files[1].exists());
    c.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stored_copy_of_a_gallery_file_becomes_a_reference() {
    let (c, nodes) = phone_and_keeper().await;
    let dir = tempfile::tempdir().unwrap();
    let files = gallery(dir.path());
    // An earlier build copied the photos in.
    let report = nodes[0]
        .import_paths(&files, ImportOptions::default())
        .await
        .unwrap();
    let ids: Vec<Blake3Hash> = files
        .iter()
        .map(|f| report.by_path.iter().find(|(p, _)| p == f).unwrap().1)
        .collect();
    let phone_blobs = c.node(0).config().data_dir.join("blobs");
    let with_copies = store_bytes(&phone_blobs);
    let id = ids[0];
    assert!(!nodes[0].is_in_place(&id).await.unwrap());
    nodes[0].pin_original(&id).await.unwrap();

    nodes[0].begin_move_in_place(&id).await.unwrap();
    wait_until(Duration::from_secs(10), || async {
        nodes[0].finish_move_in_place(&id, &files[0]).await.unwrap()
    })
    .await
    .expect("the copy is collected and the file referenced");
    assert!(nodes[0].is_in_place(&id).await.unwrap());
    assert!(
        nodes[0]
            .node()
            .blobs()
            .protections(&id)
            .await
            .unwrap()
            .contains(&Protection::Pin),
        "a pin survives the move"
    );
    assert!(nodes[0].node().blobs().get(&id).await.unwrap() == std::fs::read(&files[0]).unwrap());
    let size = files[0].metadata().unwrap().len();
    assert!(
        store_bytes(&phone_blobs) + size / 2 < with_copies,
        "the copy is gone"
    );
    // Still held as far as the library knows.
    assert!(nodes[0].copy_count(&id).await.unwrap() >= 1);
    c.shutdown().await;
}

/// A gallery video is referenced and served like a photo: the keeper gets its bytes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_gallery_video_reaches_the_keeper() {
    let (_c, nodes) = phone_and_keeper().await;
    let dir = tempfile::tempdir().unwrap();
    let samples = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/samples");
    let file = dir.path().join("VID_0001.mp4");
    std::fs::copy(samples.join("clip.mp4"), &file).unwrap();
    let report = nodes[0]
        .import_paths(std::slice::from_ref(&file), in_place())
        .await
        .unwrap();
    assert_eq!(report.imported.len(), 1, "{report:?}");
    let id = report.imported[0];
    assert!(nodes[0].is_in_place(&id).await.unwrap());
    keeper_holds(&nodes, &report.imported).await;
    assert_eq!(
        nodes[1].node().blobs().get(&id).await.unwrap(),
        std::fs::read(&file).unwrap()
    );
}
