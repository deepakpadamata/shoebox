//! Import pipeline (M1-04): `analyze(path)` hashes a file, reads its metadata and renders a
//! thumbnail and a preview. It is a pure function of the file — no node, no store.
//!
//! Formats: JPEG, PNG, WebP, GIF, BMP and TIFF (always; GIF and animated WebP by their first
//! frame), HEIC and AVIF (the platform's decoder, or libheif with feature `heic`), RAW via the embedded JPEG preview, and videos as metadata only
//! (poster frames are M3-10): MP4, MOV, M4V and 3GP from the `moov` box; AVI, MPEG and Matroska
//! from their headers. Anything else is reported as
//! [`AnalyzeError::Unsupported`]; the pipeline never panics on bad input.

pub mod archive;
mod detect;
mod exif_data;
mod motion;
mod raw;
mod render;
mod strip;
pub mod takeout;
mod video;

use shoebox_core::ids::{Blake3Hash, Timestamp};
use shoebox_core::model::ExifSummary;
use std::io::Read;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

pub use image::{DynamicImage, RgbImage};
pub use motion::{find_embedded as find_embedded_video, EmbeddedVideo};
pub use render::{set_platform_decoder, PlatformDecoder, PREVIEW_LONG_EDGE, THUMB_SIZE};
pub use strip::{strip_location, StripError, Stripped};

/// The picture in an image file (a preview, or an original shown as it is), at most `max_edge`
/// px on its long side and upright (EXIF orientation applied), as a viewer shows it.
pub fn decode_upright(bytes: &[u8], max_edge: u32) -> Result<image::DynamicImage, AnalyzeError> {
    let (image, orientation) = decode_with_orientation(bytes)?;
    Ok(render::upright(image, orientation, max_edge))
}

/// A square JPEG of `size` px around the face at `rect` (left, top, right, bottom as fractions of
/// the upright picture) in an image file: a preview, or an original shown as it is. A phone shows
/// faces this way instead of handing its webview whole originals (S1-19).
pub fn face_crop(bytes: &[u8], rect: [f32; 4], size: u32) -> Result<Rendition, AnalyzeError> {
    let (image, orientation) = decode_with_orientation(bytes)?;
    render::face_crop(image, orientation, rect, size)
}

/// The preview of a still picture, made again from its original: the same bytes the import made
/// (same decoder, same resize, same encoder), so the preview's hash in the library still matches.
pub fn preview_jpeg(bytes: &[u8]) -> Result<Vec<u8>, AnalyzeError> {
    let (image, orientation) = decode_with_orientation(bytes)?;
    let (_, preview, _) = render::renditions(image, orientation)?;
    Ok(preview.bytes)
}

fn decode_with_orientation(bytes: &[u8]) -> Result<(image::DynamicImage, u16), AnalyzeError> {
    let format = detect::sniff(bytes, None);
    decode_picture(bytes, &format, || exif_data::read(bytes).orientation)
}

/// The pixels of a still picture as stored, and the EXIF orientation still to apply to them.
fn decode_picture(
    bytes: &[u8],
    format: &detect::Format,
    exif_orientation: impl FnOnce() -> u16,
) -> Result<(image::DynamicImage, u16), AnalyzeError> {
    use detect::Format;
    // `true`: the stored pixels still need the EXIF orientation applied.
    let (image, exif) = match format {
        Format::Jpeg => (render::decode(bytes, image::ImageFormat::Jpeg)?, true),
        Format::Png => (render::decode(bytes, image::ImageFormat::Png)?, true),
        Format::Webp => (render::decode(bytes, image::ImageFormat::WebP)?, true),
        Format::Tiff => (render::decode(bytes, image::ImageFormat::Tiff)?, true),
        Format::Gif => (render::decode(bytes, image::ImageFormat::Gif)?, false),
        Format::Bmp => (render::decode(bytes, image::ImageFormat::Bmp)?, false),
        Format::Raw => (raw::embedded_preview(bytes)?, true),
        // libheif applies the file's own transforms (irot/imir); EXIF orientation must not be applied twice.
        Format::Heic | Format::Avif => (render::decode_heic(bytes)?, false),
        other => {
            return Err(AnalyzeError::Unsupported(format!(
                "not a picture: {other:?}"
            )))
        }
    };
    Ok((image, if exif { exif_orientation() } else { 1 }))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    Photo,
    Video,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TakenAtSource {
    /// From the file's own metadata (EXIF `DateTimeOriginal`, or the video container).
    Exif,
    /// The file's modification time, used when there is no embedded date.
    FileMtime,
}

/// A generated JPEG thumbnail or preview.
#[derive(Clone, Debug)]
pub struct Rendition {
    pub bytes: Vec<u8>,
    pub mime: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug)]
