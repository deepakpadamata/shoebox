//! F2.10: Live Photos (a picture and its video, two files) and motion photos (a JPEG with its video
//! appended) import as one photo that has a video.

use shoebox_core::clock::ManualClock;
use shoebox_core::ids::Blake3Hash;
use shoebox_core::model::*;
use shoebox_core::store::*;
use shoebox_node::{ImportOptions, LibraryNode, Node, NodeConfig};
use shoebox_store_mem::{MemBackend, MemLibrary, MemNetwork};
use std::path::{Path, PathBuf};
use std::sync::Arc;

struct Rig {
    ln: LibraryNode<MemLibrary>,
    dir: tempfile::TempDir,
}

async fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let clock = ManualClock::new(1_700_000_000_000);
    let node = Node::open(NodeConfig::local_only(dir.path().join("node"), "n"))
        .await
        .unwrap();
    let device = shoebox_core::ids::EndpointId(*node.id().as_bytes());
    let backend = MemBackend::new(MemNetwork::new(), device, clock.clone());
    let (lib, _) = backend
        .create_library(&RootPublicKey([1; 32]))
        .await
        .unwrap();
    Rig {
        ln: LibraryNode::new(node, Arc::new(lib), clock),
        dir,
    }
}

fn samples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/samples")
}

/// `IMG_0001.JPG` and `IMG_0001.MOV` (2 s) in a new folder.
fn live_pair(r: &Rig, folder: &str, video_name: &str) -> (PathBuf, PathBuf, PathBuf) {
    let dir = r.dir.path().join(folder);
    std::fs::create_dir_all(&dir).unwrap();
    let still = dir.join("IMG_0001.JPG");
    let video = dir.join(video_name);
    std::fs::copy(samples().join("photo-gps.jpg"), &still).unwrap();
    std::fs::copy(samples().join("clip.mov"), &video).unwrap();
    (dir, still, video)
}

async fn timeline(r: &Rig) -> Vec<PhotoRecord> {
    r.ln.store()
        .photos_by_time(TimeRange::ALL, Page::all())
        .await
        .unwrap()
}

#[tokio::test]
async fn a_picture_and_its_video_are_one_photo_with_a_video() {
    let r = rig().await;
    let (dir, still, video) = live_pair(&r, "a", "IMG_0001.MOV");
    let report =
        r.ln.import_paths(std::slice::from_ref(&dir), ImportOptions::default())
            .await
            .unwrap();
    assert!(
        report.failed.is_empty() && report.skipped.is_empty(),
        "{report:?}"
    );
    assert_eq!(
        report.imported.len(),
        1,
        "the video is not a photo of its own"
    );
    let id = report.imported[0];
    assert_eq!(report.with_video, vec![id]);
    assert!(
        report.by_path.contains(&(still, id)) && report.by_path.contains(&(video.clone(), id)),
        "both files are accounted for: {:?}",
        report.by_path
    );

    let photos = timeline(&r).await;
    assert_eq!(photos.len(), 1);
    assert_eq!(photos[0].original.mime, "image/jpeg");

    let m =
        r.ln.store()
            .motion(&id)
            .await
            .unwrap()
            .expect("has a video");
    let clip = std::fs::read(&video).unwrap();
    assert_eq!(m.video.hash, Blake3Hash::of_bytes(&clip));
    assert_eq!(
        (m.video.mime.as_str(), m.video.size),
        ("video/quicktime", clip.len() as u64)
    );
    assert_eq!(m.source, MotionSource::Paired);
    assert!(m.duration_ms.abs_diff(2_000) <= 100, "{}", m.duration_ms);
    assert!(r.ln.node().blobs().kept(&m.video.hash).await.unwrap());
    assert!(
        r.ln.store().photo(&m.video.hash).await.unwrap().is_none(),
        "no timeline item for the video"
    );
    // This device says it holds the video, as a video blob (so keepers count it and peers can fetch it).
    let holders = r.ln.store().holders(&m.video.hash).await.unwrap();
    assert!(
        holders.iter().any(|h| h.kind == BlobKind::Motion),
        "{holders:?}"
    );

    // Again: nothing new.
    let again =
        r.ln.import_paths(std::slice::from_ref(&dir), ImportOptions::default())
            .await
            .unwrap();
    assert!(
        again.imported.is_empty() && again.with_video.is_empty(),
        "{again:?}"
    );
    assert_eq!(again.duplicates, vec![id]);
    assert_eq!(timeline(&r).await.len(), 1);
}

