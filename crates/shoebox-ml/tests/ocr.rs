//! PP-OCRv5 on tract reads text in photos (S2-03). Needs the three OCR files (`cargo xtask models`,
//! or in `models/build`); without them the tests say so and pass, as CI has no model files.

use shoebox_ml::files::search_dirs;
use shoebox_ml::PpOcr;
use shoebox_search::ocr::Ocr;

fn engine() -> Option<PpOcr> {
    let e = PpOcr::locate(&search_dirs(&[])).expect("the OCR models load");
    if e.is_none() {
        eprintln!("the OCR model files are not installed: skipped");
    }
    e
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/../../fixtures/ocr/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

/// The photo encoded as PNG, as a blob would hold it.
fn png(img: &image::RgbImage) -> Vec<u8> {
    let mut out = std::io::Cursor::new(vec![]);
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

fn squash(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_uppercase()
}

/// The photo turned by `degrees` about its centre, on a white ground, enlarged to hold it.
fn tilted(img: &image::RgbImage, degrees: f32) -> image::RgbImage {
    let (s, c) = degrees.to_radians().sin_cos();
    let (w, h) = (img.width() as f32, img.height() as f32);
    let (nw, nh) = (
        (w * c.abs() + h * s.abs()).ceil(),
        (w * s.abs() + h * c.abs()).ceil(),
    );
    let mut out = image::RgbImage::from_pixel(nw as u32, nh as u32, image::Rgb([255, 255, 255]));
    for y in 0..nh as u32 {
        for x in 0..nw as u32 {
            let (dx, dy) = (x as f32 - nw / 2.0, y as f32 - nh / 2.0);
            let (sx, sy) = (dx * c + dy * s + w / 2.0, -dx * s + dy * c + h / 2.0);
            if sx >= 0.0 && sy >= 0.0 && sx < w - 1.0 && sy < h - 1.0 {
                let p = img.get_pixel(sx as u32, sy as u32);
                out.put_pixel(x, y, *p);
            }
        }
    }
    out
}

#[test]
fn reads_the_receipt() {
    let Some(ocr) = engine() else { return };
    let text = squash(&ocr.read_text(&fixture("receipt.png")).unwrap());
    for want in [
        "CORNER SHOP",
        "2 X COFFEE",
        "1 X CROISSANT",
        "GRAND TOTAL 12.50",
        "THANK YOU",
    ] {
        assert!(text.contains(want), "{want:?} is not in {text:?}");
    }
}

#[test]
fn reads_lines_top_to_bottom() {
    let Some(ocr) = engine() else { return };
    let text = ocr.read_text(&fixture("receipt.png")).unwrap();
    let at = |w: &str| {
        text.to_uppercase()
            .find(w)
            .unwrap_or_else(|| panic!("{w} in {text:?}"))
    };
    assert!(at("CORNER") < at("CROISSANT") && at("CROISSANT") < at("GRAND"));
}

#[test]
fn reads_a_tilted_photo() {
    let Some(ocr) = engine() else { return };
    let img = image::load_from_memory(&fixture("receipt.png"))
        .unwrap()
        .to_rgb8();
    let text = squash(&ocr.read_text(&png(&tilted(&img, 12.0))).unwrap());
    for want in ["CORNER SHOP", "GRAND TOTAL", "12.50"] {
        assert!(text.contains(want), "{want:?} is not in {text:?}");
    }
}

#[test]
fn reads_text_that_runs_down_the_photo() {
    let Some(ocr) = engine() else { return };
    let img = image::load_from_memory(&fixture("receipt.png"))
        .unwrap()
        .to_rgb8();
    // A photo taken turned a quarter: the lines run top to bottom.
    let text = squash(
        &ocr.read_text(&png(&image::imageops::rotate90(&img)))
            .unwrap(),
    );
    for want in ["CORNER SHOP", "GRAND TOTAL"] {
        assert!(text.contains(want), "{want:?} is not in {text:?}");
    }
}

#[test]
fn a_photo_with_no_text_reads_as_nothing() {
    let Some(ocr) = engine() else { return };
    let img =
        image::RgbImage::from_fn(300, 200, |x, y| image::Rgb([(x / 2) as u8, (y) as u8, 120]));
    assert_eq!(ocr.read_text(&png(&img)).unwrap(), "");
}

#[test]
fn something_that_is_not_a_picture_is_an_error() {
    let Some(ocr) = engine() else { return };
    assert!(ocr.read_text(b"not an image").is_err());
}

#[test]
fn a_damaged_model_file_is_refused() {
    let dict = "a\nb\n";
    assert!(PpOcr::from_bytes(b"nonsense", b"nonsense", dict).is_err());
    assert!(PpOcr::from_bytes(b"x", b"y", "").is_err());
}
