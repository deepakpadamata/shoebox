//! M1-06: folder import with dedupe, progress events and watched folders. Uses the in-memory
//! store backend; the docs backend is exercised by the multi-node tests.

use shoebox_core::clock::{Clock, ManualClock};
use shoebox_core::ids::Timestamp;
use shoebox_core::model::*;
use shoebox_core::store::*;
use shoebox_node::{ImportMode, ImportOptions, LibraryNode, Node, NodeConfig, NodeEvent};
use shoebox_store_mem::{MemBackend, MemLibrary, MemNetwork};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

struct Rig {
    ln: LibraryNode<MemLibrary>,
    clock: ManualClock,
    dir: tempfile::TempDir,
}

async fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    open_in(dir, ManualClock::new(1_700_000_000_000)).await
}

async fn open_in(dir: tempfile::TempDir, clock: ManualClock) -> Rig {
    let node = Node::open(NodeConfig::local_only(dir.path().join("node"), "n"))
        .await
        .unwrap();
    let device = shoebox_core::ids::EndpointId(*node.id().as_bytes());
    let backend = MemBackend::new(MemNetwork::new(), device, clock.clone());
    let (lib, _) = backend
        .create_library(&RootPublicKey([1; 32]))
        .await
        .unwrap();
    let ln = LibraryNode::new(node, Arc::new(lib), clock.clone());
    Rig { ln, clock, dir }
}

fn corpus(dir: &Path, name: &str) -> (PathBuf, xtask::corpus::Summary) {
    let out = dir.join(name);
    let summary = xtask::corpus::generate(&xtask::corpus::Options {
        preset: xtask::corpus::Preset::Small,
        out_dir: out.clone(),
        seed: 7,
        metadata_only: None,
    })
    .unwrap();
    (out.join("photos"), summary)
}

async fn all_photos(l: &MemLibrary) -> Vec<PhotoRecord> {
    l.photos_by_time(TimeRange::ALL, Page::all()).await.unwrap()
}

#[tokio::test]
async fn importing_a_folder_creates_one_record_per_unique_photo() {
    let r = rig().await;
    let (folder, summary) = corpus(r.dir.path(), "c");
    let report =
        r.ln.import_paths(&[folder], ImportOptions::default())
            .await
            .unwrap();

    let unique = summary.photos - summary.duplicates;
    assert_eq!(report.imported.len(), unique);
    assert_eq!(
        report.duplicates.len(),
        summary.duplicates,
        "exact copies are recognised by hash"
    );
    assert!(
        report.failed.is_empty() && report.skipped.is_empty(),
        "{report:?}"
    );

    let photos = all_photos(r.ln.store()).await;
    assert_eq!(photos.len(), unique);
    let device = r.ln.device();
    for p in &photos {
        assert_eq!(p.imported_by, device);
        // Original, preview and thumbnail are stored and protected.
        for blob in [&p.original, &p.preview, &p.thumb] {
            assert!(
                r.ln.node().blobs().has(&blob.hash).await.unwrap(),
                "{} stored",
                blob.mime
            );
        }
        assert_eq!(p.id, p.original.hash);
        assert_eq!(p.preview.mime, "image/jpeg");
        // This device holds all three and publishes it.
        let holders = r.ln.store().holders(&p.original.hash).await.unwrap();
        assert_eq!(holders.len(), 1);
        assert_eq!(
            (holders[0].device, holders[0].kind),
            (device, BlobKind::Original)
        );
    }
    // 'at risk': a single copy everywhere.
    assert_eq!(
        r.ln.store().at_risk(2, Page::all()).await.unwrap().len(),
        unique
    );
    assert!(r
        .ln
        .store()
        .at_risk(1, Page::all())
        .await
        .unwrap()
        .is_empty());
    r.ln.node().shutdown().await.unwrap();
}

#[tokio::test]
async fn importing_the_same_folder_twice_creates_no_new_records() {
    let r = rig().await;
    let (folder, summary) = corpus(r.dir.path(), "c");
    r.ln.import_paths(std::slice::from_ref(&folder), ImportOptions::default())
        .await
        .unwrap();
    let before = all_photos(r.ln.store()).await;

    let again =
        r.ln.import_paths(&[folder], ImportOptions::default())
            .await
            .unwrap();
    assert!(again.imported.is_empty(), "nothing new");
    assert_eq!(
        again.duplicates.len(),
        summary.photos,
        "every file is a known photo"
    );
    assert_eq!(
        all_photos(r.ln.store()).await,
        before,
        "records are untouched"
    );
    r.ln.node().shutdown().await.unwrap();
}

