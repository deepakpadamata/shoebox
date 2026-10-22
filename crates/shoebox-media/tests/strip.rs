//! M5-03: location removal changes only metadata bytes.

use shoebox_media::{analyze, strip_location, StripError};
use xtask::exifgen::{build_exif_tiff, ExifSpec};
use xtask::jpeg::render_jpeg;

fn spec(gps: Option<(f64, f64)>) -> ExifSpec {
    ExifSpec {
        make: "Shoebox".into(),
        model: "Test".into(),
        lens: None,
        taken_at_secs: 1_717_243_200,
        gps,
        width: 64,
        height: 48,
        orientation: 1,
    }
}

fn gps_of(bytes: &[u8], name: &str) -> Option<shoebox_core::model::Gps> {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join(name);
    std::fs::write(&p, bytes).unwrap();
    analyze(&p).unwrap().exif.gps
}

fn pixels(bytes: &[u8]) -> Vec<u8> {
    image::load_from_memory(bytes).unwrap().to_rgba8().into_raw()
}

#[test]
fn a_jpeg_loses_its_gps_and_nothing_else() {
    let original = render_jpeg(&spec(Some((38.7223, -9.1393))), 7);
    assert!(gps_of(&original, "a.jpg").is_some());
    let s = strip_location(&original).unwrap();
    assert!(s.changed);
    assert_eq!(s.bytes.len(), original.len(), "no re-encode, no resize");
    assert!(gps_of(&s.bytes, "b.jpg").is_none());
    assert_eq!(pixels(&s.bytes), pixels(&original), "identical pixels");
    // The other metadata survives.
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("c.jpg");
    std::fs::write(&p, &s.bytes).unwrap();
    let meta = analyze(&p).unwrap().exif;
    assert_eq!(meta.camera_make.as_deref(), Some("Shoebox"));
    // Only bytes before the image data differ.
    let sos = original.windows(2).position(|w| w == [0xFF, 0xDA]).unwrap();
    assert_eq!(&original[sos..], &s.bytes[sos..]);
    // Nothing left that looks like the coordinates (38° 43' 20"): numerators 38, 43, 20 stored
    // as rationals sit in the zeroed GPS block.
    let again = strip_location(&s.bytes).unwrap();
    assert!(!again.changed, "stripping twice finds nothing");
}

#[test]
fn a_jpeg_without_gps_is_returned_as_it_was() {
    let original = render_jpeg(&spec(None), 3);
    let s = strip_location(&original).unwrap();
    assert!(!s.changed);
    assert_eq!(s.bytes, original);
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for b in data {
        crc ^= *b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn chunk(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut v = (body.len() as u32).to_be_bytes().to_vec();
    v.extend(kind);
    v.extend(body);
    v.extend(crc32(&[kind.as_slice(), body].concat()).to_be_bytes());
    v
}

#[test]
fn a_png_drops_its_exif_chunk_with_gps_and_keeps_the_image() {
    let mut png = Vec::new();
    image::DynamicImage::new_rgb8(8, 8)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let iend = png.len() - 12;
    let exif = build_exif_tiff(&spec(Some((1.0, 2.0))));
    let mut with = png[..iend].to_vec();
    with.extend(chunk(b"eXIf", &exif));
    with.extend(&png[iend..]);
    let s = strip_location(&with).unwrap();
    assert!(s.changed);
    assert_eq!(s.bytes, png, "back to the plain PNG");
    assert_eq!(pixels(&s.bytes), pixels(&with));
}

fn bx(kind: &[u8], body: &[u8]) -> Vec<u8> {
    let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
    v.extend(kind);
    v.extend(body);
    v
}

#[test]
fn an_mp4_loses_its_location_atom_and_keeps_the_media() {
    let loc = bx(b"\xA9xyz", b"\x00\x0e\x15\xc7+38.7223-009.1393/");
    let other = bx(b"name", b"clip");
    let udta = bx(b"udta", &[other.clone(), loc.clone()].concat());
    let moov = bx(b"moov", &udta);
    let ftyp = bx(b"ftyp", b"isom\0\0\0\0isom");
    let mdat = bx(b"mdat", &[7u8; 64]);
    let file = [ftyp.clone(), moov, mdat.clone()].concat();
    let s = strip_location(&file).unwrap();
    assert!(s.changed);
    assert_eq!(s.bytes.len(), file.len());
    assert!(!s.bytes.windows(7).any(|w| w == b"+38.722"), "the coordinates are gone");
    assert!(s.bytes.windows(4).any(|w| w == b"free"));
    assert!(s.bytes.ends_with(&mdat), "media data untouched");
    assert!(s.bytes.windows(4).any(|w| w == b"clip"), "other metadata kept");
}

#[test]
fn xmp_gps_properties_are_blanked() {
    let mut xmp = b"http://ns.adobe.com/xap/1.0/\0<x:xmpmeta><rdf:Description exif:GPSLatitude=\"38,43.3N\" tiff:Make=\"Z\"><exif:GPSLongitude>9,8.3W</exif:GPSLongitude></rdf:Description></x:xmpmeta>".to_vec();
    let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE1];
    jpeg.extend(((xmp.len() + 2) as u16).to_be_bytes());
    jpeg.append(&mut xmp);
    jpeg.extend([0xFF, 0xDA, 0, 2, 0xFF, 0xD9]);
    let s = strip_location(&jpeg).unwrap();
    assert!(s.changed);
    let text = String::from_utf8_lossy(&s.bytes);
    assert!(!text.contains("GPS") && !text.contains("38,43"), "{text}");
    assert!(text.contains("tiff:Make=\"Z\""));
}

#[test]
fn heic_and_unknown_files_are_refused_not_passed_through() {
    let heic = bx(b"ftyp", b"heic\0\0\0\0mif1heic");
    assert!(matches!(strip_location(&heic), Err(StripError::Unsupported(_))));
    assert!(matches!(strip_location(b"GIF89a....."), Err(StripError::Unsupported(_))));
    assert!(matches!(
        strip_location(&[0xFF, 0xD8, 0xFF, 0xE1, 0xFF, 0xFF]),
        Err(StripError::Malformed(_))
    ));
}
