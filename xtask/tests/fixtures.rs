use std::collections::HashSet;
use std::io::Cursor;
use std::path::PathBuf;
use xtask::corpus::*;
use xtask::exifgen::*;
use xtask::jpeg::render_jpeg;

fn spec(gps: Option<(f64, f64)>) -> ExifSpec {
    ExifSpec {
        make: "Fujifilm".into(),
        model: "X100V".into(),
        lens: Some("23mm F2".into()),
        taken_at_secs: 1_700_000_000, // 2023-11-14 22:13:20 UTC
        gps,
        width: 64,
        height: 48,
    }
}

fn read_exif(bytes: &[u8]) -> exif::Exif {
    exif::Reader::new()
        .read_from_container(&mut Cursor::new(bytes))
        .expect("exif parses")
}

fn field(e: &exif::Exif, tag: exif::Tag) -> String {
    e.get_field(tag, exif::In::PRIMARY)
        .unwrap_or_else(|| panic!("missing {tag}"))
        .display_value()
        .to_string()
}

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("shoebox-xtask-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn civil_datetime_matches_known_instants() {
    assert_eq!(civil_datetime(0), (1970, 1, 1, 0, 0, 0));
    assert_eq!(civil_datetime(1_700_000_000), (2023, 11, 14, 22, 13, 20));
    assert_eq!(civil_datetime(951_782_400), (2000, 2, 29, 0, 0, 0)); // leap day
    assert_eq!(civil_datetime(-1), (1969, 12, 31, 23, 59, 59));
}

#[test]
fn generated_jpeg_carries_the_requested_exif() {
    let bytes = render_jpeg(&spec(Some((51.5074, -0.1278))), 7);
    assert_eq!(&bytes[..2], &[0xFF, 0xD8], "is a JPEG");
    let e = read_exif(&bytes);
    assert_eq!(field(&e, exif::Tag::Make), "\"Fujifilm\"");
    assert_eq!(field(&e, exif::Tag::Model), "\"X100V\"");
    assert_eq!(field(&e, exif::Tag::LensModel), "\"23mm F2\"");
    assert_eq!(
        field(&e, exif::Tag::DateTimeOriginal),
        "2023-11-14 22:13:20"
    );
    assert_eq!(field(&e, exif::Tag::PixelXDimension), "64");
    assert_eq!(field(&e, exif::Tag::PixelYDimension), "48");
    assert_eq!(field(&e, exif::Tag::GPSLatitudeRef), "N");
    assert_eq!(field(&e, exif::Tag::GPSLongitudeRef), "W");
    let lat = e
        .get_field(exif::Tag::GPSLatitude, exif::In::PRIMARY)
        .unwrap();
    if let exif::Value::Rational(r) = &lat.value {
        let deg = r[0].to_f64() + r[1].to_f64() / 60.0 + r[2].to_f64() / 3600.0;
        assert!((deg - 51.5074).abs() < 1e-3, "lat {deg}");
    } else {
        panic!("latitude is rational");
    }
}

#[test]
fn jpeg_without_gps_has_no_gps_fields_and_seed_changes_pixels() {
    let a = render_jpeg(&spec(None), 1);
    let e = read_exif(&a);
    assert!(e
        .get_field(exif::Tag::GPSLatitude, exif::In::PRIMARY)
        .is_none());
    assert!(e.get_field(exif::Tag::Make, exif::In::PRIMARY).is_some());
    assert_ne!(a, render_jpeg(&spec(None), 2));
    assert_eq!(a, render_jpeg(&spec(None), 1), "rendering is deterministic");
}

#[test]
fn presets_have_the_documented_sizes() {
    assert_eq!(Preset::parse("small"), Some(Preset::Small));
    assert_eq!(Preset::parse("nope"), None);
    assert_eq!(
        [
            Preset::Small.count(),
            Preset::Medium.count(),
            Preset::Large.count()
        ],
        [200, 10_000, 100_000]
    );
    assert!(Preset::Large.metadata_only_by_default());
    assert!(
        !Preset::Small.metadata_only_by_default() && !Preset::Medium.metadata_only_by_default()
    );
}

