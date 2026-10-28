//! Decoding, orientation and the thumbnail/preview renditions.

use crate::{AnalyzeError, Rendition};
use image::imageops::FilterType;
use image::metadata::Orientation;
use image::{DynamicImage, ImageFormat};

/// Square thumbnail edge.
pub const THUMB_SIZE: u32 = 256;
/// Preview long edge (smaller images are kept at their size).
pub const PREVIEW_LONG_EDGE: u32 = 2048;

/// JPEG decoders happily return a partly grey picture for a cut-off file; an importer must not
/// accept that. Walk the file's own structure and require that the first image ends with an
/// end-of-image marker. Anything after it is fine: phones append gain maps, motion-photo videos
/// and vendor trailers (so "the file ends with FFD9" is not the test).
fn jpeg_is_complete(bytes: &[u8]) -> bool {
    let mut i = 2; // after SOI
    while i + 1 < bytes.len() {
        if bytes[i] != 0xFF {
            // Entropy-coded data between markers is handled below; here it means damage.
            return false;
        }
        let m = bytes[i + 1];
        match m {
            0xFF => i += 1,
            0xD9 => return true,
            0x01 | 0xD0..=0xD8 => i += 2,
            _ => {
                if i + 3 >= bytes.len() {
                    return false;
                }
                let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
                i += 2 + len;
                if m == 0xDA {
                    // Scan data: runs until a marker that is not a stuffed zero or a restart.
                    while i + 1 < bytes.len() {
                        if bytes[i] == 0xFF
                            && bytes[i + 1] != 0x00
                            && !(0xD0..=0xD7).contains(&bytes[i + 1])
                        {
                            break;
                        }
                        i += 1;
                    }
                }
            }
        }
    }
    false
}

pub fn decode(bytes: &[u8], format: ImageFormat) -> Result<DynamicImage, AnalyzeError> {
    if format == ImageFormat::Jpeg && !jpeg_is_complete(bytes) {
        return Err(AnalyzeError::Decode(
            "truncated JPEG (missing end-of-image marker)".into(),
        ));
    }
    image::load_from_memory_with_format(bytes, format)
        .map_err(|e| AnalyzeError::Decode(e.to_string()))
}

#[cfg(feature = "heic")]
pub fn decode_heic(bytes: &[u8]) -> Result<DynamicImage, AnalyzeError> {
    use libheif_rs::{ColorSpace, HeifContext, LibHeif, RgbChroma};
    let err = |e: libheif_rs::HeifError| AnalyzeError::Decode(e.to_string());
    let lib = LibHeif::new();
    let ctx = HeifContext::read_from_bytes(bytes).map_err(err)?;
    let handle = ctx.primary_image_handle().map_err(err)?;
    let img = lib
        .decode(&handle, ColorSpace::Rgb(RgbChroma::Rgb), None)
        .map_err(err)?;
    let plane = img
        .planes()
        .interleaved
        .ok_or_else(|| AnalyzeError::Decode("no interleaved plane".into()))?;
    let (w, h) = (plane.width, plane.height);
    let mut rgb = Vec::with_capacity((w * h * 3) as usize);
    for row in plane.data.chunks(plane.stride).take(h as usize) {
        rgb.extend_from_slice(&row[..(w * 3) as usize]);
    }
    image::RgbImage::from_raw(w, h, rgb)
        .map(DynamicImage::ImageRgb8)
        .ok_or_else(|| AnalyzeError::Decode("bad HEIC pixel buffer".into()))
}

#[cfg(not(feature = "heic"))]
pub fn decode_heic(_bytes: &[u8]) -> Result<DynamicImage, AnalyzeError> {
    Err(AnalyzeError::Unsupported(
        "HEIC support is not compiled in (enable the `heic` feature)".into(),
    ))
}

/// Apply an EXIF orientation (1–8) so the pixels are upright.
pub fn orient(mut image: DynamicImage, exif_orientation: u16) -> DynamicImage {
    if let Some(o) = Orientation::from_exif(exif_orientation as u8) {
        image.apply_orientation(o);
    }
    image
}

fn encode_jpeg(image: &DynamicImage, quality: u8) -> Result<Rendition, AnalyzeError> {
    let rgb = image.to_rgb8();
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, quality)
        .encode_image(&rgb)
        .map_err(|e| AnalyzeError::Decode(e.to_string()))?;
    Ok(Rendition {
        bytes,
        mime: "image/jpeg".into(),
        width: rgb.width(),
        height: rgb.height(),
    })
}

/// Center-cropped square thumbnail.
pub fn thumbnail(image: &DynamicImage) -> Result<Rendition, AnalyzeError> {
    encode_jpeg(
        &image.resize_to_fill(THUMB_SIZE, THUMB_SIZE, FilterType::Triangle),
        80,
    )
}

/// The thumbnail and the preview (long edge at most [`PREVIEW_LONG_EDGE`], never upscaled) of a
/// decoded picture that is not yet upright, and its upright size. The work is done on the
/// smallest picture that serves: the preview is cut from the full image and turned upright, the
/// thumbnail is cut from the preview. (For a 12 MP photo, cutting the thumbnail from the full
/// image cost 21 ms against 6, and turning it upright 12 ms against 4.)
pub fn renditions(
    image: DynamicImage,
    exif_orientation: u16,
) -> Result<(Rendition, Rendition, (u32, u32)), AnalyzeError> {
    let (w, h) = (image.width(), image.height());
    // Orientations 5 to 8 turn the picture a quarter.
    let upright = if (5..=8).contains(&exif_orientation) {
        (h, w)
    } else {
        (w, h)
    };
    let small = if w.max(h) > PREVIEW_LONG_EDGE {
        image.resize(PREVIEW_LONG_EDGE, PREVIEW_LONG_EDGE, FilterType::Triangle)
    } else {
        image
    };
    let small = orient(small, exif_orientation);
    Ok((thumbnail(&small)?, encode_jpeg(&small, 85)?, upright))
}
