//! Motion photos (F2.10): Samsung and Google cameras write a JPEG with the video appended after
//! the picture. The picture is an ordinary JPEG (a viewer that knows nothing shows it); the video
//! is found by structure, not by the vendors' XMP (which differs between Pixel, Samsung and their
//! versions): after the first image ends, an MP4 starts at the first `ftyp` box that is followed by
//! a whole movie.

use crate::{render, video};

/// A video inside a still's file: `len` bytes from `offset`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EmbeddedVideo {
    pub offset: u64,
    pub len: u64,
    pub duration_ms: u32,
}

/// Anything appended that is smaller than this holds no video (gain maps and vendor trailers do).
const MIN_TRAILER: usize = 1024;
/// A `ftyp` box is a brand and a few compatible brands.
const FTYP_SIZES: std::ops::RangeInclusive<usize> = 16..=1024;

/// The video appended to a JPEG, if there is one.
pub fn find_embedded(jpeg: &[u8]) -> Option<EmbeddedVideo> {
    let end = render::jpeg_image_end(jpeg)?;
    let tail = jpeg.get(end..).filter(|t| t.len() >= MIN_TRAILER)?;
    let mut from = 0;
    while let Some(i) = tail[from..].windows(4).position(|w| w == b"ftyp") {
        let at = from + i;
        if let Some(start) = at.checked_sub(4) {
            let size = u32::from_be_bytes(tail[start..start + 4].try_into().expect("4 bytes"));
            if FTYP_SIZES.contains(&(size as usize)) {
                if let Some((len, meta)) = video::mp4_extent(&tail[start..]) {
                    return Some(EmbeddedVideo {
                        offset: (end + start) as u64,
                        len: len as u64,
                        duration_ms: meta.duration_ms.unwrap_or(0),
                    });
                }
            }
        }
        from = at + 4;
    }
    None
}