#[tokio::test]
async fn unsupported_and_broken_files_do_not_stop_the_import() {
    let r = rig().await;
    let dir = r.dir.path().join("mixed");
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    let samples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/samples");
    std::fs::copy(samples.join("photo-gps.jpg"), dir.join("a.jpg")).unwrap();
    std::fs::copy(samples.join("screenshot.png"), dir.join("sub/b.png")).unwrap();
    std::fs::write(dir.join("notes.txt"), b"not a photo").unwrap();
    std::fs::write(dir.join(".hidden.jpg"), b"hidden files are ignored").unwrap();
    let good = std::fs::read(samples.join("photo-gps.jpg")).unwrap();
    std::fs::write(dir.join("cut.jpg"), &good[..good.len() / 3]).unwrap();

    let report =
        r.ln.import_paths(std::slice::from_ref(&dir), ImportOptions::default())
            .await
            .unwrap();
    assert_eq!(report.imported.len(), 2, "{report:?}");
    assert_eq!(report.skipped, vec![dir.join("notes.txt")]);
    assert_eq!(report.failed.len(), 1);
    assert_eq!(report.failed[0].0, dir.join("cut.jpg"));
    assert!(
        report.failed[0].1.contains("truncated"),
        "{}",
        report.failed[0].1
    );
    r.ln.node().shutdown().await.unwrap();
}

#[tokio::test]
async fn videos_are_imported_with_placeholder_renditions() {
    let r = rig().await;
    let samples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/samples");
    let report =
        r.ln.import_paths(&[samples.join("clip.mp4")], ImportOptions::default())
            .await
            .unwrap();
    assert_eq!(report.imported.len(), 1, "{report:?}");
    let p =
        r.ln.store()
            .photo(&report.imported[0])
            .await
            .unwrap()
            .unwrap();
    assert_eq!(p.original.mime, "video/mp4");
    assert_eq!(
        p.thumb, p.original,
        "until a device adds a poster frame, the original stands in"
    );
    assert_eq!(p.exif.width, 320);
    r.ln.node().shutdown().await.unwrap();
}

#[tokio::test]
async fn a_video_poster_becomes_its_thumb_once() {
    let r = rig().await;
    let samples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/samples");
    let report =
        r.ln.import_paths(&[samples.join("clip.mp4")], ImportOptions::default())
            .await
            .unwrap();
    let id = report.imported[0];
    let poster = |b: u8| [vec![0xFF, 0xD8, 0xFF, 0xE0], vec![b; 64]].concat();
    assert!(r
        .ln
        .set_video_poster(&id, b"not a jpeg".to_vec())
        .await
        .is_err());
    assert!(r.ln.set_video_poster(&id, poster(1)).await.unwrap());
    let p = r.ln.store().photo(&id).await.unwrap().unwrap();
    assert_ne!(p.thumb.hash, p.original.hash);
    assert_eq!(p.thumb.mime, "image/jpeg");
    assert_eq!(p.preview, p.original, "the viewer still plays the video");
    assert!(r.ln.node().blobs().has(&p.thumb.hash).await.unwrap());
    // A second poster does not replace the first.
    assert!(!r.ln.set_video_poster(&id, poster(2)).await.unwrap());
    assert_eq!(
        r.ln.store().photo(&id).await.unwrap().unwrap().thumb,
        p.thumb
    );
    r.ln.node().shutdown().await.unwrap();
}

#[tokio::test]
async fn import_by_reference_does_not_copy_the_original() {
    let r = rig().await;
    let folder = r.dir.path().join("big");
    std::fs::create_dir_all(&folder).unwrap();
    let samples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/samples");
    std::fs::copy(samples.join("clip.mov"), folder.join("v.mov")).unwrap();
    let opts = ImportOptions {
        mode: ImportMode::Reference,
        ..Default::default()
    };
    let report = r.ln.import_paths(&[folder], opts).await.unwrap();
    assert_eq!(report.imported.len(), 1);
    let rec =
        r.ln.store()
            .photo(&report.imported[0])
            .await
            .unwrap()
            .unwrap();
    assert_eq!(
        r.ln.node().blobs().size(&rec.original.hash).await.unwrap(),
        Some(rec.original.size)
    );
    r.ln.node().shutdown().await.unwrap();
}

