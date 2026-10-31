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
async fn a_motion_photo_keeps_its_video_as_a_blob() {
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
    assert!(r.ln.node().blobs().kept(&m.video.hash).await.unwrap());

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
