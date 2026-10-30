//! S1-12: search by meaning on any device, offline, fused with keywords; similar photos.

mod common;

use common::*;
use shoebox_core::ids::{Blake3Hash, Timestamp};
use shoebox_core::merge::Lww;
use shoebox_core::model::{BlobRef, ExifSummary, PhotoId, PhotoRecord, Role, ShardKind};
use shoebox_core::store::LibraryStore;
use shoebox_node::{Always, FetcherConfig, IndexerConfig, LibraryNode, Protection, SearchOptions};
use shoebox_search::encoder::{TestEncoder, TextEncoder};
use shoebox_store::Library;
use shoebox_testkit::{wait_until, Cluster};
use std::sync::Arc;
use std::time::Duration;

const ENC: TestEncoder = TestEncoder { dims: 64 };

fn jpeg(rgb: [u8; 3], seed: u32) -> Vec<u8> {
    let mut img = image::RgbImage::from_pixel(32, 32, image::Rgb(rgb));
    img.put_pixel(0, 0, image::Rgb([(seed % 251) as u8, 0, 0])); // distinct bytes per photo
    let mut out = vec![];
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90)
        .encode_image(&img)
        .unwrap();
    out
}

async fn add(
    n: &LibraryNode<Library>,
    others: &[&LibraryNode<Library>],
    i: u32,
    rgb: [u8; 3],
    taken: i64,
) -> PhotoId {
    let thumb = jpeg(rgb, i);
    let th = n
        .node()
        .blobs()
        .add_bytes(thumb.clone(), Protection::Library)
        .await
        .unwrap();
    for o in others {
        o.node()
            .blobs()
            .add_bytes(thumb.clone(), Protection::Library)
            .await
            .unwrap();
    }
    let id = Blake3Hash::of_bytes(&[b"p".as_slice(), &i.to_le_bytes()].concat());
    let blob = |hash| BlobRef {
        hash,
        size: 1,
        mime: "image/jpeg".into(),
    };
    let h = shoebox_core::hlc::Hlc {
        physical_ms: 1,
        counter: 0,
        device: n.device(),
    };
    n.store()
        .upsert_photo(PhotoRecord {
            id,
            original: blob(id),
            preview: blob(Blake3Hash::of_bytes(&[i as u8, 1, 2])),
            thumb: blob(th),
            taken_at: Timestamp(taken),
            exif: ExifSummary::default(),
            imported_by: n.device(),
            favorite: Lww::new(false, h),
            deleted_at: Lww::new(None, h),
        })
        .await
        .unwrap();
    id
}

