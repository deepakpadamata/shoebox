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

/// Samsung writes this tag right before the motion photo's video. Some of its files carry another
/// short clip (0.6 s, no sound) ahead of the tag: the video the camera shows is the one after it.
const SAMSUNG_TAG: &[u8] = b"MotionPhoto_Data";

/// The video appended to a JPEG, if there is one: the one after Samsung's tag when the file has
/// it, else the first one.
pub fn find_embedded(jpeg: &[u8]) -> Option<EmbeddedVideo> {
    let end = render::jpeg_image_end(jpeg)?;
    let tail = jpeg.get(end..).filter(|t| t.len() >= MIN_TRAILER)?;
    if let Some(tag) = tail
        .windows(SAMSUNG_TAG.len())
        .position(|w| w == SAMSUNG_TAG)
    {
        let from = tag + SAMSUNG_TAG.len();
        if let Some(found) = first_mp4(&tail[from..], end + from) {
            return Some(found);
        }
    }
    first_mp4(tail, end)
}

/// The first whole MP4 in `tail`, which starts at byte `base` of the file.
fn first_mp4(tail: &[u8], base: usize) -> Option<EmbeddedVideo> {
    let mut from = 0;
    while let Some(i) = tail[from..].windows(4).position(|w| w == b"ftyp") {
        let at = from + i;
        if let Some(start) = at.checked_sub(4) {
            let size = u32::from_be_bytes(tail[start..start + 4].try_into().expect("4 bytes"));
            if FTYP_SIZES.contains(&(size as usize)) {
                if let Some((len, meta)) = video::mp4_extent(&tail[start..]) {
                    return Some(EmbeddedVideo {
                        offset: (base + start) as u64,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn samples() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/samples")
    }

    #[test]
    fn samsungs_video_is_the_one_after_its_tag_not_a_clip_ahead_of_it() {
        let jpeg = std::fs::read(samples().join("IMG_0001.jpg")).unwrap();
        let clip = std::fs::read(samples().join("clip.mp4")).unwrap();
        // A picture, a first clip, a gap, the tag, the camera's video.
        let mut file = jpeg.clone();
        file.extend_from_slice(&clip);
        file.extend_from_slice(&[0u8; 8]);
        file.extend_from_slice(SAMSUNG_TAG);
        let second = file.len();
        file.extend_from_slice(&clip);

        let found = find_embedded(&file).expect("a video");
        assert_eq!(found.offset as usize, second);
        assert_eq!(found.len as usize, clip.len());

        // Without the tag, the first clip is the video (Pixel and older files).
        let mut plain = jpeg.clone();
        plain.extend_from_slice(&clip);
        let found = find_embedded(&plain).expect("a video");
        assert_eq!(found.offset as usize, jpeg.len());
    }
}
