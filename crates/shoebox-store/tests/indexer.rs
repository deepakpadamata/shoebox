//! S1-11: indexing shared between devices without claims, with a 48-hour takeover.

mod common;

use common::*;
use shoebox_core::clock::Clock;
use shoebox_core::ids::{Blake3Hash, EndpointId, Timestamp};
use shoebox_core::merge::Lww;
use shoebox_core::model::{
    BlobKind, BlobRef, ExifSummary, IndexerStatus, PhotoId, PhotoRecord, Role, ShardKind,
};
use shoebox_core::policy::{index_owner, IndexCandidate};
use shoebox_core::store::LibraryStore;
use shoebox_node::{Always, IndexGate, IndexerConfig, LibraryNode, Protection};
use shoebox_search::encoder::TestEncoder;
use shoebox_store::Library;
use shoebox_testkit::{wait_until, Cluster};
use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

const ENC: TestEncoder = TestEncoder { dims: 64 };

fn cfg(weight: u32) -> IndexerConfig {
    IndexerConfig {
        tick: Duration::from_millis(200),
        batch: 32,
        shard_size: 64,
        shard_after: Duration::from_millis(300),
        threads: 2,
        always_on: true,
        rate_override: Some(weight),
        ..Default::default()
    }
}

fn fetch() -> shoebox_node::FetcherConfig {
    shoebox_node::FetcherConfig {
        concurrency: 4,
        tick: Duration::from_millis(200),
        retry: Duration::from_millis(200),
    }
}

fn jpeg(i: u32) -> Vec<u8> {
    let c = [
        (i * 37 % 256) as u8,
        (i * 91 % 256) as u8,
        (i * 53 % 256) as u8,
    ];
    let img = image::RgbImage::from_pixel(16, 16, image::Rgb(c));
    let mut out = vec![];
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 80)
        .encode_image(&img)
        .unwrap();
    out
}

/// `n` photos imported by `importer`, whose thumbnails every node in `holders` already has.
async fn photos(
    nodes: &[LibraryNode<Library>],
    holders: &[usize],
    importer: EndpointId,
    range: std::ops::Range<u32>,
) -> Vec<PhotoId> {
    let mut ids = vec![];
    for i in range {
        let thumb = jpeg(i);
        let mut th = Blake3Hash::of_bytes(&thumb);
        for h in holders {
            th = nodes[*h]
                .node()
                .blobs()
                .add_bytes(thumb.clone(), Protection::Library)
                .await
                .unwrap();
        }
        let id = Blake3Hash::of_bytes(&[b"photo".as_slice(), &i.to_le_bytes()].concat());
        let blob = |hash| BlobRef {
            hash,
            size: 10,
            mime: "image/jpeg".into(),
        };
        let h = shoebox_core::hlc::Hlc {
            physical_ms: 1,
            counter: 0,
            device: importer,
        };
        nodes[holders[0]]
            .store()
            .upsert_photo(PhotoRecord {
                id,
                original: blob(id),
                preview: blob(Blake3Hash::of_bytes(&[i as u8, 7])),
                thumb: blob(th),
                taken_at: Timestamp(i64::from(i)),
                exif: ExifSummary::default(),
                imported_by: importer,
                favorite: Lww::new(false, h),
                deleted_at: Lww::new(None, h),
            })
            .await
            .unwrap();
        ids.push(id);
    }
    ids
}

/// Photo → number of live vec8 shards holding it, as `n` sees them.
async fn coverage(n: &LibraryNode<Library>) -> BTreeMap<PhotoId, usize> {
    let mut out = BTreeMap::new();
    for (_, s) in n
        .live_shards_of(TestEncoder::MODEL, ShardKind::Vec8)
        .await
        .unwrap()
    {
        for id in &s.ids {
            *out.entry(*id).or_default() += 1;
        }
    }
    out
}

async fn announce(n: &LibraryNode<Library>, weight: u32) {
    n.store()
        .put_indexer_status(IndexerStatus {
            device: n.device(),
            models: vec![TestEncoder::MODEL],
            kinds: vec![ShardKind::Vec8, ShardKind::Vec1],
            photos_per_min: weight,
            always_on: true,
            hlc: shoebox_core::hlc::Hlc::GENESIS,
        })
        .await
        .unwrap();
}