#[tokio::test]
async fn progress_events_report_counts() {
    let r = rig().await;
    let (folder, summary) = corpus(r.dir.path(), "c");
    let mut events = r.ln.node().events();
    let report =
        r.ln.import_paths(&[folder], ImportOptions::default())
            .await
            .unwrap();

    let mut last = None;
    while let Ok(res) = tokio::time::timeout(Duration::from_millis(300), events.recv()).await {
        let Ok(ev) = res else { continue }; // lagged: keep reading
        if let NodeEvent::ImportProgress {
            done,
            total,
            imported,
            duplicates,
            skipped,
            failed,
        } = ev
        {
            assert!(done <= total && imported + duplicates + skipped + failed <= done);
            last = Some((done, total, imported, duplicates, skipped, failed));
        }
    }
    let (done, total, imported, duplicates, ..) = last.expect("progress events were emitted");
    assert_eq!((done, total), (summary.photos, summary.photos));
    assert_eq!(
        (imported, duplicates),
        (report.imported.len(), report.duplicates.len())
    );
    r.ln.node().shutdown().await.unwrap();
}

#[tokio::test]
async fn reimporting_after_a_purge_brings_the_photo_back() {
    let r = rig().await;
    let samples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/samples");
    let file = samples.join("photo-gps.jpg");
    let id =
        r.ln.import_paths(std::slice::from_ref(&file), ImportOptions::default())
            .await
            .unwrap()
            .imported[0];

    r.clock.advance(1_000);
    r.ln.store().mark_deleted(&id).await.unwrap();
    let later = r.clock.now_ms() + RETENTION_MS as u64 + 1_000;
    r.clock.set(later);
    r.ln.store()
        .purge_expired(Timestamp(later as i64))
        .await
        .unwrap();
    assert!(r.ln.store().photo(&id).await.unwrap().is_none());

    r.clock.advance(1_000);
    let report =
        r.ln.import_paths(&[file], ImportOptions::default())
            .await
            .unwrap();
    assert_eq!(
        report.imported,
        vec![id],
        "a purged photo can be imported again"
    );
    assert!(!r.ln.store().photo(&id).await.unwrap().unwrap().is_deleted());
    r.ln.node().shutdown().await.unwrap();
}

#[tokio::test]
async fn a_file_dropped_into_a_watched_folder_appears_within_five_seconds() {
    let r = rig().await;
    let watched = r.dir.path().join("inbox");
    std::fs::create_dir_all(&watched).unwrap();
    r.ln.watch_folder(&watched).await.unwrap();
    assert_eq!(r.ln.watched_folders(), vec![watched.clone()]);

    let samples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/samples");
    let start = std::time::Instant::now();
    std::fs::copy(samples.join("photo-gps.jpg"), watched.join("dropped.jpg")).unwrap();
    shoebox_testkit_wait(
        || async { !all_photos(r.ln.store()).await.is_empty() },
        Duration::from_secs(5),
    )
    .await;
    assert!(start.elapsed() < Duration::from_secs(5));
    assert_eq!(all_photos(r.ln.store()).await.len(), 1);

    // A second, identical file is recognised as a duplicate rather than a new record.
    std::fs::copy(samples.join("photo-gps.jpg"), watched.join("copy.jpg")).unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(all_photos(r.ln.store()).await.len(), 1);

    r.ln.unwatch_folder(&watched).await.unwrap();
    assert!(r.ln.watched_folders().is_empty());
    std::fs::copy(samples.join("screenshot.png"), watched.join("later.png")).unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(
        all_photos(r.ln.store()).await.len(),
        1,
        "unwatched folders are ignored"
    );
    r.ln.node().shutdown().await.unwrap();
}

#[tokio::test]
async fn watched_folders_persist_and_catch_up_after_a_restart() {
    let r = rig().await;
    let watched = r.dir.path().join("inbox");
    std::fs::create_dir_all(&watched).unwrap();
    r.ln.watch_folder(&watched).await.unwrap();
    r.ln.node().shutdown().await.unwrap();
    let Rig { dir, clock, .. } = r;

    // A file arrives while the app is closed.
    let samples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/samples");
    std::fs::copy(
        samples.join("photo-gps.jpg"),
        watched.join("while-away.jpg"),
    )
    .unwrap();

    let r = open_in(dir, clock).await;
    assert_eq!(
        r.ln.watched_folders(),
        vec![watched.clone()],
        "list persisted in config.toml"
    );
    shoebox_testkit_wait(
        || async { !all_photos(r.ln.store()).await.is_empty() },
        Duration::from_secs(10),
    )
    .await;
    r.ln.node().shutdown().await.unwrap();
}

async fn shoebox_testkit_wait<F, Fut>(mut f: F, timeout: Duration)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = std::time::Instant::now() + timeout;
    while !f().await {
        assert!(
            std::time::Instant::now() < deadline,
            "condition not met within {timeout:?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