pub struct ImportResult {
    /// BLAKE3 of the original file; this is the photo's id.
    pub hash: Blake3Hash,
    pub size: u64,
    pub mime: String,
    pub kind: MediaKind,
    pub taken_at: Timestamp,
    pub taken_at_source: TakenAtSource,
    /// Dimensions are as displayed (EXIF orientation applied).
    pub exif: ExifSummary,
    /// `None` for videos until poster frames exist.
    pub thumb: Option<Rendition>,
    pub preview: Option<Rendition>,
    /// Length of a video, when its container says.
    pub duration_ms: Option<u32>,
    /// The video inside a Samsung or Google motion photo (F2.10); the file is still the original.
    pub embedded_video: Option<EmbeddedVideo>,
}

#[derive(Debug, thiserror::Error)]
pub enum AnalyzeError {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("unsupported file: {0}")]
    Unsupported(String),
    #[error("could not decode: {0}")]
    Decode(String),
}

/// Stream-hash a file without loading it into memory.
fn hash_file(path: &Path) -> Result<(Blake3Hash, u64), AnalyzeError> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut size = 0u64;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        size += n as u64;
    }
    Ok((Blake3Hash(*hasher.finalize().as_bytes()), size))
}

fn mtime_ms(path: &Path) -> Result<i64, AnalyzeError> {
    let modified = std::fs::metadata(path)?.modified()?;
    Ok(match modified.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(e) => -(e.duration().as_millis() as i64),
    })
}

/// Analyze one file. See the module docs for what is supported.
pub fn analyze(path: &Path) -> Result<ImportResult, AnalyzeError> {
    // A malformed file must surface as an error even if a decoder inside panics.
    match catch_unwind(AssertUnwindSafe(|| analyze_inner(path))) {
        Ok(result) => result,
        Err(_) => Err(AnalyzeError::Decode(
            "decoder panicked on malformed input".into(),
        )),
    }
}

fn analyze_inner(path: &Path) -> Result<ImportResult, AnalyzeError> {
    if path.is_dir() {
        return Err(AnalyzeError::Unsupported("is a directory".into()));
    }
    let mut head = Vec::with_capacity(detect::HEAD_LEN);
    std::fs::File::open(path)?
        .take(detect::HEAD_LEN as u64)
        .read_to_end(&mut head)?;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    let format = detect::sniff(&head, ext.as_deref());

    match format {
        video if video.is_video() => {
            let (hash, size) = hash_file(path)?;
            let meta = match video {
                detect::Format::Mp4 { .. } | detect::Format::Mov => video::read_metadata(path)?,
                ref other => video::read_other_metadata(path, other)?,
            };
            let (taken_at, source) = match meta.created_unix_secs {
                Some(s) => (Timestamp(s * 1000), TakenAtSource::Exif),
                None => (Timestamp(mtime_ms(path)?), TakenAtSource::FileMtime),
            };
            let exif = ExifSummary {
                width: meta.width,
                height: meta.height,
                ..Default::default()
            };
            Ok(ImportResult {
                hash,
                size,
                mime: video.mime().into(),
                kind: MediaKind::Video,
                taken_at,
                taken_at_source: source,
                exif,
                thumb: None,
                preview: None,
                duration_ms: meta.duration_ms,
                embedded_video: None,
            })
        }
        detect::Format::Unknown(why) => Err(AnalyzeError::Unsupported(why)),
        photo => {
            let bytes = std::fs::read(path)?;
            let hash = Blake3Hash(*blake3::hash(&bytes).as_bytes());
            let meta = exif_data::read(&bytes);
            let (image, orientation) = decode_picture(&bytes, &photo, || meta.orientation)?;
            let (thumb, preview, (width, height)) = render::renditions(image, orientation)?;
            let (taken_at, source) = match meta.taken_at_ms {
                Some(ms) => (Timestamp(ms), TakenAtSource::Exif),
                None => (Timestamp(mtime_ms(path)?), TakenAtSource::FileMtime),
            };
            let exif = ExifSummary {
                camera_make: meta.make,
                camera_model: meta.model,
                lens: meta.lens,
                width,
                height,
                gps: meta.gps,
            };
            // For RAW the "display" size is the preview's; the sensor size is not in the summary.
            Ok(ImportResult {
                hash,
                size: bytes.len() as u64,
                mime: photo.mime().into(),
                kind: MediaKind::Photo,
                taken_at,
                taken_at_source: source,
                exif,
                thumb: Some(thumb),
                preview: Some(preview),
                duration_ms: None,
                embedded_video: (photo == detect::Format::Jpeg)
                    .then(|| motion::find_embedded(&bytes))
                    .flatten(),
            })
        }
    }
}
