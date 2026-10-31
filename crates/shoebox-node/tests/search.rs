//! S1-03: search on the node. The facts index follows the library's change stream, is saved to
//! `search/facts.idx`, and is rebuilt when that file is missing or damaged.

use shoebox_core::clock::ManualClock;
use shoebox_core::hlc::Hlc;
use shoebox_core::ids::{Blake3Hash, EndpointId, Timestamp};
use shoebox_core::merge::Lww;
use shoebox_core::model::*;
use shoebox_core::store::*;
use shoebox_node::{LibraryNode, Node, NodeConfig, SearchOptions};
use shoebox_store_mem::{MemBackend, MemLibrary, MemNetwork};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

const DAY: i64 = 86_400_000;
/// 2019-06-15 and 2021-06-15, UTC.
const IN_2019: i64 = 18_062 * DAY;
const IN_2021: i64 = 18_793 * DAY;

async fn open(dir: &Path) -> LibraryNode<MemLibrary> {
    let clock = ManualClock::new(1_700_000_000_000);
    let node = Node::open(NodeConfig::local_only(dir.join("node"), "n"))
        .await
        .unwrap();
    let device = EndpointId(*node.id().as_bytes());
    let backend = MemBackend::new(MemNetwork::new(), device, clock.clone());
    let (lib, _) = backend
        .create_library(&RootPublicKey([1; 32]))
        .await
        .unwrap();
    LibraryNode::new(node, Arc::new(lib), clock)
}

fn record(name: &str, taken_ms: i64, camera: &str) -> PhotoRecord {
    let blob = |t: &str| BlobRef {
        hash: Blake3Hash::of_bytes(format!("{name}{t}").as_bytes()),
        size: 1,
        mime: "image/jpeg".into(),
    };
    PhotoRecord {
        id: Blake3Hash::of_bytes(name.as_bytes()),
        original: blob("o"),
        preview: blob("p"),
        thumb: blob("t"),
        taken_at: Timestamp(taken_ms),
        exif: ExifSummary {
            camera_model: Some(camera.into()),
            width: 4000,
            height: 3000,
            ..Default::default()
        },
        imported_by: EndpointId([9; 32]),
        favorite: Lww::new(false, Hlc::GENESIS),
        deleted_at: Lww::new(None, Hlc::GENESIS),
    }
}

fn opts() -> SearchOptions {
    SearchOptions::utc(Timestamp(1_800_000_000_000))
}

async fn ids(ln: &LibraryNode<MemLibrary>, q: &str) -> Vec<PhotoId> {
    ln.search(q, &opts(), 0, 100).await.unwrap().hits.ids
}

