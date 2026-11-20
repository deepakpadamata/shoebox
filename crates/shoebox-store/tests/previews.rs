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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_jpeg_preview_is_converted_to_avif_once_and_every_device_takes_it() {
    use shoebox_core::merge::Lww;
    use shoebox_core::model::{BlobKind, BlobRef, PhotoRecord};
    let c = Cluster::with_roles(&[Role::Companion, Role::Keeper])
        .await
        .unwrap();
    let nodes = library_nodes_auto(&c).await;
    let (phone, keeper) = (&nodes[0], &nodes[1]);

    // A photo imported before previews were AVIF: a 2048 px JPEG preview, held by both devices,
    // the original held by both too.
    let bytes = std::fs::read(sample()).unwrap();
    let a = shoebox_media::analyze(&sample()).unwrap();
    let jpeg = shoebox_media::remake_preview(&bytes, "image/jpeg", "image/jpeg").unwrap();
    let thumb = a.thumb.unwrap().bytes;
    let mut refs = vec![];
    for n in [phone, keeper] {
        let b = n.node().blobs();
        let o = b
            .add_bytes(bytes.clone(), Protection::Library)
            .await
            .unwrap();
        let p = b
            .add_bytes(jpeg.clone(), Protection::Library)
            .await
            .unwrap();
        let t = b
            .add_bytes(thumb.clone(), Protection::Library)
            .await
            .unwrap();
        n.publish_holdings(vec![
            (o, BlobKind::Original),
            (p, BlobKind::Preview),
            (t, BlobKind::Thumb),
        ])
        .await
        .unwrap();
        refs = vec![o, p, t];
    }
    let blob = |hash, size: usize, mime: &str| BlobRef {
        hash,
        size: size as u64,
        mime: mime.into(),
    };
    let old = PhotoRecord {
        id: a.hash,
        original: blob(refs[0], bytes.len(), &a.mime),
        preview: blob(refs[1], jpeg.len(), "image/jpeg"),
        thumb: blob(refs[2], thumb.len(), "image/jpeg"),
        taken_at: a.taken_at,
        exif: a.exif.clone(),
        imported_by: phone.device(),
        favorite: Lww::new(false, hlc(1, 0, 1)),
        deleted_at: Lww::new(None, hlc(1, 0, 1)),
    };
    phone.store().upsert_photo(old.clone()).await.unwrap();
    wait_until(Duration::from_secs(20), || async {
        keeper.store().photo(&a.hash).await.unwrap().is_some()
    })
    .await
    .expect("the record reaches the keeper");
    wait_until(Duration::from_secs(20), || async {
        keeper.store().holders(&refs[0]).await.unwrap().len() == 2
    })
    .await
    .expect("both holdings reach the keeper");

    // The phone holds the original too, but the keeper converts.
    assert_eq!(phone.convert_previews(10).await.unwrap(), 0);
    assert_eq!(keeper.convert_previews(10).await.unwrap(), 1);
    let rec = keeper.store().photo(&a.hash).await.unwrap().unwrap();
    assert_eq!(rec.preview.mime, "image/avif");
    assert!(
        rec.preview.size < jpeg.len() as u64,
        "{} vs {}",
        rec.preview.size,
        jpeg.len()
    );
    let avif = shoebox_media::remake_preview(&bytes, "image/jpeg", "image/avif").unwrap();
    assert_eq!(
        rec.preview.hash,
        shoebox_core::Blake3Hash::of_bytes(&avif),
        "the same bytes a device that dropped it makes again"
    );
    assert!(keeper.node().blobs().has(&rec.preview.hash).await.unwrap());
    assert_eq!(keeper.convert_previews(10).await.unwrap(), 0, "once");

    // Every device takes it, whichever order the two versions arrive in.
    wait_until(Duration::from_secs(20), || async {
        phone.store().photo(&a.hash).await.unwrap().unwrap().preview == rec.preview
    })
    .await
    .expect("the phone takes the AVIF preview");
    phone.store().upsert_photo(old).await.unwrap();
    assert_eq!(
        phone.store().photo(&a.hash).await.unwrap().unwrap().preview,
        rec.preview,
        "the old version does not win back"
    );

    // And the old JPEG is let go.
    assert!(keeper.reconcile_blobs().await.unwrap().contains(&refs[1]));
    assert!(phone.reconcile_blobs().await.unwrap().contains(&refs[1]));
    c.shutdown().await;
}