fn fetch() -> FetcherConfig {
    FetcherConfig {
        concurrency: 4,
        tick: Duration::from_millis(200),
        retry: Duration::from_millis(200),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn search_by_meaning_works_with_every_other_device_off() {
    let c = Cluster::with_roles(&[Role::Keeper, Role::Companion])
        .await
        .unwrap();
    let nodes = library_nodes_auto(&c).await;
    let year = |y: i32| {
        shoebox_search::calendar::Zone::UTC
            .start_of(shoebox_search::calendar::Date::new(y, 6, 1))
            .0
    };
    let mut red = vec![];
    let mut blue = vec![];
    for i in 0..12u32 {
        red.push(
            add(
                &nodes[0],
                &[],
                i,
                [220, 30, 30],
                year(2019 + (i % 2) as i32),
            )
            .await,
        );
        blue.push(add(&nodes[0], &[], 100 + i, [30, 30, 220], year(2019)).await);
    }
    nodes[0].start_indexer(
        IndexerConfig {
            tick: Duration::from_millis(200),
            shard_after: Duration::from_millis(200),
            ..Default::default()
        },
        Arc::new(ENC),
        Arc::new(Always),
    );
    nodes[1].start_fetcher(fetch());
    for n in &nodes {
        n.set_text_encoder(Arc::new(|| Ok(Arc::new(ENC) as Arc<dyn TextEncoder>)));
        n.start_search();
    }
    wait_until(Duration::from_secs(60), || async {
        let mut have = 0;
        for (_, s) in nodes[1]
            .live_shards_of(TestEncoder::MODEL, ShardKind::Vec8)
            .await
            .unwrap()
        {
            have += s.ids.len();
        }
        have == 24
    })
    .await
    .expect("the phone holds the keeper's shards");

    // The keeper goes away: the phone searches on its own.
    c.take_offline(0);
    let opts = SearchOptions::utc(Timestamp(year(2026)));
    let r = nodes[1].search("red", &opts, 0, 50).await.unwrap();
    assert!(r.hits.ranked);
    assert_eq!(r.hits.ids.len(), 12, "the red photos and no blue one");
    assert!(r.hits.ids.iter().all(|id| red.contains(id)));
    // Filters still apply: red photos from 2019 only.
    let r = nodes[1].search("red 2019", &opts, 0, 50).await.unwrap();
    assert_eq!(r.hits.ids.len(), 6);
    let r = nodes[1].search("blue", &opts, 0, 50).await.unwrap();
    assert!(r.hits.ids.iter().all(|id| blue.contains(id)));
    // Similar photos: the other red ones, not itself.
    let sim = nodes[1].similar(&red[0], 0, 50).await.unwrap();
    assert!(!sim.ids.contains(&red[0]));
    assert_eq!(sim.ids.len(), 11);
    assert!(sim.ids.iter().all(|id| red.contains(id)));
    // A deleted photo is not found.
    nodes[1].store().mark_deleted(&red[1]).await.unwrap();
    wait_until(Duration::from_secs(10), || async {
        !nodes[1]
            .search("red", &opts, 0, 50)
            .await
            .unwrap()
            .hits
            .ids
            .contains(&red[1])
    })
    .await
    .expect("a deleted photo drops out");
    c.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_a_text_encoder_search_falls_back_to_keywords() {
    let c = Cluster::with_roles(&[Role::Keeper]).await.unwrap();
    let nodes = library_nodes_auto(&c).await;
    add(&nodes[0], &[], 1, [220, 30, 30], 0).await;
    let opts = SearchOptions::utc(Timestamp(0));
    let r = nodes[0].search("red", &opts, 0, 10).await.unwrap();
    assert!(r.hits.ids.is_empty());
    // A loader that fails turns meaning off quietly.
    nodes[0].set_text_encoder(Arc::new(|| anyhow::bail!("no model here")));
    let r = nodes[0].search("red", &opts, 0, 10).await.unwrap();
    assert!(r.hits.ids.is_empty());
    assert!(!nodes[0].meaning_available().await);
    c.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn labels_make_content_searchable_as_words() {
    use shoebox_search::labels::{encode_vectors, Labels, PROMPT};
    use shoebox_search::shard::Vec8;
    let c = Cluster::with_roles(&[Role::Keeper]).await.unwrap();
    let nodes = library_nodes_auto(&c).await;
    let list = "red: crimson, scarlet\nblue: navy\ngreen\n";
    let vectors: Vec<Vec8> = ["red", "blue", "green"]
        .iter()
        .map(|w| Vec8::quantize(&ENC.encode_text(&PROMPT.replace("{}", w)).unwrap()))
        .collect();
    let labels = Labels::new(
        list,
        &encode_vectors(
            TestEncoder::MODEL,
            shoebox_search::labels::LABELS_VERSION,
            &vectors,
            64,
        ),
    );
    let red = add(&nodes[0], &[], 1, [230, 20, 20], 0).await;
    let blue = add(&nodes[0], &[], 2, [20, 20, 230], 0).await;
    nodes[0].set_labels(Arc::new(labels));
    nodes[0].start_search();
    nodes[0].start_indexer(
        IndexerConfig {
            tick: Duration::from_millis(200),
            shard_after: Duration::from_millis(200),
            ..Default::default()
        },
        Arc::new(ENC),
        Arc::new(Always),
    );
    // No text model on this device: the words come from the labels alone.
    let opts = SearchOptions::utc(Timestamp(0));
    wait_until(Duration::from_secs(30), || async {
        nodes[0]
            .search("crimson", &opts, 0, 10)
            .await
            .unwrap()
            .hits
            .ids
            == vec![red]
    })
    .await
    .expect("a synonym finds the red photo by its label");
    assert_eq!(
        nodes[0]
            .search("navy", &opts, 0, 10)
            .await
            .unwrap()
            .hits
            .ids,
        vec![blue]
    );
    let s = nodes[0].search_suggestions("re", 5).await.unwrap();
    assert!(
        s.iter()
            .any(|(k, n)| *k == shoebox_search::NameKind::Label && n == "red"),
        "{s:?}"
    );
    c.shutdown().await;
}

/// A text reader that counts what it reads.
struct Counting(std::sync::atomic::AtomicUsize);

impl shoebox_search::ocr::Ocr for Counting {
    fn read_text(&self, bytes: &[u8]) -> Result<String, shoebox_search::encoder::EncodeError> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        shoebox_search::ocr::TestOcr.read_text(bytes)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn text_in_photos_is_found_on_every_device() {
    use shoebox_core::model::ExifSummary;
    // Every log line at every level, to check no read text ever reaches a log (diagnostics).
    let logs = Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let sink = logs.clone();
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(move || Sink(sink.clone()))
        .try_init();
    let c = Cluster::with_roles(&[Role::Keeper, Role::Companion])
        .await
        .unwrap();
    let nodes = library_nodes_auto(&c).await;
    let mut receipts = vec![];
    for i in 0..20u32 {
        let screenshot = i < 10;
        let thumb = jpeg([120, 120, 120], i);
        let th = nodes[0]
            .node()
            .blobs()
            .add_bytes(thumb, Protection::Library)
            .await
            .unwrap();
        let preview = format!(
            "{}TEXT: Corner Shop receipt {i}\nGRAND TOTAL 12.50",
            if screenshot { "PNG" } else { "JPEG" }
        );
        let pv = nodes[0]
            .node()
            .blobs()
            .add_bytes(preview.into_bytes(), Protection::Library)
            .await
            .unwrap();
        let id = Blake3Hash::of_bytes(&[b"r".as_slice(), &i.to_le_bytes()].concat());
        let blob = |hash, mime: &str| BlobRef {
            hash,
            size: 1,
            mime: mime.into(),
        };
        let h = shoebox_core::hlc::Hlc {
            physical_ms: 1,
            counter: 0,
            device: nodes[0].device(),
        };
        nodes[0]
            .store()
            .upsert_photo(PhotoRecord {
                id,
                original: blob(
                    id,
                    if screenshot {
                        "image/png"
                    } else {
                        "image/jpeg"
                    },
                ),
                preview: blob(pv, "image/jpeg"),
                thumb: blob(th, "image/jpeg"),
                taken_at: Timestamp(i64::from(i)),
                exif: if screenshot {
                    ExifSummary {
                        width: 1170,
                        height: 2532,
                        ..Default::default()
                    }
                } else {
                    ExifSummary {
                        camera_make: Some("Canon".into()),
                        camera_model: Some("Canon EOS R5".into()),
                        width: 4000,
                        height: 3000,
                        ..Default::default()
                    }
                },
                imported_by: nodes[0].device(),
                favorite: Lww::new(false, h),
                deleted_at: Lww::new(None, h),
            })
            .await
            .unwrap();
        if screenshot {
            receipts.push(id);
        }
    }
    let engine = Arc::new(Counting(Default::default()));
    nodes[0].set_ocr(engine.clone());
    nodes[0].start_indexer(
        IndexerConfig {
            tick: Duration::from_millis(200),
            shard_after: Duration::from_millis(200),
            ..Default::default()
        },
        Arc::new(ENC),
        Arc::new(Always),
    );
    nodes[1].start_fetcher(fetch());
    for n in &nodes {
        n.start_search();
    }
    let opts = SearchOptions::utc(Timestamp(0));
    for n in &nodes {
        wait_until(Duration::from_secs(60), || async {
            let mut got = n
                .search("grand total", &opts, 0, 50)
                .await
                .unwrap()
                .hits
                .ids;
            got.sort();
            let mut want = receipts.clone();
            want.sort();
            got == want
        })
        .await
        .expect("every receipt found by a word printed on it");
    }
    // Only the screenshots were read, each once.
    assert_eq!(engine.0.load(std::sync::atomic::Ordering::SeqCst), 10);
    let logged = String::from_utf8_lossy(&logs.lock().unwrap()).to_lowercase();
    assert!(!logged.is_empty(), "the log capture works");
    assert!(
        !logged.contains("grand total") && !logged.contains("corner shop"),
        "read text was logged"
    );
    c.shutdown().await;
}

struct Sink(Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for Sink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
