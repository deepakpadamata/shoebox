//! MP4/MOV metadata (creation time and frame size) from the `moov` box, without decoding.

use crate::AnalyzeError;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

#[derive(Debug, Default)]
pub struct VideoMeta {
    /// Unix seconds; `None` when the container carries no (or a zero) creation time.
    pub created_unix_secs: Option<i64>,
    pub width: u32,
    pub height: u32,
}

/// Seconds between the MP4 epoch (1904-01-01) and the Unix epoch.
const MP4_TO_UNIX: i64 = 2_082_844_800;
/// Refuse absurd `moov` boxes rather than allocating gigabytes for a corrupt size field.
const MAX_MOOV: u64 = 64 * 1024 * 1024;

/// Iterate `(type, body)` over the boxes in `data`; stops quietly at malformed sizes.
fn boxes(data: &[u8]) -> impl Iterator<Item = ([u8; 4], &[u8])> {
    let mut pos = 0usize;
    std::iter::from_fn(move || {
        let rest = data.get(pos..)?;
        if rest.len() < 8 {
            return None;
        }
        let size32 = u32::from_be_bytes(rest[0..4].try_into().ok()?) as u64;
        let kind: [u8; 4] = rest[4..8].try_into().ok()?;
        let (header, size) = match size32 {
            0 => (8, rest.len() as u64),
            1 => (16, u64::from_be_bytes(rest.get(8..16)?.try_into().ok()?)),
            n => (8, n),
        };
        if size < header || size > rest.len() as u64 {
            return None;
        }
        pos += size as usize;
        Some((kind, &rest[header as usize..size as usize]))
    })
}

fn find<'a>(data: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    boxes(data).find(|(k, _)| k == kind).map(|(_, b)| b)
}

fn mvhd_created(body: &[u8]) -> Option<i64> {
    let created = match body.first()? {
        0 => i64::from(u32::from_be_bytes(body.get(4..8)?.try_into().ok()?)),
        1 => i64::try_from(u64::from_be_bytes(body.get(4..12)?.try_into().ok()?)).ok()?,
        _ => return None,
    };
    (created > 0).then(|| created - MP4_TO_UNIX)
}

fn tkhd_size(body: &[u8]) -> Option<(u32, u32)> {
    // Width and height are the last two 16.16 fixed-point values of the box.
    let version_skip = if *body.first()? == 1 {
        4 + 8 + 8 + 4 + 4 + 8
    } else {
        4 + 4 + 4 + 4 + 4 + 4
    };
    let tail = body.get(version_skip + 8 + 2 + 2 + 2 + 2 + 36..)?;
    let w = u32::from_be_bytes(tail.get(0..4)?.try_into().ok()?) >> 16;
    let h = u32::from_be_bytes(tail.get(4..8)?.try_into().ok()?) >> 16;
    Some((w, h))
}

fn parse_moov(moov: &[u8]) -> VideoMeta {
    let created = find(moov, b"mvhd").and_then(mvhd_created);
    let (width, height) = boxes(moov)
        .filter(|(k, _)| k == b"trak")
        .filter_map(|(_, trak)| find(trak, b"tkhd").and_then(tkhd_size))
        .find(|(w, h)| *w > 0 && *h > 0)
        .unwrap_or((0, 0));
    VideoMeta {
        created_unix_secs: created,
        width,
        height,
    }
}

pub fn read_metadata(path: &Path) -> Result<VideoMeta, AnalyzeError> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    let mut pos = 0u64;
    // Walk top-level boxes by seeking, so a multi-GB `mdat` is never read.
    while pos + 8 <= len {
        file.seek(SeekFrom::Start(pos))?;
        let mut header = [0u8; 16];
        let got = file.read(&mut header)?;
        if got < 8 {
            break;
        }
        let size32 = u64::from(u32::from_be_bytes(
            header[0..4].try_into().expect("4 bytes"),
        ));
        let kind = &header[4..8];
        let (hdr, size) = match size32 {
            0 => (8, len - pos),
            1 if got >= 16 => (
                16,
                u64::from_be_bytes(header[8..16].try_into().expect("8 bytes")),
            ),
            n => (8, n),
        };
        if size < hdr || pos + size > len {
            return Err(AnalyzeError::Decode("truncated or corrupt MP4 box".into()));
        }
        if kind == b"moov" {
            if size - hdr > MAX_MOOV {
                return Err(AnalyzeError::Decode("implausibly large moov box".into()));
            }
            let mut body = vec![0u8; (size - hdr) as usize];
            file.seek(SeekFrom::Start(pos + hdr))?;
            file.read_exact(&mut body)?;
            return Ok(parse_moov(&body));
        }
        pos += size;
    }
    Err(AnalyzeError::Decode(
        "no moov box (not a complete MP4/MOV file)".into(),
    ))
}
