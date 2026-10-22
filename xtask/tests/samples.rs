use std::io::Cursor;
use xtask::samples::*;

fn exif_of(bytes: &[u8]) -> exif::Exif {
    exif::Reader::new()
        .read_from_container(&mut Cursor::new(bytes))
        .expect("exif parses")
}

#[test]
fn screenshot_is_a_phone_sized_png() {
    let png = screenshot_png();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    let decoder = png::Decoder::new(Cursor::new(&png));
    let reader = decoder.read_info().unwrap();
    let info = reader.info();
    assert_eq!((info.width, info.height), (390, 844));
}

#[test]
fn synthetic_dng_is_a_tiff_with_dng_tags() {
    let dng = synthetic_dng(64, 48);
    assert_eq!(&dng[..4], b"II*\0");
    let e = exif_of(&dng);
    let get = |tag| {
        e.get_field(tag, exif::In::PRIMARY)
            .unwrap()
            .display_value()
            .to_string()
    };
    assert_eq!(get(exif::Tag::ImageWidth), "64");
    assert_eq!(get(exif::Tag::ImageLength), "48");
    assert_eq!(get(exif::Tag::Make), "\"Shoebox\"");
    assert_eq!(get(exif::Tag::Compression), "uncompressed");
    // 16-bit samples for every pixel follow the IFD.
    assert!(dng.len() > 64 * 48 * 2);
}

#[test]
fn samples_include_a_raw_jpeg_pair_with_matching_stems() {
    let dir = std::env::temp_dir().join(format!("shoebox-samples-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let written = write_samples(&dir).unwrap();
    for must in [
        "photo-gps.jpg",
        "IMG_0001.jpg",
        "IMG_0001.dng",
        "screenshot.png",
    ] {
        assert!(
            written.iter().any(|w| w == must),
            "{must} written, got {written:?}"
        );
        assert!(dir.join(must).metadata().unwrap().len() > 0);
    }
    let jpg = std::fs::read(dir.join("IMG_0001.jpg")).unwrap();
    let e = exif_of(&jpg);
    assert!(e
        .get_field(exif::Tag::GPSLatitude, exif::In::PRIMARY)
        .is_some());
    let total: u64 = std::fs::read_dir(&dir)
        .unwrap()
        .map(|f| f.unwrap().metadata().unwrap().len())
        .sum();
    assert!(
        total < 20 * 1024 * 1024,
        "sample set stays under 20 MB, got {total}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}