async fn eventually(ln: &LibraryNode<MemLibrary>, q: &str, want: usize) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    loop {
        let got = ids(ln, q).await.len();
        if got == want {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{q:?}: {got} results, want {want}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn finds_by_year_and_camera_and_follows_changes() {
    let dir = tempfile::tempdir().unwrap();
    let ln = open(dir.path()).await;
    ln.store()
        .upsert_photo(record("a", IN_2019, "Pixel 8"))
        .await
        .unwrap();
    ln.store()
        .upsert_photo(record("b", IN_2021, "Pixel 8"))
        .await
        .unwrap();
    ln.start_search();

    assert_eq!(ids(&ln, "2019").await, [Blake3Hash::of_bytes(b"a")]);
    assert_eq!(ids(&ln, "pixel 8").await.len(), 2);
    let r = ln.search("pixel 8 2021", &opts(), 0, 10).await.unwrap();
    assert_eq!(r.query.filters.len(), 2, "camera and year are both chips");

    // A record arriving later (from this device or by sync) is found within a second.
    ln.store()
        .upsert_photo(record("c", IN_2019 + DAY, "X-T5"))
        .await
        .unwrap();
    eventually(&ln, "2019", 2).await;
    ln.store()
        .set_favorite(&Blake3Hash::of_bytes(b"c"), true)
        .await
        .unwrap();
    eventually(&ln, "favourites", 1).await;
    ln.store()
        .mark_deleted(&Blake3Hash::of_bytes(b"a"))
        .await
        .unwrap();
    eventually(&ln, "2019", 1).await;

    // Albums are names.
    let album = AlbumMeta {
        id: AlbumId([3; 16]),
        name: Lww::new("Road trip".into(), Hlc::GENESIS),
        cover: Lww::new(None, Hlc::GENESIS),
        items: shoebox_core::merge::OrSet::new(),
    };
    ln.store().upsert_album(album).await.unwrap();
    ln.store()
        .album_add(&AlbumId([3; 16]), &[Blake3Hash::of_bytes(b"b")])
        .await
        .unwrap();
    eventually(&ln, "road trip", 1).await;
    let s = ln.search_suggestions("roa", 5).await.unwrap();
    assert_eq!(s.len(), 1);
}

#[tokio::test]
async fn a_missing_or_damaged_index_is_rebuilt() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("node/search/facts.idx");
    {
        let ln = open(dir.path()).await;
        ln.store()
            .upsert_photo(record("a", IN_2019, "Pixel 8"))
            .await
            .unwrap();
        assert_eq!(ids(&ln, "2019").await.len(), 1);
        assert!(file.exists(), "the first build is saved");
        ln.node().shutdown().await.unwrap();
    }
    // Truncated: detected and rebuilt from the library (here a fresh in-memory one with a
    // different photo, so a stale index would show the old photo).
    let bytes = std::fs::read(&file).unwrap();
    std::fs::write(&file, &bytes[..bytes.len() / 2]).unwrap();
    let ln = open(dir.path()).await;
    ln.store()
        .upsert_photo(record("z", IN_2021, "Pixel 8"))
        .await
        .unwrap();
    assert_eq!(ids(&ln, "2021").await, [Blake3Hash::of_bytes(b"z")]);
    assert!(
        ids(&ln, "2019").await.is_empty(),
        "photos gone from the library are dropped"
    );
    assert_ne!(std::fs::read(&file).unwrap(), bytes[..bytes.len() / 2]);

    // Deleted: rebuilt as well.
    std::fs::remove_dir_all(dir.path().join("node/search")).unwrap();
    ln.rebuild_search_index().await.unwrap();
    assert_eq!(ids(&ln, "2021").await.len(), 1);
    assert!(file.exists());
}

#[tokio::test]
async fn photos_with_gps_are_found_by_place() {
    let dir = tempfile::tempdir().unwrap();
    let ln = open(dir.path()).await;
    let mut rec = record("lisbon", IN_2019, "Pixel 8");
    rec.exif.gps = Some(Gps {
        lat_e7: 387_075_000,
        lon_e7: -91_364_000,
    });
    ln.store().upsert_photo(rec).await.unwrap();
    ln.store()
        .upsert_photo(record("nowhere", IN_2019, "Pixel 8"))
        .await
        .unwrap();
    for q in ["Lisbon", "portugal", "lisboa 2019", "Lisbon 2019"] {
        let r = ln.search(q, &opts(), 0, 10).await.unwrap();
        // "lisboa" is not a name in the list, so it is a keyword; it matches nothing.
        let want = usize::from(q != "lisboa 2019");
        assert_eq!(r.hits.ids.len(), want, "{q}");
    }
    let r = ln.search("Portugal", &opts(), 0, 10).await.unwrap();
    assert_eq!(r.query.filters.len(), 1, "a place in the library is a chip");
}

/// F2.10: "live photos" finds the photos that have a video, before and after the index is built.
#[tokio::test]
async fn live_photos_are_found_by_their_chip() {
    let dir = tempfile::tempdir().unwrap();
    let ln = open(dir.path()).await;
    let (a, b) = (Blake3Hash::of_bytes(b"a"), Blake3Hash::of_bytes(b"b"));
    for name in ["a", "b"] {
        ln.store()
            .upsert_photo(record(name, IN_2019, "Pixel 8"))
            .await
            .unwrap();
    }
    let motion = |photo| MotionRecord {
        photo,
        video: BlobRef {
            hash: Blake3Hash::of_bytes(b"clip"),
            size: 1,
            mime: "video/quicktime".into(),
        },
        duration_ms: 1_500,
        source: MotionSource::Paired,
        made_by: EndpointId([9; 32]),
        hlc: Hlc::GENESIS,
    };
    // Found when the index is first built from the library...
    ln.store().put_motion(motion(a)).await.unwrap();
    ln.start_search();
    assert_eq!(ids(&ln, "live photos").await, [a]);
    assert_eq!(
        ids(&ln, "photos").await.len(),
        2,
        "a Live Photo is a photo too"
    );

    // ...and when a video arrives later, from this device or by sync.
    ln.store().put_motion(motion(b)).await.unwrap();
    eventually(&ln, "live photos", 2).await;
}