#[tokio::test]
async fn either_file_alone_brings_the_pair() {
    // A watched folder can hand over the video first, or only the video.
    for video_first in [true, false] {
        let r = rig().await;
        let (_, still, video) = live_pair(&r, "w", "img_0001.mp4");
        let (first, second) = if video_first {
            (&video, &still)
        } else {
            (&still, &video)
        };
        let one =
            r.ln.import_paths(std::slice::from_ref(first), ImportOptions::default())
                .await
                .unwrap();
        assert_eq!(
            (one.imported.len(), one.with_video.len()),
            (1, 1),
            "{one:?}"
        );
        let two =
            r.ln.import_paths(std::slice::from_ref(second), ImportOptions::default())
                .await
                .unwrap();
        assert!(
            two.imported.is_empty() && two.with_video.is_empty(),
            "{two:?}"
        );
        assert_eq!(timeline(&r).await.len(), 1);
        let id = one.imported[0];
        let m = r.ln.store().motion(&id).await.unwrap().unwrap();
        // Google's export calls the video `.mp4`; its type comes from what is inside.
        assert_eq!(m.video.mime, "video/quicktime");
    }
}

/// `clip.mov` with a header that says it runs for a minute.
fn long_clip() -> Vec<u8> {
    let mut bytes = std::fs::read(samples().join("clip.mov")).unwrap();
    let at = bytes.windows(4).position(|w| w == b"mvhd").unwrap() + 4;
    let timescale = u32::from_be_bytes(bytes[at + 12..at + 16].try_into().unwrap());
    bytes[at + 16..at + 20].copy_from_slice(&(timescale * 60).to_be_bytes());
    bytes
}

