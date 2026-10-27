//! M1-04: the import pipeline as a pure function over a path.

use image::GenericImageView;
use shoebox_core::ids::{Blake3Hash, Timestamp};
use shoebox_media::{analyze, AnalyzeError, MediaKind, TakenAtSource};
use std::path::{Path, PathBuf};
use xtask::exifgen::ExifSpec;
use xtask::jpeg::render_jpeg;

fn samples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/samples")
}

fn decode(bytes: &[u8]) -> image::DynamicImage {
    image::load_from_memory(bytes).expect("rendition is a decodable image")
}

fn spec(width: u32, height: u32, orientation: u16) -> ExifSpec {
    ExifSpec {
        make: "Shoebox".into(),
        model: "Test".into(),
        lens: None,
        taken_at_secs: 1_717_243_200,
        gps: None,
        width,
        height,
        orientation,
    }
}

fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, bytes).unwrap();
    p
}

#[test]
fn jpeg_sample_yields_hash_exif_and_renditions() {
    let path = samples().join("photo-gps.jpg");
    let r = analyze(&path).unwrap();
    let bytes = std::fs::read(&path).unwrap();

    assert_eq!(r.kind, MediaKind::Photo);
    assert_eq!(r.hash, Blake3Hash::of_bytes(&bytes));
    assert_eq!(
        (r.size, r.mime.as_str()),
        (bytes.len() as u64, "image/jpeg")
    );
    assert_eq!(
        r.taken_at,
        Timestamp(1_717_243_200_000),
        "2024-06-01 12:00:00 UTC"
    );
    assert_eq!(r.taken_at_source, TakenAtSource::Exif);
    assert_eq!(r.exif.camera_make.as_deref(), Some("Shoebox"));
    assert_eq!(r.exif.camera_model.as_deref(), Some("Synthetic Camera"));
    assert_eq!(r.exif.lens.as_deref(), Some("Synthetic 35mm f/2"));
    assert_eq!((r.exif.width, r.exif.height), (640, 480));
    let gps = r.exif.gps.expect("gps");
    assert!((gps.lat_e7 as f64 / 1e7 - 48.8584).abs() < 1e-3, "{gps:?}");
    assert!((gps.lon_e7 as f64 / 1e7 - 2.2945).abs() < 1e-3, "{gps:?}");

    let thumb = r.thumb.expect("thumb");
    let img = decode(&thumb.bytes);
    assert_eq!(img.dimensions(), (256, 256), "square crop");
    assert_eq!(thumb.mime, "image/jpeg");
    let preview = r.preview.expect("preview");
    assert_eq!(
        decode(&preview.bytes).dimensions(),
        (640, 480),
        "small images are not upscaled"
    );
}

#[test]
fn large_images_get_a_2048_long_edge_preview() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "big.jpg", &render_jpeg(&spec(4000, 3000, 1), 5));
    let r = analyze(&path).unwrap();
    let preview = r.preview.unwrap();
    assert_eq!(decode(&preview.bytes).dimensions(), (2048, 1536));
    assert_eq!((preview.width, preview.height), (2048, 1536));
    assert_eq!((r.exif.width, r.exif.height), (4000, 3000));
}

#[test]
fn thumbnails_and_previews_respect_exif_orientation() {
    // The generator's red channel increases left→right. Orientation 6 (rotate 90° CW) turns
    // that into top→bottom, so a correctly oriented rendition has red increasing downwards.
    let dir = tempfile::tempdir().unwrap();
    let upright = analyze(&write(
        dir.path(),
        "o1.jpg",
        &render_jpeg(&spec(80, 40, 1), 0),
    ))
    .unwrap();
    let rotated = analyze(&write(
        dir.path(),
        "o6.jpg",
        &render_jpeg(&spec(80, 40, 6), 0),
    ))
    .unwrap();

    assert_eq!((upright.exif.width, upright.exif.height), (80, 40));
    assert_eq!(
        (rotated.exif.width, rotated.exif.height),
        (40, 80),
        "reported dimensions are as displayed"
    );
    let up = decode(&upright.preview.unwrap().bytes);
    let rot = decode(&rotated.preview.unwrap().bytes);
    assert_eq!(up.dimensions(), (80, 40));
    assert_eq!(rot.dimensions(), (40, 80));

    let red = |img: &image::DynamicImage, x0: u32, y0: u32, w: u32, h: u32| -> f64 {
        let rgb = img.to_rgb8();
        let mut sum = 0u64;
        for y in y0..y0 + h {
            for x in x0..x0 + w {
                sum += rgb.get_pixel(x, y)[0] as u64;
            }
        }
        sum as f64 / (w * h) as f64
    };
    // Upright: right side redder than left side.
    assert!(red(&up, 60, 0, 20, 40) > red(&up, 0, 0, 20, 40) + 80.0);
    // Rotated: bottom redder than top, and left/right roughly equal.
    assert!(red(&rot, 0, 60, 40, 20) > red(&rot, 0, 0, 40, 20) + 80.0);
    assert!((red(&rot, 0, 0, 10, 80) - red(&rot, 30, 0, 10, 80)).abs() < 40.0);
    // Thumbnails are oriented too (checked via the same gradient direction).
    let t = decode(&rotated.thumb.unwrap().bytes);
    assert!(red(&t, 0, 200, 256, 56) > red(&t, 0, 0, 256, 56) + 60.0);
}