async fn statuses_seen(n: &LibraryNode<Library>, count: usize) -> bool {
    n.store().indexer_statuses().await.unwrap().len() == count
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn three_indexers_share_the_backlog_by_speed() {
    let c = Cluster::with_roles(&[Role::Keeper, Role::Keeper, Role::Keeper])
        .await
        .unwrap();
    let nodes = library_nodes_auto(&c).await;
    let total = 700u32;
    // Imported by a phone that does not index: everything is backlog.
    let ids = photos(&nodes, &[0, 1, 2], EndpointId([77; 32]), 0..total).await;
    let weights = [10u32, 20, 40];
    for (n, w) in nodes.iter().zip(weights) {
        announce(n, w).await;
    }
    for n in &nodes {
        wait_until(Duration::from_secs(30), || async {
            statuses_seen(n, 3).await
                && n.store()
                    .photo(ids.last().unwrap())
                    .await
                    .unwrap()
                    .is_some()
        })
        .await
        .expect("statuses and photos reach every device");
    }
    for (n, w) in nodes.iter().zip(weights) {
        n.start_indexer(cfg(w), Arc::new(ENC), Arc::new(Always));
        n.start_fetcher(fetch());
    }
    wait_until(Duration::from_secs(120), || async {
        coverage(&nodes[0]).await.len() == total as usize
    })
    .await
    .expect("every photo indexed");
    let cov = coverage(&nodes[0]).await;
    let dups: usize = cov.values().map(|n| n - 1).sum();
    assert!(dups * 20 <= total as usize, "{dups} duplicates");
    // Shares follow the weights: count what each device made.
    let records = nodes[0].store().index_shards().await.unwrap();
    let made = |d: EndpointId| -> f64 {
        records
            .iter()
            .filter(|r| r.made_by == d && r.kind == ShardKind::Vec8 && r.replaces.is_empty())
            .map(|r| r.count as f64)
            .sum::<f64>()
            / f64::from(total)
    };
    for (n, want) in nodes.iter().zip([1.0 / 7.0, 2.0 / 7.0, 4.0 / 7.0]) {
        let got = made(n.device());
        assert!(
            (got - want).abs() < 0.1,
            "{}: {got:.2} vs {want:.2}",
            n.device()
        );
    }
    c.shutdown().await;
}

/// A gate the test opens and closes (a phone on and off the charger).
struct Charging(AtomicBool);

impl IndexGate for Charging {
    fn may_index(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stopped_indexer_and_a_phone_that_never_charges_are_covered_after_48_hours() {
    let c = Cluster::with_roles(&[Role::Keeper, Role::Keeper, Role::Companion])
        .await
        .unwrap();
    let nodes = library_nodes_auto(&c).await;
    // Keeper 1 announces itself but never indexes (it died).
    announce(&nodes[1], 1_000).await;
    let backlog = photos(&nodes, &[0, 1], EndpointId([77; 32]), 0..60).await;
    // The phone's own photos: thumbnails on the phone and the keeper.
    let own = photos(&nodes, &[2, 0], nodes[2].device(), 100..110).await;
    for n in &nodes {
        wait_until(Duration::from_secs(30), || async {
            statuses_seen(n, 1).await
                && n.store()
                    .photo(own.last().unwrap())
                    .await
                    .unwrap()
                    .is_some()
        })
        .await
        .unwrap();
    }
    let charging = Arc::new(Charging(AtomicBool::new(false)));
    nodes[0].start_indexer(cfg(10), Arc::new(ENC), Arc::new(Always));
    nodes[2].start_indexer(
        IndexerConfig {
            always_on: false,
            ..cfg(5)
        },
        Arc::new(ENC),
        charging.clone(),
    );
    // Keeper 0 does its own share only; the dead keeper's and the phone's wait.
    tokio::time::sleep(Duration::from_secs(3)).await;
    nodes[0].flush_index().await.unwrap();
    let cov = coverage(&nodes[0]).await;
    let dead_share: Vec<&PhotoId> = backlog.iter().filter(|p| !cov.contains_key(p)).collect();
    assert!(
        !dead_share.is_empty(),
        "the dead keeper's share is left at first"
    );
    // Except those the rendezvous gives keeper 0 itself: the phone imported them, but they are
    // also keeper 0's share (device ids differ every run, so that is about one photo in ten runs).
    let candidates = [(&nodes[0], 10), (&nodes[1], 1_000)].map(|(n, weight)| IndexCandidate {
        device: n.device(),
        weight,
    });
    let keeper0 = nodes[0].device();
    assert!(
        own.iter()
            .filter(|p| index_owner(p, ShardKind::Vec8, &candidates) != Some(keeper0))
            .all(|p| !cov.contains_key(p)),
        "the phone's own photos wait for it"
    );

    // 49 hours later (the phone never charged): the keeper covers everything.
    c.clock().set(c.clock().now_ms() + 49 * 3600 * 1000);
    wait_until(Duration::from_secs(60), || async {
        nodes[0].flush_index().await.unwrap();
        let cov = coverage(&nodes[0]).await;
        backlog.iter().chain(&own).all(|p| cov.contains_key(p))
    })
    .await
    .expect("the keeper takes over after the grace");
    c.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_phone_indexes_its_own_photos_while_charging() {
    let c = Cluster::with_roles(&[Role::Keeper, Role::Companion])
        .await
        .unwrap();
    let nodes = library_nodes_auto(&c).await;
    let own = photos(&nodes, &[1], nodes[1].device(), 0..20).await;
    let charging = Arc::new(Charging(AtomicBool::new(false)));
    nodes[1].start_indexer(
        IndexerConfig {
            always_on: false,
            ..cfg(5)
        },
        Arc::new(ENC),
        charging.clone(),
    );
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(coverage(&nodes[1]).await.is_empty(), "not while unplugged");
    assert!(nodes[1].index_progress().waiting.is_some());
    charging.0.store(true, Ordering::SeqCst);
    wait_until(Duration::from_secs(30), || async {
        let cov = coverage(&nodes[1]).await;
        own.iter().all(|p| cov.contains_key(p))
    })
    .await
    .expect("indexed on the phone once charging");
    // The shards reach the keeper, which holds them.
    nodes[0].start_fetcher(fetch());
    wait_until(Duration::from_secs(30), || async {
        coverage(&nodes[0]).await.len() == own.len()
    })
    .await
    .expect("the keeper gets the phone's shards");
    let held: HashSet<_> = nodes[0]
        .store()
        .index_shards()
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.blob.hash)
        .collect();
    for h in held {
        assert!(nodes[0]
            .store()
            .holders(&h)
            .await
            .unwrap()
            .iter()
            .any(|x| x.kind == BlobKind::Index));
    }
    c.shutdown().await;
}

async fn live_count(n: &LibraryNode<Library>, kind: ShardKind) -> usize {
    n.live_shards_of(TestEncoder::MODEL, kind)
        .await
        .unwrap()
        .len()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn small_shards_compact_and_every_device_lets_the_old_ones_go() {
    use shoebox_core::ids::Timestamp;
    use shoebox_node::SearchOptions;
    use shoebox_search::encoder::TextEncoder;
    let c = Cluster::with_roles(&[Role::Keeper, Role::Keeper, Role::Keeper])
        .await
        .unwrap();
    let nodes = library_nodes_auto(&c).await;
    let total = 120u32;
    photos(&nodes, &[0, 1, 2], EndpointId([77; 32]), 0..total).await;
    for (n, w) in nodes.iter().zip([10u32, 20, 40]) {
        announce(n, w).await;
    }
    for n in &nodes {
        wait_until(Duration::from_secs(30), || async {
            statuses_seen(n, 3).await
        })
        .await
        .unwrap();
    }
    for (n, w) in nodes.iter().zip([10u32, 20, 40]) {
        n.start_indexer(
            IndexerConfig {
                shard_size: 4,
                batch: 4,
                compact_every: Duration::from_secs(3600 * 24),
                ..cfg(w)
            },
            Arc::new(ENC),
            Arc::new(Always),
        );
        n.start_fetcher(fetch());
        n.set_text_encoder(Arc::new(|| Ok(Arc::new(ENC) as Arc<dyn TextEncoder>)));
        n.start_search();
    }
    for n in &nodes {
        wait_until(Duration::from_secs(120), || async {
            n.flush_index().await.unwrap();
            coverage(n).await.len() == total as usize
        })
        .await
        .expect("indexed and fetched everywhere");
    }
    let before = live_count(&nodes[0], ShardKind::Vec8).await;
    assert!(before >= 10, "{before} shards");
    let opts = SearchOptions::utc(Timestamp(0));
    let results = nodes[2]
        .search("red", &opts, 0, 500)
        .await
        .unwrap()
        .hits
        .ids;
    assert!(!results.is_empty());
    let old: Vec<_> = nodes[0]
        .shard_view()
        .await
        .unwrap()
        .live
        .into_iter()
        .collect();

    // Two devices compact the same shards at once: duplicates, still correct.
    let (a, b) = tokio::join!(nodes[0].compact_shards(true), nodes[1].compact_shards(true));
    assert!(a.unwrap().written > 0 && b.unwrap().written > 0);
    for n in &nodes {
        wait_until(Duration::from_secs(60), || async {
            n.reconcile_blobs().await.unwrap();
            coverage(n).await.len() == total as usize
        })
        .await
        .expect("every photo still covered while the merged shards spread");
    }
    // Later rounds merge the duplicates away.
    wait_until(Duration::from_secs(90), || async {
        nodes[2].compact_shards(true).await.unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;
        let mut ok = true;
        for n in &nodes {
            n.reconcile_blobs().await.unwrap();
            ok &= live_count(n, ShardKind::Vec8).await == 1
                && live_count(n, ShardKind::Vec1).await == 1;
        }
        ok
    })
    .await
    .expect("one shard per kind on every device");
    for n in &nodes {
        for h in &old {
            assert!(
                !n.node().blobs().kept(h).await.unwrap(),
                "old shards released"
            );
        }
        let cov = coverage(n).await;
        assert_eq!(cov.len(), total as usize);
        assert!(cov.values().all(|k| *k == 1), "no duplicate rows left");
    }
    let after = nodes[2]
        .search("red", &opts, 0, 500)
        .await
        .unwrap()
        .hits
        .ids;
    let (mut x, mut y) = (results.clone(), after.clone());
    x.sort();
    y.sort();
    assert_eq!(x, y, "the same results before and after");
    c.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pausing_hands_the_share_to_the_others_and_the_status_counts_right() {
    let c = Cluster::with_roles(&[Role::Keeper, Role::Keeper])
        .await
        .unwrap();
    let nodes = library_nodes_auto(&c).await;
    let total = 80u32;
    photos(&nodes, &[0, 1], EndpointId([77; 32]), 0..total).await;
    // Device 1 is set to index but paused from the start: it announces 0 photos a minute.
    nodes[1].pause_indexer();
    nodes[1].start_indexer(cfg(1_000), Arc::new(ENC), Arc::new(Always));
    wait_until(Duration::from_secs(30), || async {
        nodes[0]
            .store()
            .indexer_statuses()
            .await
            .unwrap()
            .iter()
            .any(|s| s.device == nodes[1].device() && s.photos_per_min == 0)
    })
    .await
    .expect("the paused device says it is not indexing");
    nodes[0].start_indexer(cfg(10), Arc::new(ENC), Arc::new(Always));
    wait_until(Duration::from_secs(60), || async {
        nodes[0].flush_index().await.unwrap();
        coverage(&nodes[0]).await.len() == total as usize
    })
    .await
    .expect("the other device does the whole backlog");
    let records = nodes[0].store().index_shards().await.unwrap();
    assert!(records.iter().all(|r| r.made_by == nodes[0].device()));
    let s = nodes[0].search_status().await.unwrap();
    assert_eq!((s.total, s.content), (total as usize, total as usize));
    assert_eq!(s.eta_secs, None);
    assert_eq!(s.indexers.len(), 1, "the paused one is not listed");
    assert!(s.held_bytes > 0);
    c.shutdown().await;
}
