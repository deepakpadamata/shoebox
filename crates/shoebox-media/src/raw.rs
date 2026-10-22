//! RAW support through the embedded JPEG preview every camera RAW carries.

use crate::AnalyzeError;
use image::{DynamicImage, ImageFormat};

/// Largest decodable JPEG embedded anywhere in the file (previews, not tiny EXIF thumbnails).
pub fn embedded_preview(bytes: &[u8]) -> Result<DynamicImage, AnalyzeError> {
    let mut best: Option<DynamicImage> = None;
    let mut tried = 0;
    let mut i = 0;
    while let Some(pos) = bytes[i..].windows(3).position(|w| w == [0xFF, 0xD8, 0xFF]) {
        let start = i + pos;
        i = start + 3;
        tried += 1;
        if tried > 32 {
            break;
        }
        if let Ok(img) = image::load_from_memory_with_format(&bytes[start..], ImageFormat::Jpeg) {
            let pixels = |im: &DynamicImage| u64::from(im.width()) * u64::from(im.height());
            if best.as_ref().is_none_or(|b| pixels(&img) > pixels(b)) {
                best = Some(img);
            }
        }
    }
    best.ok_or_else(|| AnalyzeError::Unsupported("RAW file has no embedded JPEG preview".into()))
}
