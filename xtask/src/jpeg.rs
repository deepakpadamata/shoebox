//! Tiny synthetic JPEGs with embedded EXIF.

use crate::exifgen::{build_exif_tiff, ExifSpec};
use jpeg_encoder::{ColorType, Encoder};

/// A small JPEG whose pixels depend on `content_seed` (so different seeds → different bytes).
pub fn render_jpeg(spec: &ExifSpec, content_seed: u64) -> Vec<u8> {
    let (w, h) = (spec.width as usize, spec.height as usize);
    let s = content_seed.to_le_bytes();
    let (r0, g0, b0) = (s[0], s[1], s[2]);
    let block = 4 + (s[3] % 8) as usize;
    let mut rgb = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        for x in 0..w {
            let checker = (((x / block) + (y / block)) % 2) as u8 * 40;
            rgb.push(r0.wrapping_add((x * 3) as u8).wrapping_add(checker));
            rgb.push(g0.wrapping_add((y * 2) as u8));
            rgb.push(
                b0.wrapping_add(((x + y) / 2) as u8)
                    .wrapping_add(s[4] & 0x1F),
            );
        }
    }
    let mut out = Vec::new();
    let mut enc = Encoder::new(&mut out, 80);
    let mut app1 = b"Exif\0\0".to_vec();
    app1.extend(build_exif_tiff(spec));
    enc.add_app_segment(1, app1)
        .expect("EXIF fits in an APP1 segment");
    enc.encode(&rgb, w as u16, h as u16, ColorType::Rgb)
        .expect("encode in-memory JPEG");
    out
}