#[test]
fn plan_is_deterministic_varied_and_has_exact_duplicates() {
    let a = plan(1_000, 42, 5);
    assert_eq!(a, plan(1_000, 42, 5));
    assert_ne!(a, plan(1_000, 43, 5));
    assert_eq!(a.len(), 1_000);

    let dups: Vec<_> = a.iter().filter(|p| p.duplicate_of.is_some()).collect();
    assert!(
        (30..=70).contains(&dups.len()),
        "≈5% duplicates, got {}",
        dups.len()
    );
    for d in &dups {
        let orig = &a[d.duplicate_of.unwrap()];
        assert!(orig.index < d.index, "duplicates point backwards");
        assert_eq!(orig.content_seed, d.content_seed);
        assert_eq!(orig.exif, d.exif, "same bytes ⇒ same EXIF");
    }
    let seeds: HashSet<_> = a.iter().map(|p| p.content_seed).collect();
    assert_eq!(
        seeds.len(),
        1_000 - dups.len(),
        "every non-duplicate is unique"
    );

    let with_gps = a.iter().filter(|p| p.exif.gps.is_some()).count();
    assert!(
        (250..=550).contains(&with_gps),
        "GPS on for a good share, got {with_gps}"
    );
    let cameras: HashSet<_> = a.iter().map(|p| p.exif.model.clone()).collect();
    assert!(cameras.len() >= 4, "several cameras");
    let years: HashSet<_> = a
        .iter()
        .map(|p| civil_datetime(p.exif.taken_at_secs).0)
        .collect();
    assert!(years.len() >= 8, "dates span many years");
}

#[test]
fn generate_small_writes_files_manifest_and_dedupes() {
    let out = tmp("small");
    let s = generate(&Options {
        preset: Preset::Small,
        out_dir: out.clone(),
        seed: 1,
        metadata_only: None,
    })
    .unwrap();
    assert_eq!(s.photos, 200);
    assert_eq!(s.files_written, 200);
    assert!(s.duplicates > 0);

    let manifest = std::fs::read_to_string(&s.manifest).unwrap();
    let rows: Vec<serde_json::Value> = manifest
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(rows.len(), 200);
    let mut hashes = HashSet::new();
    for row in &rows {
        let path = out.join(row["path"].as_str().unwrap());
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(
            row["blake3"].as_str().unwrap(),
            blake3::hash(&bytes).to_hex().as_str()
        );
        assert_eq!(row["size"].as_u64().unwrap(), bytes.len() as u64);
        hashes.insert(row["blake3"].as_str().unwrap().to_string());
        if let Some(of) = row["duplicate_of"].as_str() {
            let orig = rows
                .iter()
                .find(|r| r["path"] == of)
                .expect("duplicate_of exists");
            assert_eq!(orig["blake3"], row["blake3"], "exact duplicate");
        }
    }
    assert_eq!(
        hashes.len(),
        200 - s.duplicates,
        "dedupe by hash leaves the unique photos"
    );
    std::fs::remove_dir_all(out).unwrap();
}

#[test]
fn metadata_only_mode_writes_just_the_manifest() {
    let out = tmp("meta");
    let s = generate(&Options {
        preset: Preset::Small,
        out_dir: out.clone(),
        seed: 1,
        metadata_only: Some(true),
    })
    .unwrap();
    assert_eq!((s.photos, s.files_written), (200, 0));
    let files: Vec<_> = std::fs::read_dir(&out)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(files.len(), 1, "only manifest.jsonl, got {files:?}");
    let first: serde_json::Value = serde_json::from_str(
        std::fs::read_to_string(&s.manifest)
            .unwrap()
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    assert!(first["blake3"].as_str().unwrap().len() == 64);
    std::fs::remove_dir_all(out).unwrap();
}
