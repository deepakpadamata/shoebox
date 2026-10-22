//! `store-bench`: measures a `LibraryStore` backend at library scale (M1-15, decides #7).
//!
//! One backend per process so peak memory is attributable. Both replicas live in this process
//! (so "peak RSS" covers two replicas). Prints one JSON line on stdout and a summary on stderr.
//!
//!     cargo run --release -p store-bench -- --backend docs --photos 100000
//!
//! Measured: populate time, initial sync (empty replica catches up), incremental sync (N edits),
//! bytes needed to persist a replica, peak RSS, and query latencies (first page of the timeline,
//! a page from the middle, and `at_risk`).

use anyhow::Result;
use clap::Parser;
use serde_json::json;
use shoebox_core::clock::ManualClock;
use shoebox_core::hlc::Hlc;
use shoebox_core::ids::{Blake3Hash, EndpointId, Timestamp};
use shoebox_core::merge::Lww;
use shoebox_core::model::*;
use shoebox_core::store::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Parser)]
struct Args {
    /// docs | loro | mem
    #[arg(long, default_value = "docs")]
    backend: String,
    #[arg(long, default_value_t = 100_000)]
    photos: usize,
    /// Which blobs devices publish holdings for: none | originals | all (thumb+preview+original).
    #[arg(long, default_value = "originals")]
    holdings: String,
    /// Edits made between the initial and the incremental sync.
    #[arg(long, default_value_t = 100)]
    edits: usize,
}

fn dev(n: u8) -> EndpointId {
    EndpointId([n; 32])
}

fn id_of(i: usize, salt: &[u8]) -> Blake3Hash {
    let mut bytes = (i as u64).to_le_bytes().to_vec();
    bytes.extend_from_slice(salt);
    Blake3Hash::of_bytes(&bytes)
}

/// Capture time of photo `i`: a deterministic shuffle so insertion order is not time order.
fn taken_at(i: usize, n: usize) -> i64 {
    1_400_000_000_000 + ((i * 7919) % n) as i64 * 37_000
}

fn photo(i: usize, n: usize) -> PhotoRecord {
    let blob = |h: Blake3Hash, mime: &str| BlobRef {
        hash: h,
        size: 4_000_000,
        mime: mime.into(),
    };
    let original = id_of(i, b"");
    PhotoRecord {
        id: original,
        original: blob(original, "image/jpeg"),
        preview: blob(id_of(i, b"p"), "image/jpeg"),
        thumb: blob(id_of(i, b"t"), "image/jpeg"),
        taken_at: Timestamp(taken_at(i, n)),
        exif: ExifSummary {
            camera_make: Some("Apple".into()),
            camera_model: Some("iPhone 15 Pro".into()),
            lens: Some("iPhone 15 Pro back triple camera 6.8mm f/1.78".into()),
            width: 4032,
            height: 3024,
            gps: Some(Gps {
                lat_e7: 515_074_000,
                lon_e7: -1_278_000,
            }),
        },
        imported_by: dev(1),
        favorite: Lww::new(false, Hlc::GENESIS),
        deleted_at: Lww::new(None, Hlc::GENESIS),
    }
}

fn dir_size(path: &Path) -> u64 {
    std::fs::read_dir(path)
        .map(|rd| {
            rd.flatten()
                .map(|e| {
                    let m = e.metadata().ok();
                    match m {
                        Some(m) if m.is_dir() => dir_size(&e.path()),
                        Some(m) => m.len(),
                        None => 0,
                    }
                })
                .sum()
        })
        .unwrap_or(0)
}