#[test]
fn file_mtime_is_the_fallback_when_there_is_no_exif() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shot.png");
    std::fs::copy(samples().join("screenshot.png"), &path).unwrap();
    let mtime = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(mtime)
        .unwrap();

    let r = analyze(&path).unwrap();
    assert_eq!(r.mime, "image/png");
    assert_eq!(r.taken_at, Timestamp(1_600_000_000_000));
    assert_eq!(r.taken_at_source, TakenAtSource::FileMtime);
    assert_eq!((r.exif.width, r.exif.height), (390, 844));
    assert!(r.exif.camera_make.is_none() && r.exif.gps.is_none());
    assert_eq!(decode(&r.thumb.unwrap().bytes).dimensions(), (256, 256));
    assert_eq!(decode(&r.preview.unwrap().bytes).dimensions(), (390, 844));
}

#[test]
fn raw_files_use_the_embedded_jpeg_preview() {
    let dir = tempfile::tempdir().unwrap();
    // A TIFF-based RAW with an embedded JPEG preview (here appended after the sensor data).
    let mut raw = xtask::samples::synthetic_dng(64, 48);
    raw.extend(render_jpeg(&spec(320, 240, 1), 9));
    let path = write(dir.path(), "IMG_9999.dng", &raw);
    let r = analyze(&path).unwrap();
    assert_eq!(r.kind, MediaKind::Photo);
    assert_eq!(
        r.hash,
        Blake3Hash::of_bytes(&raw),
        "the RAW file itself is the original"
    );
    assert_eq!(r.mime, "image/x-adobe-dng");
    assert_eq!(r.exif.camera_make.as_deref(), Some("Shoebox"));
    assert_eq!(decode(&r.preview.unwrap().bytes).dimensions(), (320, 240));
    assert_eq!(decode(&r.thumb.unwrap().bytes).dimensions(), (256, 256));

    // A RAW without any embedded preview is reported, not guessed at.
    let bare = write(
        dir.path(),
        "bare.dng",
        &xtask::samples::synthetic_dng(64, 48),
    );
    assert!(matches!(analyze(&bare), Err(AnalyzeError::Unsupported(_))));
}

#[test]
fn videos_yield_metadata_only() {
    for name in ["clip.mp4", "clip.mov"] {
        let r = analyze(&samples().join(name)).unwrap();
        assert_eq!(r.kind, MediaKind::Video, "{name}");
        assert_eq!(
            r.taken_at,
            Timestamp(1_717_243_200_000),
            "{name}: creation_time from mvhd"
        );
        assert_eq!(r.taken_at_source, TakenAtSource::Exif, "{name}");
        assert_eq!((r.exif.width, r.exif.height), (320, 240), "{name}");
        assert!(
            r.thumb.is_none() && r.preview.is_none(),
            "{name}: poster frames come in M3-10"
        );
        assert!(r.mime.starts_with("video/"), "{name}: {}", r.mime);
        assert_eq!(
            r.hash,
            Blake3Hash::of_bytes(&std::fs::read(samples().join(name)).unwrap())
        );
    }
}

