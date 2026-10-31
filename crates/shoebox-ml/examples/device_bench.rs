//! Search on one device at 100k photos (S1-20): facts query and meaning scoring latency, the
//! encoders' speed, and memory. Built for the machine it runs on and copied there with the models
//! and a few photos.
//!
//! ```text
//! cargo run --release -p shoebox-ml --example device_bench -- <models dir> <photos dir> [scratch dir]
//! device_bench --idle <scratch dir>
//! ```
//!
//! The first run leaves a 100k-photo `bench-facts.idx` and `bench-vec.bin` in the scratch dir;
//! `--idle`, in a fresh process, loads them as the app does at start and says what they hold.

use anyhow::{Context, Result};
use image::imageops::FilterType;
use shoebox_core::hlc::Hlc;
use shoebox_core::ids::{Blake3Hash, EndpointId, Timestamp};
use shoebox_core::merge::Lww;
use shoebox_core::model::{BlobRef, ExifSummary, Gps, ModelId, PhotoRecord};
use shoebox_ml::{ClipImage, ClipText, SFace, YuNet};
use shoebox_search::calendar::{Date, Zone};
use shoebox_search::clip_tokenizer::ClipTokenizer;
use shoebox_search::encoder::{normalize, ImageEncoder, Rgb, TextEncoder};
use shoebox_search::faces::{FaceDetector, FaceEmbedder};
use shoebox_search::facts::{decode, encode, FactsIndex, Field, PhotoFacts};
use shoebox_search::query::{parse, ParseContext};
use shoebox_search::shard::{sign_bits, Vec8};
use shoebox_search::vectors::{RowIn, VectorIndex};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const PHOTOS: u32 = 100_000;

/// Resident memory in MB.
fn rss_mb() -> u64 {
    if let Ok(s) = std::fs::read_to_string("/proc/self/status") {
        if let Some(line) = s.lines().find(|l| l.starts_with("VmRSS:")) {
            let kb: u64 = line
                .split_whitespace()
                .nth(1)
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            return kb / 1024;
        }
    }
    std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .trim()
                .parse::<u64>()
                .ok()
        })
        .unwrap_or(0)
        / 1024
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// p50 and p95 of `runs` in ms.
fn pct(mut runs: Vec<Duration>) -> (f64, f64) {
    runs.sort();
    let at = |q: f64| ms(runs[((runs.len() - 1) as f64 * q).round() as usize]);
    (at(0.5), at(0.95))
}

/// A deterministic pseudo-random unit vector.
fn vector(seed: u64, dims: usize) -> Vec<f32> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut v: Vec<f32> = (0..dims)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 40) as f32 / (1u64 << 24) as f32 - 0.5
        })
        .collect();
    normalize(&mut v);
    v
}

fn blob(tag: &str) -> BlobRef {
    BlobRef {
        hash: Blake3Hash::of_bytes(tag.as_bytes()),
        size: 1,
        mime: "image/jpeg".into(),
    }
}