/// Samples this process's resident set size (via `ps`) and remembers the peak, in KiB.
struct PeakRss {
    peak: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl PeakRss {
    fn start() -> Self {
        let peak = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (p, s) = (peak.clone(), stop.clone());
        let pid = std::process::id().to_string();
        let handle = std::thread::spawn(move || {
            while !s.load(Ordering::Relaxed) {
                if let Ok(out) = std::process::Command::new("ps")
                    .args(["-o", "rss=", "-p", &pid])
                    .output()
                {
                    if let Ok(kb) = String::from_utf8_lossy(&out.stdout).trim().parse::<u64>() {
                        p.fetch_max(kb, Ordering::Relaxed);
                    }
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        });
        Self {
            peak,
            stop,
            handle: Some(handle),
        }
    }

    fn stop(mut self) -> u64 {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        self.peak.load(Ordering::Relaxed)
    }
}

type DiskProbe<S> = Box<dyn Fn(&S, &S) -> (u64, u64)>;

struct Pair<S> {
    a: S,
    b: S,
    /// How `b` addresses `a`.
    a_addr: PeerAddr,
    /// The device identity `a` writes holdings as (a backend's own device id).
    a_device: EndpointId,
    /// Bytes needed to persist each replica.
    disk: DiskProbe<S>,
}

async fn measure<S: LibraryStore>(
    args: &Args,
    pair: Pair<S>,
    rss: PeakRss,
) -> Result<serde_json::Value> {
    let n = args.photos;
    let Pair {
        a,
        b,
        a_addr,
        a_device,
        disk,
    } = pair;

    let t = Instant::now();
    for i in 0..n {
        a.upsert_photo(photo(i, n)).await?;
        if i % 10_000 == 9_999 {
            eprintln!("  populated {} in {:.1?}", i + 1, t.elapsed());
        }
    }
    let populate = t.elapsed();

    let t = Instant::now();
    if args.holdings != "none" {
        let kinds_all = args.holdings == "all";
        let indices: Vec<usize> = (0..n).collect();
        for (round, chunk) in indices.chunks(5_000).enumerate() {
            let hlc_ms = 11 + round as u64;
            let mut added = vec![];
            for &i in chunk {
                added.push((id_of(i, b""), BlobKind::Original));
                if kinds_all {
                    added.push((id_of(i, b"p"), BlobKind::Preview));
                    added.push((id_of(i, b"t"), BlobKind::Thumb));
                }
            }
            let delta = HoldingsDelta {
                hlc: Hlc {
                    physical_ms: hlc_ms,
                    counter: 0,
                    device: a_device,
                },
                added,
                removed: vec![],
            };
            a.put_holdings(a_device, delta).await?;
        }
    }
    let holdings_time = t.elapsed();

    let t = Instant::now();
    b.sync_with(&a_addr).await?;
    let initial_sync = t.elapsed();
    let have = b.photos_by_time(TimeRange::ALL, Page::all()).await?.len();
    anyhow::ensure!(
        have == n,
        "replica B has {have} of {n} photos after the initial sync"
    );

    // Incremental: favorite `edits` photos on A, then catch B up.
    for k in 0..args.edits {
        a.set_favorite(&id_of((k * 997) % n, b""), true).await?;
    }
    let t = Instant::now();
    b.sync_with(&a_addr).await?;
    let incremental = t.elapsed();
    let edited = b
        .photo(&id_of(0, b""))
        .await?
        .is_some_and(|p| p.favorite.value);
    anyhow::ensure!(edited, "incremental sync did not deliver the edits");

    // Queries on the synced replica.
    let page1 = {
        let mut samples = vec![];
        for _ in 0..20 {
            let t = Instant::now();
            let p = b.photos_by_time(TimeRange::ALL, Page::first(100)).await?;
            samples.push(t.elapsed().as_secs_f64() * 1e6);
            anyhow::ensure!(p.len() == 100);
        }
        samples.iter().sum::<f64>() / samples.len() as f64
    };
    let mid_cursor = {
        let rec = b
            .photos_by_time(TimeRange::ALL, Page::first(n / 2))
            .await?
            .pop()
            .expect("median record");
        Cursor::after_photo(&rec)
    };
    let mid = {
        let mut samples = vec![];
        for _ in 0..20 {
            let t = Instant::now();
            let p = b
                .photos_by_time(
                    TimeRange::ALL,
                    Page {
                        after: Some(mid_cursor),
                        limit: 100,
                    },
                )
                .await?;
            samples.push(t.elapsed().as_secs_f64() * 1e6);
            anyhow::ensure!(p.len() == 100);
        }
        samples.iter().sum::<f64>() / samples.len() as f64
    };
    let t = Instant::now();
    let risky = b.at_risk(2, Page::all()).await?;
    let at_risk_all = t.elapsed();
    let at_risk_page = {
        let mut samples = vec![];
        for _ in 0..20 {
            let t = Instant::now();
            let _ = b.at_risk(2, Page::first(100)).await?;
            samples.push(t.elapsed().as_secs_f64() * 1e6);
        }
        samples.iter().sum::<f64>() / samples.len() as f64
    };

    let (disk_a, disk_b) = disk(&a, &b);
    let peak_kib = rss.stop();
    Ok(json!({
        "backend": args.backend,
        "photos": n,
        "holdings": args.holdings,
        "populate_s": populate.as_secs_f64(),
        "holdings_publish_s": holdings_time.as_secs_f64(),
        "initial_sync_s": initial_sync.as_secs_f64(),
        "incremental_sync_ms": incremental.as_secs_f64() * 1000.0,
        "edits": args.edits,
        "disk_mb_a": disk_a as f64 / 1e6,
        "disk_mb_b": disk_b as f64 / 1e6,
        "peak_rss_mb": peak_kib as f64 / 1024.0,
        "photos_page1_us": page1,
        "photos_midpage_us": mid,
        "at_risk_all_ms": at_risk_all.as_secs_f64() * 1000.0,
        "at_risk_all_count": risky.len(),
        "at_risk_page_us": at_risk_page,
    }))
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let rss = PeakRss::start();
    let result = match args.backend.as_str() {
        "mem" => {
            let net = shoebox_store_mem::MemNetwork::new();
            let ba =
                shoebox_store_mem::MemBackend::new(net.clone(), dev(1), ManualClock::new(1_000));
            let bb = shoebox_store_mem::MemBackend::new(net, dev(2), ManualClock::new(1_000));
            let (a, cap) = ba.create_library(&RootPublicKey([1; 32])).await?;
            let b = bb.open_library(&cap).await?;
            let pair = Pair {
                a,
                b,
                a_addr: PeerAddr::id_only(dev(1)),
                a_device: dev(1),
                disk: Box::new(|_, _| (0, 0)),
            };
            measure(&args, pair, rss).await?
        }
        "loro" => {
            let net = shoebox_store_loro::LoroNetwork::new();
            let ba =
                shoebox_store_loro::LoroBackend::new(net.clone(), dev(1), ManualClock::new(1_000));
            let bb = shoebox_store_loro::LoroBackend::new(net, dev(2), ManualClock::new(1_000));
            let (a, cap) = ba.create_library(&RootPublicKey([1; 32])).await?;
            let b = bb.open_library(&cap).await?;
            // Persisting a Loro replica = one snapshot blob.
            let pair = Pair {
                a,
                b,
                a_addr: PeerAddr::id_only(dev(1)),
                a_device: dev(1),
                disk: Box::new(|a, b| {
                    (
                        a.export_snapshot().len() as u64,
                        b.export_snapshot().len() as u64,
                    )
                }),
            };
            measure(&args, pair, rss).await?
        }
        "docs" => {
            let dir = tempfile::tempdir()?;
            let mk = |name: &str| {
                let mut cfg = shoebox_node::NodeConfig::local_only(dir.path().join(name), name);
                cfg.gc_interval_ms = None; // GC would only add noise to the measurement
                cfg
            };
            let (na, nb) = (
                shoebox_node::Node::open(mk("a")).await?,
                shoebox_node::Node::open(mk("b")).await?,
            );
            na.add_peer_addr(nb.addr());
            nb.add_peer_addr(na.addr());
            let ba = shoebox_store_docs::DocsBackend::for_node(na.clone(), ManualClock::new(1_000));
            let bb = shoebox_store_docs::DocsBackend::for_node(nb.clone(), ManualClock::new(1_000));
            let (a, cap) = ba.create_library(&RootPublicKey([1; 32])).await?;
            let b = bb.open_library(&cap).await?;
            let (pa, pb): (PathBuf, PathBuf) = (dir.path().join("a"), dir.path().join("b"));
            let pair = Pair {
                a,
                b,
                a_addr: shoebox_node::from_endpoint_addr(&na.addr()),
                a_device: ba.device(),
                disk: Box::new(move |_, _| (dir_size(&pa), dir_size(&pb))),
            };
            let out = measure(&args, pair, rss).await;
            na.shutdown().await.ok();
            nb.shutdown().await.ok();
            out?
        }
        other => anyhow::bail!("unknown backend `{other}` (docs | loro | mem)"),
    };
    println!("{result}");
    eprintln!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
