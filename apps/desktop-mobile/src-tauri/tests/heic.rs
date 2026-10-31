//! I1-02: HEIC read by the system's decoder on macOS (the app registers it when it opens).
#![cfg(target_os = "macos")]

use desktop_mobile::{AppCore, AppOptions};
use shoebox_media::{analyze, MediaKind, TakenAtSource};
use std::path::{Path, PathBuf};
use std::process::Command;
use xtask::exifgen::ExifSpec;
use xtask::jpeg::render_jpeg;

fn samples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/samples")
}

#[tokio::test]
async fn heic_photos_import_upright_with_their_date_and_place() {
    let dir = tempfile::tempdir().unwrap();
    let _core = AppCore::open(dir.path(), AppOptions::local_only())
        .await
        .unwrap();

    let r = analyze(&samples().join("photo.heic")).unwrap();
    assert_eq!((r.kind, r.mime.as_str()), (MediaKind::Photo, "image/heic"));
    assert_eq!((r.exif.width, r.exif.height), (640, 480));
    assert_eq!(r.taken_at_source, TakenAtSource::Exif);
    assert_eq!(r.taken_at.0, 1_717_243_200_000);
    assert!(r.exif.gps.is_some());
    assert_eq!(r.thumb.expect("thumb").width, 256);
    assert_eq!(r.preview.expect("preview").width, 640);

    // Sideways pixels with EXIF orientation 6 (rotate 90): heif-enc stores it as a rotation of
    // the picture, so the decoded size is 480x640 and the rotation is applied exactly once.
    let jpg = dir.path().join("sideways.jpg");
    let spec = ExifSpec {
        make: "Shoebox".into(),
        model: "Test".into(),
        lens: None,
        taken_at_secs: 1_717_243_200,
        gps: None,
        width: 640,
        height: 480,
        orientation: 6,
    };
    std::fs::write(&jpg, render_jpeg(&spec, 3)).unwrap();
    let heic = dir.path().join("sideways.heic");
    let made = Command::new("heif-enc")
        .arg(&jpg)
        .arg("-o")
        .arg(&heic)
        .output();
    if !matches!(made, Ok(ref o) if o.status.success()) {
        eprintln!("skipped the rotation check: heif-enc is not installed");
        return;
    }
    let r = analyze(&heic).unwrap();
    assert_eq!(
        (r.exif.width, r.exif.height),
        (480, 640),
        "upright: portrait"
    );
}

#[tokio::test]
async fn avif_photos_import_too() {
    let dir = tempfile::tempdir().unwrap();
    let _core = AppCore::open(dir.path(), AppOptions::local_only())
        .await
        .unwrap();
    let r = analyze(&samples().join("photo.avif")).unwrap();
    assert_eq!((r.kind, r.mime.as_str()), (MediaKind::Photo, "image/avif"));
    assert_eq!((r.exif.width, r.exif.height), (640, 480));
    assert_eq!(r.preview.expect("preview").width, 640);
}

#[tokio::test]
async fn a_broken_heic_is_reported_not_panicked_on() {
    let dir = tempfile::tempdir().unwrap();
    let _core = AppCore::open(dir.path(), AppOptions::local_only())
        .await
        .unwrap();
    let good = std::fs::read(samples().join("photo.heic")).unwrap();
    let cut = dir.path().join("cut.heic");
    std::fs::write(&cut, &good[..good.len() / 2]).unwrap();
    assert!(analyze(&cut).is_err());
}