fn facts(scratch: &Path) -> Result<()> {
    let mut ix = FactsIndex::new();
    let start = Date::new(2005, 1, 1).days();
    let cams = ["Canon EOS R5", "iPhone 15 Pro", "Pixel 8", "X-T5"];
    let t = Instant::now();
    for i in 0..PHOTOS {
        let name = format!("photo-{i}");
        let rec = PhotoRecord {
            id: Blake3Hash::of_bytes(name.as_bytes()),
            original: blob(&name),
            preview: blob(&format!("{name}-p")),
            thumb: blob(&format!("{name}-t")),
            taken_at: Timestamp((start + i64::from(i) / 12) * 86_400_000),
            exif: ExifSummary {
                camera_make: Some(String::new()),
                camera_model: Some(cams[i as usize % 4].into()),
                lens: None,
                width: 4000,
                height: 3000,
                gps: (i % 3 == 0).then_some(Gps {
                    lat_e7: 100_000_000,
                    lon_e7: 760_000_000,
                }),
            },
            imported_by: EndpointId([1; 32]),
            favorite: Lww::new(i % 50 == 0, Hlc::GENESIS),
            deleted_at: Lww::new(None, Hlc::GENESIS),
        };
        ix.upsert(PhotoFacts::of(&rec));
        if i % 3 == 0 {
            ix.set_extra(&rec.id, Field::Place, Some("Kochi\nKerala\nIndia".into()));
        }
    }
    let built = t.elapsed();
    let bytes = encode(&ix.snapshot());
    let size = bytes.len();
    std::fs::write(scratch.join("bench-facts.idx"), &bytes)?;
    let cx = ParseContext {
        now: Zone::UTC.start_of(Date::new(2026, 10, 30)),
        zone: Zone::UTC,
        southern: false,
    };
    let v = ix.vocabulary();
    let queries = [
        "kerala 2019 pixel 8",
        "2012",
        "favourites canon",
        "march 2010",
    ];
    let runs: Vec<Duration> = (0..40)
        .map(|i| {
            let t = Instant::now();
            let q = parse(queries[i % queries.len()], &v, &cx);
            let hits = ix.search(&q, Zone::UTC, false, 0, 200);
            assert!(hits.total > 0);
            t.elapsed()
        })
        .collect();
    let (p50, p95) = pct(runs);
    println!(
        "facts: built in {:.1} s, facts.idx {:.1} MB, query p50 {p50:.1} ms p95 {p95:.1} ms, rss {} MB",
        built.as_secs_f64(),
        size as f64 / 1e6,
        rss_mb()
    );
    Ok(())
}

fn vectors(scratch: &Path) -> Result<()> {
    let dims = 512;
    let rows: Vec<(Blake3Hash, Vec8, Vec<u8>)> = (0..u64::from(PHOTOS))
        .map(|i| {
            let v = vector(i, dims);
            (
                Blake3Hash::of_bytes(&i.to_le_bytes()),
                Vec8::quantize(&v),
                sign_bits(&v),
            )
        })
        .collect();
    let path = scratch.join("bench-vec.bin");
    let ix = VectorIndex::build(
        &path,
        ModelId([0; 32]),
        dims,
        rows.iter().map(|(id, v8, v1)| RowIn {
            id: *id,
            vec8: Some(v8),
            vec1: Some(v1),
        }),
    )?;
    drop(rows);
    let size = std::fs::metadata(&path)?.len();
    let before = rss_mb();
    let runs: Vec<Duration> = (0..40)
        .map(|i| {
            let q = vector(1_000_000 + i, dims);
            let t = Instant::now();
            let s = ix.score(&q, None).expect("scores");
            assert!(!s.is_empty());
            t.elapsed()
        })
        .collect();
    let (p50, p95) = pct(runs);
    println!(
        "meaning: scoring {PHOTOS} vectors p50 {p50:.1} ms p95 {p95:.1} ms, file {:.1} MB, rss {before} MB",
        size as f64 / 1e6
    );
    Ok(())
}

fn rgb(img: image::RgbImage) -> Rgb {
    Rgb {
        width: img.width(),
        height: img.height(),
        data: img.into_raw(),
    }
}

fn photos(dir: &Path) -> Result<Vec<image::DynamicImage>> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "jpg"))
        .collect();
    paths.sort();
    paths
        .iter()
        .take(20)
        .map(|p| image::open(p).with_context(|| p.display().to_string()))
        .collect()
}

