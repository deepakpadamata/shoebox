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
/// accept that, so require the end-of-image marker (ignoring trailing zero padding).
fn jpeg_is_complete(bytes: &[u8]) -> bool {
    let end = bytes.iter().rposition(|b| *b != 0).map_or(0, |i| i + 1);
    bytes[..end].ends_with(&[0xFF, 0xD9])
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

/// Long edge at most [`PREVIEW_LONG_EDGE`]; never upscaled.
pub fn preview(image: &DynamicImage) -> Result<Rendition, AnalyzeError> {
    if image.width().max(image.height()) > PREVIEW_LONG_EDGE {
        encode_jpeg(
            &image.resize(PREVIEW_LONG_EDGE, PREVIEW_LONG_EDGE, FilterType::Triangle),
            85,
        )
    } else {
        encode_jpeg(image, 85)
    }
}
