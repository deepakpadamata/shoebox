//! A device that holds an original keeps no preview next to it (`drop_previews`), and makes the
//! preview again, with the hash the library has, when another device needs it.

mod common;

use common::*;
use shoebox_core::model::Role;
use shoebox_core::store::LibraryStore;
use shoebox_node::{ImportOptions, Protection};
use shoebox_testkit::{wait_until, Cluster};
use std::path::PathBuf;
use std::time::Duration;

fn sample() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/samples/IMG_0001.jpg")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_device_alone_keeps_no_preview_and_can_make_it_again() {
    let c = Cluster::with_roles(&[Role::Companion]).await.unwrap();
    let nodes = library_nodes_auto(&c).await;
    let phone = &nodes[0];
    phone
        .node()
        .update_config(|c| c.drop_previews = true)
        .unwrap();
    let id = phone
        .import_paths(&[sample()], ImportOptions::default())
        .await
        .unwrap()
        .imported[0];
    let rec = phone.store().photo(&id).await.unwrap().unwrap();
    let blobs = phone.node().blobs();
    assert!(
        blobs.has(&rec.thumb.hash).await.unwrap(),
        "the thumbnail stays"
    );
    assert!(
        !blobs.has(&rec.preview.hash).await.unwrap(),
        "no preview is stored next to a held original"
    );
    assert_ne!(rec.preview.hash, rec.original.hash);

    // Made again from the original: the very same bytes, so the record still finds them.
    assert!(phone.ensure_preview(&rec).await.unwrap());
    assert!(blobs.has(&rec.preview.hash).await.unwrap());
    assert_eq!(
        blobs.size(&rec.preview.hash).await.unwrap(),
        Some(rec.preview.size)
    );

    // And let go again by the next maintenance pass.
    let released = phone.release_previews().await.unwrap();
    assert_eq!(released, vec![rec.preview.hash]);
    c.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn with_another_device_the_preview_is_kept_and_made_again_if_it_was_lost() {
    let c = Cluster::with_roles(&[Role::Companion, Role::Companion])
        .await
        .unwrap();
    let nodes = library_nodes_auto(&c).await;
    let phone = &nodes[0];
    phone
        .node()
        .update_config(|c| c.drop_previews = true)
        .unwrap();
    let id = phone
        .import_paths(&[sample()], ImportOptions::default())
        .await
        .unwrap()
        .imported[0];
    let rec = phone.store().photo(&id).await.unwrap().unwrap();
    let blobs = phone.node().blobs();
    assert!(
        blobs.has(&rec.preview.hash).await.unwrap(),
        "another device may need it: it is kept"
    );
    assert!(
        phone.release_previews().await.unwrap().is_empty(),
        "no keeper holds it, so it is not released"
    );

    // Lost (a library that began on one phone): nobody holds it, the phone makes it again.
    blobs
        .unprotect(&rec.preview.hash, Protection::Library)
        .await
        .unwrap();
    phone
        .retract_holdings(vec![rec.preview.hash])
        .await
        .unwrap();
    blobs.remove(&rec.preview.hash).await.unwrap();
    wait_until(Duration::from_secs(20), || async {
        !blobs.has(&rec.preview.hash).await.unwrap()
    })
    .await
    .expect("the store collects the unprotected preview");
    assert_eq!(phone.restore_previews().await.unwrap(), 1);
    assert!(blobs.has(&rec.preview.hash).await.unwrap());
    c.shutdown().await;
}
