//! Import pipeline (M1-04): `analyze(path)` hashes a file, reads its metadata and renders a
//! thumbnail and a preview. It is a pure function of the file — no node, no store.
//!
//! Formats: JPEG and PNG (always), HEIC (feature `heic`), RAW via the embedded JPEG preview,
//! and MP4/MOV as metadata only (poster frames are M3-10). Anything else is reported as
//! [`AnalyzeError::Unsupported`]; the pipeline never panics on bad input.

mod detect;
mod exif_data;
mod raw;
mod render;
mod strip;
mod video;

use shoebox_core::ids::{Blake3Hash, Timestamp};
use shoebox_core::model::ExifSummary;
use std::io::Read;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

pub use render::{PREVIEW_LONG_EDGE, THUMB_SIZE};
pub use strip::{strip_location, StripError, Stripped};

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
    let mut head = [0u8; 16];
    let head_len = std::fs::File::open(path)?.read(&mut head)?;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    let format = detect::sniff(&head[..head_len], ext.as_deref());

    match format {
        detect::Format::Mp4 | detect::Format::Mov => {
            let (hash, size) = hash_file(path)?;
            let meta = video::read_metadata(path)?;
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
                mime: format.mime().into(),
                kind: MediaKind::Video,
                taken_at,
                taken_at_source: source,
                exif,
                thumb: None,
                preview: None,
            })
        }
        detect::Format::Unknown(why) => Err(AnalyzeError::Unsupported(why)),
        photo => {
            let bytes = std::fs::read(path)?;
            let hash = Blake3Hash(*blake3::hash(&bytes).as_bytes());
            let meta = exif_data::read(&bytes);
            let (image, orientation) = match photo {
                detect::Format::Jpeg => (
                    render::decode(&bytes, image::ImageFormat::Jpeg)?,
                    meta.orientation,
                ),
                detect::Format::Png => (
                    render::decode(&bytes, image::ImageFormat::Png)?,
                    meta.orientation,
                ),
                detect::Format::Raw => (raw::embedded_preview(&bytes)?, meta.orientation),
                detect::Format::Heic => (render::decode_heic(&bytes)?, 1),
                _ => unreachable!("videos and unknown handled above"),
            };
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
            })
        }
    }
}