/// What search holds while idle: the facts index and the meaning index loaded at start.
fn idle(scratch: &Path) -> Result<()> {
    let base = rss_mb();
    let bytes = std::fs::read(scratch.join("bench-facts.idx"))?;
    let ix = FactsIndex::from_snapshot(decode(&bytes).context("bench-facts.idx")?);
    drop(bytes);
    let facts = rss_mb();
    let v = VectorIndex::open(&scratch.join("bench-vec.bin")).context("bench-vec.bin")?;
    let vectors = rss_mb();
    println!(
        "idle at {PHOTOS} photos: baseline {base} MB, facts index +{} MB, meaning index +{} MB, total +{} MB",
        facts.saturating_sub(base),
        vectors.saturating_sub(facts),
        vectors.saturating_sub(base)
    );
    drop((ix, v));
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
    if args.first().is_some_and(|a| a.as_os_str() == "--idle") {
        return idle(args.get(1).context("scratch dir")?);
    }
    let models = args.first().context("models dir")?;
    let photo_dir = args.get(1).context("photos dir")?;
    let scratch = args.get(2).cloned().unwrap_or_else(std::env::temp_dir);
    println!("baseline rss {} MB", rss_mb());
    facts(&scratch)?;
    vectors(&scratch)?;

    let tok = ClipTokenizer::from_merges(include_str!("../../shoebox-search/data/clip-merges.txt"))
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let before = rss_mb();
    let t = Instant::now();
    let text = ClipText::load(&models.join("clip-text-int8.onnx"), tok, ModelId([0; 32]))?;
    let loaded = t.elapsed();
    let queries = [
        "a dog on a beach",
        "birthday cake with candles",
        "receipt from a restaurant",
        "kids playing football in the rain",
    ];
    let _ = text.encode_text(queries[0])?;
    let runs: Vec<Duration> = (0..20)
        .map(|i| {
            let t = Instant::now();
            text.encode_text(queries[i % queries.len()])
                .expect("encodes");
            t.elapsed()
        })
        .collect();
    let (p50, p95) = pct(runs);
    println!(
        "text model: loaded in {:.2} s, +{} MB resident, query p50 {p50:.1} ms p95 {p95:.1} ms",
        loaded.as_secs_f64(),
        rss_mb().saturating_sub(before)
    );
    drop(text);

    let photos = photos(photo_dir)?;
    let before = rss_mb();
    let t = Instant::now();
    let image = ClipImage::load(&models.join("clip-image-int8.onnx"), ModelId([0; 32]))?;
    let loaded = t.elapsed();
    let thumbs: Vec<Rgb> = photos
        .iter()
        .map(|p| rgb(p.resize_to_fill(256, 256, FilterType::Triangle).to_rgb8()))
        .collect();
    let _ = image.encode_image(&thumbs[0])?;
    let t = Instant::now();
    for th in &thumbs {
        image.encode_image(th)?;
    }
    println!(
        "image model: loaded in {:.2} s, +{} MB resident, {:.0} ms/photo",
        loaded.as_secs_f64(),
        rss_mb().saturating_sub(before),
        ms(t.elapsed()) / thumbs.len() as f64
    );
    drop(image);

    let before = rss_mb();
    let detector = YuNet::load(&models.join("face-detect-yunet.onnx"))?;
    let embedder = SFace::load(&models.join("face-embed-sface.onnx"))?;
    let images: Vec<Rgb> = photos.iter().map(|p| rgb(p.to_rgb8())).collect();
    let _ = detector.detect(&images[0])?;
    let (mut detect, mut embed, mut faces) = (Duration::ZERO, Duration::ZERO, 0);
    for img in &images {
        let t = Instant::now();
        let found = detector.detect(img)?;
        detect += t.elapsed();
        let t = Instant::now();
        for d in &found {
            embedder.embed(img, &d.landmarks)?;
        }
        embed += t.elapsed();
        faces += found.len();
    }
    println!(
        "faces: +{} MB resident, detect {:.0} ms/photo, embed {:.0} ms/face ({faces} faces in {} photos)",
        rss_mb().saturating_sub(before),
        ms(detect) / images.len() as f64,
        if faces > 0 {
            ms(embed) / faces as f64
        } else {
            0.0
        },
        images.len()
    );
    Ok(())
}
