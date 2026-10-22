//! Deterministic synthetic photo corpus with duplicates, written to disk with a manifest.

use crate::exifgen::{civil_datetime, ExifSpec};
use crate::jpeg::render_jpeg;
use serde::Serialize;
use std::fs;
use std::io::{self, BufWriter, Write};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    Small,
    Medium,
    Large,
}

impl Preset {
    pub fn parse(s: &str) -> Option<Preset> {
        match s {
            "small" => Some(Preset::Small),
            "medium" => Some(Preset::Medium),
            "large" => Some(Preset::Large),
            _ => None,
        }
    }
    pub fn count(self) -> usize {
        match self {
            Preset::Small => 200,
            Preset::Medium => 10_000,
            Preset::Large => 100_000,
        }
    }
    /// `large` writes only the manifest by default.
    pub fn metadata_only_by_default(self) -> bool {
        self == Preset::Large
    }
    pub fn name(self) -> &'static str {
        match self {
            Preset::Small => "small",
            Preset::Medium => "medium",
            Preset::Large => "large",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PhotoSpec {
    pub index: usize,
    /// Photos with the same `content_seed` have byte-identical content.
    pub content_seed: u64,
    pub exif: ExifSpec,
    /// Index of the earlier photo this is an exact copy of.
    pub duplicate_of: Option<usize>,
}

/// SplitMix64: tiny, fast, deterministic.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

const CAMERAS: &[(&str, &str, &str)] = &[
    ("Apple", "iPhone 13", "iPhone 13 back camera 5.1mm f/1.6"),
    (
        "Apple",
        "iPhone 15 Pro",
        "iPhone 15 Pro back triple camera 6.8mm f/1.78",
    ),
    ("samsung", "SM-S918B", "Galaxy S23 Ultra 6.3mm f/1.7"),
    ("Google", "Pixel 8", "Pixel 8 main camera 6.9mm f/1.68"),
    ("FUJIFILM", "X-T4", "XF35mmF2 R WR"),
    ("Canon", "Canon EOS R6", "RF24-105mm F4 L IS USM"),
    ("SONY", "ILCE-7M3", "FE 50mm F1.8"),
];
const SIZES: &[(u32, u32)] = &[(64, 48), (48, 64), (80, 60), (60, 80), (64, 64)];
/// A handful of cities (lat, lon); photos get a small jitter around one of them.
const PLACES: &[(f64, f64)] = &[
    (51.5074, -0.1278),
    (40.7128, -74.0060),
    (35.6762, 139.6503),
    (-33.8688, 151.2093),
    (12.9716, 77.5946),
    (-23.5505, -46.6333),
    (64.1466, -21.9426),
];
const START_SECS: i64 = 1_388_534_400; // 2014-01-01
const SPAN_SECS: u64 = 12 * 365 * 86_400; // ~12 years

/// Deterministic plan: `duplicate_percent` of photos are exact copies of an earlier one.
pub fn plan(count: usize, seed: u64, duplicate_percent: u32) -> Vec<PhotoSpec> {
    let mut rng = Rng(seed);
    let mut out: Vec<PhotoSpec> = Vec::with_capacity(count);
    let mut originals: Vec<usize> = Vec::new();
    for index in 0..count {
        if !originals.is_empty() && rng.below(100) < u64::from(duplicate_percent) {
            let of = originals[rng.below(originals.len() as u64) as usize];
            let src = out[of].clone();
            out.push(PhotoSpec {
                index,
                duplicate_of: Some(of),
                ..src
            });
            continue;
        }
        let (make, model, lens) = CAMERAS[rng.below(CAMERAS.len() as u64) as usize];
        let (width, height) = SIZES[rng.below(SIZES.len() as u64) as usize];
        let gps = (rng.below(100) < 40).then(|| {
            let (lat, lon) = PLACES[rng.below(PLACES.len() as u64) as usize];
            (
                lat + (rng.unit() - 0.5) * 0.2,
                lon + (rng.unit() - 0.5) * 0.2,
            )
        });
        let exif = ExifSpec {
            make: make.into(),
            model: model.into(),
            lens: (rng.below(100) < 80).then(|| lens.into()),
            taken_at_secs: START_SECS + rng.below(SPAN_SECS) as i64,
            gps,
            width,
            height,
        };
        // Unique per photo: mixing the index in guarantees no accidental collisions.
        let content_seed = rng.next() ^ ((index as u64) << 40);
        originals.push(index);
        out.push(PhotoSpec {
            index,
            content_seed,
            exif,
            duplicate_of: None,
        });
    }
    out
}

pub struct Options {
    pub preset: Preset,
    pub out_dir: PathBuf,
    pub seed: u64,
    /// `None` = the preset's default.
    pub metadata_only: Option<bool>,
}

#[derive(Debug)]
pub struct Summary {
    pub photos: usize,
    pub duplicates: usize,
    pub files_written: usize,
    pub manifest: PathBuf,
}

#[derive(Serialize)]
struct Row<'a> {
    path: String,
    blake3: String,
    size: u64,
    taken_at: i64,
    camera_make: &'a str,
    camera_model: &'a str,
    lens: Option<&'a str>,
    gps: Option<(f64, f64)>,
    width: u32,
    height: u32,
    duplicate_of: Option<String>,
}

fn rel_path(spec: &PhotoSpec) -> String {
    let (y, m, ..) = civil_datetime(spec.exif.taken_at_secs);
    format!("photos/{y:04}/{m:02}/IMG_{:06}.jpg", spec.index)
}

pub fn generate(opts: &Options) -> io::Result<Summary> {
    let metadata_only = opts
        .metadata_only
        .unwrap_or(opts.preset.metadata_only_by_default());
    let specs = plan(opts.preset.count(), opts.seed, 5);
    fs::create_dir_all(&opts.out_dir)?;
    let manifest = opts.out_dir.join("manifest.jsonl");
    let mut writer = BufWriter::new(fs::File::create(&manifest)?);
    let (mut files_written, mut duplicates) = (0, 0);
    for spec in &specs {
        let path = rel_path(spec);
        let (hash, size) = if metadata_only {
            // No bytes exist, so the hash is a stable stand-in derived from the content seed.
            let mut key = b"shoebox-fixture".to_vec();
            key.extend(spec.content_seed.to_le_bytes());
            (blake3::hash(&key), 0)
        } else {
            let bytes = render_jpeg(&spec.exif, spec.content_seed);
            let full = opts.out_dir.join(&path);
            fs::create_dir_all(full.parent().expect("has parent"))?;
            fs::write(&full, &bytes)?;
            files_written += 1;
            (blake3::hash(&bytes), bytes.len() as u64)
        };
        if spec.duplicate_of.is_some() {
            duplicates += 1;
        }
        let row = Row {
            path,
            blake3: hash.to_hex().to_string(),
            size,
            taken_at: spec.exif.taken_at_secs,
            camera_make: &spec.exif.make,
            camera_model: &spec.exif.model,
            lens: spec.exif.lens.as_deref(),
            gps: spec.exif.gps,
            width: spec.exif.width,
            height: spec.exif.height,
            duplicate_of: spec.duplicate_of.map(|i| rel_path(&specs[i])),
        };
        serde_json::to_writer(&mut writer, &row).map_err(io::Error::other)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(Summary {
        photos: specs.len(),
        duplicates,
        files_written,
        manifest,
    })
}