#[tokio::test]
async fn a_long_video_with_the_same_name_is_a_video_of_its_own() {
    let r = rig().await;
    let (dir, _, video) = live_pair(&r, "long", "IMG_0001.MOV");
    std::fs::write(&video, long_clip()).unwrap();
    let report =
        r.ln.import_paths(&[dir], ImportOptions::default())
            .await
            .unwrap();
    assert_eq!(report.imported.len(), 2, "{report:?}");
    assert!(report.with_video.is_empty());
    assert!(r.ln.store().motions().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_video_whose_picture_cannot_be_read_is_a_video() {
    let r = rig().await;
    let (dir, still, _) = live_pair(&r, "broken", "IMG_0001.MOV");
    let jpeg = std::fs::read(&still).unwrap();
    std::fs::write(&still, &jpeg[..jpeg.len() / 3]).unwrap();
    let report =
        r.ln.import_paths(&[dir], ImportOptions::default())
            .await
            .unwrap();
    assert_eq!(report.failed.len(), 1, "{report:?}");
    assert_eq!(report.imported.len(), 1, "the clip is imported on its own");
    let photos = timeline(&r).await;
    assert_eq!(photos[0].original.mime, "video/quicktime");
}

#[tokio::test]
async fn a_motion_photo_keeps_its_video_in_its_original() {
    let r = rig().await;
    let dir = r.dir.path().join("m");
    std::fs::create_dir_all(&dir).unwrap();
    let clip = std::fs::read(samples().join("clip.mp4")).unwrap();
    let mut file = std::fs::read(samples().join("photo-gps.jpg")).unwrap();
    file.extend(&clip);
    std::fs::write(dir.join("PXL_20240601_120000000.MP.jpg"), &file).unwrap();

    let report =
        r.ln.import_paths(std::slice::from_ref(&dir), ImportOptions::default())
            .await
            .unwrap();
    assert_eq!(
        (report.imported.len(), report.with_video.len()),
        (1, 1),
        "{report:?}"
    );
    let id = report.imported[0];
    assert_eq!(
        id,
        Blake3Hash::of_bytes(&file),
        "the original is the whole file"
    );
    let m = r.ln.store().motion(&id).await.unwrap().unwrap();
    assert_eq!(m.source, MotionSource::Embedded);
    assert_eq!(m.video.hash, Blake3Hash::of_bytes(&clip));
    assert_eq!(
        (m.video.mime.as_str(), m.video.size),
        ("video/mp4", clip.len() as u64)
    );
    // S2-06: the clip is in the original, not a blob of its own; it still plays from there.
    assert!(!r.ln.node().blobs().has(&m.video.hash).await.unwrap());
    assert!(r.ln.clip_available(&id).await.unwrap());
    let bytes = r.ln.clip_bytes(&id).await.unwrap().unwrap();
    assert_eq!(&bytes[..], &clip[..]);
    let part = r.ln.read_clip(&id, 100..2100).await.unwrap().unwrap();
    assert_eq!(&part[..], &clip[100..2100]);
    // Past its end the read stops at the clip's end, not the file's.
    let tail =
        r.ln.read_clip(&id, clip.len() as u64 - 10..u64::MAX)
            .await
            .unwrap()
            .unwrap();
    assert_eq!(&tail[..], &clip[clip.len() - 10..]);

    let again =
        r.ln.import_paths(&[dir], ImportOptions::default())
            .await
            .unwrap();
    assert!(again.with_video.is_empty(), "{again:?}");
}

#[tokio::test]
async fn the_video_lives_and_goes_with_its_photo() {
    let r = rig().await;
    let (dir, _, _) = live_pair(&r, "life", "IMG_0001.MOV");
    let id =
        r.ln.import_paths(&[dir], ImportOptions::default())
            .await
            .unwrap()
            .imported[0];
    let video = r.ln.store().motion(&id).await.unwrap().unwrap().video.hash;

    // Housekeeping keeps it while the photo is there, in Recently Deleted too.
    r.ln.reconcile_blobs().await.unwrap();
    assert!(r.ln.node().blobs().kept(&video).await.unwrap());
    r.ln.delete_photos(&[id]).await.unwrap();
    r.ln.reconcile_blobs().await.unwrap();
    assert!(r.ln.node().blobs().kept(&video).await.unwrap());

    // Purged, it is released with the rest.
    r.ln.purge_deleted(&[id]).await.unwrap();
    assert!(!r.ln.node().blobs().kept(&video).await.unwrap());
}

/// A generated motion photo (the sample still with the sample clip appended) in `dir`.
fn motion_photo(dir: &Path, name: &str, still: &str) -> (PathBuf, Vec<u8>, Vec<u8>) {
    std::fs::create_dir_all(dir).unwrap();
    let clip = std::fs::read(samples().join("clip.mp4")).unwrap();
    let mut file = std::fs::read(samples().join(still)).unwrap();
    file.extend(&clip);
    let path = dir.join(name);
    std::fs::write(&path, &file).unwrap();
    (path, file, clip)
}

#[tokio::test]
async fn a_referenced_motion_photo_plays_from_the_file_where_it_is() {
    let r = rig().await;
    let dir = r.dir.path().join("gallery");
    let (path, file, clip) = motion_photo(&dir, "PXL_1.MP.jpg", "photo-gps.jpg");
    let opts = ImportOptions {
        mode: shoebox_node::ImportMode::Reference,
        ..ImportOptions::default()
    };
    let report = r.ln.import_paths(&[dir], opts).await.unwrap();
    let id = report.imported[0];
    assert_eq!(report.with_video, vec![id]);
    let blobs = r.ln.node().blobs();
    let m = r.ln.store().motion(&id).await.unwrap().unwrap();
    assert!(!blobs.has(&m.video.hash).await.unwrap(), "no clip blob");
    assert!(
        r.ln.is_in_place(&id).await.unwrap(),
        "the original is the gallery file"
    );
    assert_eq!(&r.ln.clip_bytes(&id).await.unwrap().unwrap()[..], &clip[..]);
    // The file going away: the clip cannot be read (an error for the caller to sort out, as for
    // the original itself), and nothing was kept elsewhere.
    std::fs::remove_file(&path).unwrap();
    assert!(r.ln.read_clip(&id, 0..10).await.is_err());
    let _ = file;
}

#[tokio::test]
async fn a_paired_live_photo_video_is_referenced_in_place_too() {
    let r = rig().await;
    let (dir, _, _video) = live_pair(&r, "paired", "IMG_0001.MOV");
    let opts = ImportOptions {
        mode: shoebox_node::ImportMode::Reference,
        ..ImportOptions::default()
    };
    let report = r.ln.import_paths(&[dir], opts).await.unwrap();
    let id = report.imported[0];
    let m = r.ln.store().motion(&id).await.unwrap().unwrap();
    assert_eq!(m.source, MotionSource::Paired);
    let blobs = r.ln.node().blobs();
    assert!(
        r.ln.is_in_place(&m.video.hash).await.unwrap(),
        "the .MOV is read where it is"
    );
    assert!(blobs.has(&m.video.hash).await.unwrap());
}

#[tokio::test]
async fn stored_clips_are_released_once_the_original_is_held() {
    let r = rig().await;
    let dir = r.dir.path().join("old");
    let (_, _, clip) = motion_photo(&dir, "PXL_2.MP.jpg", "photo-gps.jpg");
    let report =
        r.ln.import_paths(&[dir], ImportOptions::default())
            .await
            .unwrap();
    let id = report.imported[0];
    let m = r.ln.store().motion(&id).await.unwrap().unwrap();
    let blobs = r.ln.node().blobs();
    // What an older build left behind: the clip as a blob of its own.
    blobs
        .add_bytes(clip.clone(), shoebox_node::Protection::Library)
        .await
        .unwrap();
    assert!(blobs.kept(&m.video.hash).await.unwrap());

    let released = r.ln.release_embedded_clips().await.unwrap();
    assert_eq!(released, vec![m.video.hash]);
    assert!(!blobs.kept(&m.video.hash).await.unwrap());
    assert!(blobs.kept(&id).await.unwrap(), "the original stays");
    // It still plays, from the original.
    assert_eq!(&r.ln.clip_bytes(&id).await.unwrap().unwrap()[..], &clip[..]);
    // Nothing left to release.
    assert!(r.ln.release_embedded_clips().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_clip_that_is_a_photo_of_its_own_is_not_released() {
    let r = rig().await;
    let dir = r.dir.path().join("both");
    let (_, _, clip) = motion_photo(&dir, "PXL_3.MP.jpg", "photo-gps.jpg");
    std::fs::write(dir.join("alone.mp4"), &clip).unwrap();
    let report =
        r.ln.import_paths(&[dir], ImportOptions::default())
            .await
            .unwrap();
    assert_eq!(report.imported.len(), 2, "{report:?}");
    let clip_hash = Blake3Hash::of_bytes(&clip);
    assert!(r.ln.release_embedded_clips().await.unwrap().is_empty());
    assert!(r.ln.node().blobs().kept(&clip_hash).await.unwrap());
}

/// A Samsung file of a newer camera: the picture, a short silent clip, Samsung's tag, then the
/// video the camera shows (here: the sample .mov, which differs in length from the .mp4).
fn samsung_photo(dir: &Path, name: &str) -> (PathBuf, Vec<u8>, Vec<u8>) {
    std::fs::create_dir_all(dir).unwrap();
    let first = std::fs::read(samples().join("clip.mp4")).unwrap();
    let real = std::fs::read(samples().join("clip.mov")).unwrap();
    let mut file = std::fs::read(samples().join("photo-gps.jpg")).unwrap();
    file.extend(&first);
    file.extend(b"MotionPhoto_Data");
    file.extend(&real);
    let path = dir.join(name);
    std::fs::write(&path, &file).unwrap();
    (path, first, real)
}

#[tokio::test]
async fn a_samsung_photo_plays_the_video_after_its_tag() {
    let r = rig().await;
    let dir = r.dir.path().join("samsung");
    let (_, first, real) = samsung_photo(&dir, "20261031_221440.jpg");
    assert_ne!(first.len(), real.len(), "the two clips differ in length");
    let report =
        r.ln.import_paths(&[dir], ImportOptions::default())
            .await
            .unwrap();
    let id = report.imported[0];
    let m = r.ln.store().motion(&id).await.unwrap().unwrap();
    assert_eq!(m.video.size as usize, real.len());
    assert_eq!(&r.ln.clip_bytes(&id).await.unwrap().unwrap()[..], &real[..]);
}

#[tokio::test]
async fn a_record_that_names_the_clip_ahead_of_the_tag_is_corrected() {
    let r = rig().await;
    let dir = r.dir.path().join("samsung-old");
    let (_, first, real) = samsung_photo(&dir, "20261031_221441.jpg");
    let report =
        r.ln.import_paths(&[dir], ImportOptions::default())
            .await
            .unwrap();
    let id = report.imported[0];
    // What an older build wrote: the record names the first clip.
    r.ln.store()
        .put_motion(MotionRecord {
            photo: id,
            video: BlobRef {
                hash: Blake3Hash::of_bytes(&first),
                size: first.len() as u64,
                mime: "video/mp4".into(),
            },
            duration_ms: 600,
            source: MotionSource::Embedded,
            made_by: r.ln.device(),
            hlc: shoebox_core::hlc::Hlc::GENESIS,
        })
        .await
        .unwrap();
    assert!(
        r.ln.clip_source(&id).await.unwrap().is_none(),
        "the old record's clip is not found any more"
    );

    assert_eq!(r.ln.repair_motion_clips().await.unwrap(), 1);
    let m = r.ln.store().motion(&id).await.unwrap().unwrap();
    assert_eq!(m.video.hash, Blake3Hash::of_bytes(&real));
    assert_eq!(&r.ln.clip_bytes(&id).await.unwrap().unwrap()[..], &real[..]);
    // Looked at once; the marker says the whole library has been.
    assert_eq!(r.ln.repair_motion_clips().await.unwrap(), 0);
}