#[test]
fn unsupported_and_broken_files_are_reported_not_panicked_on() {
    let dir = tempfile::tempdir().unwrap();
    let text = write(dir.path(), "notes.txt", b"hello, this is not a photo");
    assert!(matches!(analyze(&text), Err(AnalyzeError::Unsupported(_))));
    let empty = write(dir.path(), "empty.jpg", b"");
    assert!(analyze(&empty).is_err());
    assert!(matches!(
        analyze(&dir.path().join("missing.jpg")),
        Err(AnalyzeError::Io(_))
    ));
    assert!(matches!(
        analyze(dir.path()),
        Err(AnalyzeError::Io(_) | AnalyzeError::Unsupported(_))
    ));

    // A JPEG cut off mid-stream.
    let good = std::fs::read(samples().join("photo-gps.jpg")).unwrap();
    let cut = write(dir.path(), "cut.jpg", &good[..good.len() / 3]);
    assert!(matches!(analyze(&cut), Err(AnalyzeError::Decode(_))));

    // Garbage with image-looking headers, in many shapes: must never panic.
    for (i, head) in [
        &b"\xFF\xD8\xFF\xE0"[..],
        b"\x89PNG\r\n\x1a\n",
        b"II*\0",
        b"\0\0\0\x18ftypheic",
        b"\0\0\0\x18ftypmp42",
    ]
    .iter()
    .enumerate()
    {
        for len in [0usize, 7, 64, 4096] {
            let mut bytes = head.to_vec();
            bytes.extend((0..len).map(|n| (n * 31 + i) as u8));
            let p = write(dir.path(), &format!("junk-{i}-{len}.bin"), &bytes);
            let _ = analyze(&p); // Ok or Err are both fine; panicking is not
        }
    }
}

#[test]
fn small_corpus_imports_well_above_20_photos_per_second() {
    let out = tempfile::tempdir().unwrap();
    let s = xtask::corpus::generate(&xtask::corpus::Options {
        preset: xtask::corpus::Preset::Small,
        out_dir: out.path().to_path_buf(),
        seed: 3,
        metadata_only: None,
    })
    .unwrap();
    let manifest = std::fs::read_to_string(&s.manifest).unwrap();
    let start = std::time::Instant::now();
    let mut n = 0;
    for line in manifest.lines() {
        let row: serde_json::Value = serde_json::from_str(line).unwrap();
        let r = analyze(&out.path().join(row["path"].as_str().unwrap())).unwrap();
        assert_eq!(r.hash.to_string(), row["blake3"].as_str().unwrap());
        assert_eq!(r.taken_at.0, row["taken_at"].as_i64().unwrap() * 1000);
        assert_eq!(r.exif.camera_model.as_deref(), row["camera_model"].as_str());
        assert_eq!(r.exif.gps.is_some(), !row["gps"].is_null());
        n += 1;
    }
    let rate = n as f64 / start.elapsed().as_secs_f64();
    // A debug build is no measure of speed (18/s on a busy 12-core Linux box, under 20 on CI
    // VMs): enforce the bar in optimised builds (`cargo test --release`), report it otherwise.
    if cfg!(debug_assertions) {
        eprintln!("{rate:.0} photos/s (enforced only in release builds)");
        return;
    }
    assert!(rate >= 20.0, "{rate:.0} photos/s");
}

/// Acceptance check for M1-04 on the full `medium` corpus (10k photos). Opt-in, run in release:
/// `cargo test -p shoebox-media --release --test analyze -- --ignored medium_corpus --nocapture`
#[test]
#[ignore = "slow: 10k photos"]
fn medium_corpus_imports_at_20_photos_per_second_or_better() {
    let out = tempfile::tempdir().unwrap();
    let s = xtask::corpus::generate(&xtask::corpus::Options {
        preset: xtask::corpus::Preset::Medium,
        out_dir: out.path().to_path_buf(),
        seed: 1,
        metadata_only: None,
    })
    .unwrap();
    let manifest = std::fs::read_to_string(&s.manifest).unwrap();
    let start = std::time::Instant::now();
    let mut n = 0;
    for line in manifest.lines() {
        let row: serde_json::Value = serde_json::from_str(line).unwrap();
        analyze(&out.path().join(row["path"].as_str().unwrap())).unwrap();
        n += 1;
    }
    let rate = n as f64 / start.elapsed().as_secs_f64();
    eprintln!("medium corpus: {n} photos at {rate:.0} photos/s");
    assert!(rate >= 20.0, "{rate:.0} photos/s");
}
