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
    /// Length of the movie; `None` when the container does not say.
    pub duration_ms: Option<u32>,
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

/// `(timescale, duration)` of the `mvhd` box, as milliseconds.
fn mvhd_duration_ms(body: &[u8]) -> Option<u32> {
    let (timescale, duration) = match body.first()? {
        0 => (
            u64::from(u32::from_be_bytes(body.get(12..16)?.try_into().ok()?)),
            u64::from(u32::from_be_bytes(body.get(16..20)?.try_into().ok()?)),
        ),
        1 => (
            u64::from(u32::from_be_bytes(body.get(20..24)?.try_into().ok()?)),
            u64::from_be_bytes(body.get(24..32)?.try_into().ok()?),
        ),
        _ => return None,
    };
    (timescale > 0)
        .then(|| u32::try_from(duration.saturating_mul(1000) / timescale).unwrap_or(u32::MAX))
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
        duration_ms: find(moov, b"mvhd").and_then(mvhd_duration_ms),
    }
}

/// How many bytes at the start of `data` are complete top-level MP4/MOV boxes, and what the movie's
/// `moov` says, provided they include the movie header and media data (`mdat` or `moof`): the
/// test that `data` begins with a whole video, which may be followed by other things.
pub fn mp4_extent(data: &[u8]) -> Option<(usize, VideoMeta)> {
    let (mut pos, mut meta, mut media) = (0usize, None, false);
    while let Some(rest) = data.get(pos..).filter(|r| r.len() >= 8) {
        let size32 = u32::from_be_bytes(rest[0..4].try_into().ok()?) as u64;
        let kind: [u8; 4] = rest[4..8].try_into().ok()?;
        let (header, size) = match size32 {
            0 => (8, rest.len() as u64),
            1 => (16, u64::from_be_bytes(rest.get(8..16)?.try_into().ok()?)),
            n => (8, n),
        };
        if size < header || size > rest.len() as u64 {
            break;
        }
        let body = &rest[header as usize..size as usize];
        match &kind {
            b"moov" if size - header <= MAX_MOOV => meta = Some(parse_moov(body)),
            b"mdat" | b"moof" => media = true,
            b"ftyp" | b"free" | b"skip" | b"wide" | b"uuid" | b"meta" | b"mfra" | b"sidx" => {}
            _ => break,
        }
        pos += size as usize;
    }
    meta.filter(|_| media).map(|m| (pos, m))
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

/// How much of an AVI, MPEG or Matroska file is read for its headers: they come first.
const HEAD_READ: u64 = 1024 * 1024;

fn read_head(path: &Path) -> Result<Vec<u8>, AnalyzeError> {
    let mut head = Vec::new();
    File::open(path)?.take(HEAD_READ).read_to_end(&mut head)?;
    Ok(head)
}

/// Iterate `(fourcc, body)` over RIFF chunks; a `LIST` yields its list type and the rest.
fn riff_chunks(data: &[u8]) -> impl Iterator<Item = ([u8; 4], &[u8])> {
    let mut pos = 0usize;
    std::iter::from_fn(move || {
        let rest = data.get(pos..)?;
        let kind: [u8; 4] = rest.get(0..4)?.try_into().ok()?;
        let size = u32::from_le_bytes(rest.get(4..8)?.try_into().ok()?) as usize;
        // A chunk cut off by the head read still yields what is there.
        let body = &rest[8..(8 + size).min(rest.len())];
        pos += 8 + size + (size & 1);
        Some((kind, body))
    })
}

/// `IDIT` in ctime form ("THU OCT 22 13:22:11 2009") or ISO form, and `ICRD` ("2009-10-22").
fn riff_date(text: &[u8]) -> Option<i64> {
    let text = String::from_utf8_lossy(text);
    let text = text.trim_matches(['\0', '\n', '\r', ' ']);
    let parts: Vec<&str> = text.split_whitespace().collect();
    const MONTHS: [&str; 12] = [
        "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
    ];
    let (y, m, d, hms) = if parts.len() == 5 {
        let m = MONTHS
            .iter()
            .position(|x| parts[1].eq_ignore_ascii_case(x))? as i64
            + 1;
        (parts[4].parse().ok()?, m, parts[2].parse().ok()?, parts[3])
    } else {
        let (date, time) = text.split_once([' ', 'T']).unwrap_or((text, "00:00:00"));
        let mut ymd = date.split(['-', ':', '/']).map(|x| x.parse::<i64>().ok());
        (ymd.next()??, ymd.next()??, ymd.next()??, time)
    };
    let mut t = hms.split(':').map(|x| x.parse::<i64>().ok());
    let (hh, mm, ss) = (
        t.next().flatten().unwrap_or(0),
        t.next().flatten().unwrap_or(0),
        t.next().flatten().unwrap_or(0),
    );
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || y < 1970 {
        return None;
    }
    Some(crate::exif_data::days_from_civil(y, m, d) * 86_400 + hh * 3600 + mm * 60 + ss)
}

fn parse_avi(data: &[u8]) -> VideoMeta {
    // `hdrl` holds `avih` (frame size) and maybe `IDIT`; `INFO` may hold `ICRD`. `movi` is skipped.
    fn walk(chunks: &[u8], meta: &mut VideoMeta) {
        for (kind, body) in riff_chunks(chunks) {
            match &kind {
                b"LIST" if body.len() >= 4 && &body[0..4] != b"movi" => walk(&body[4..], meta),
                b"avih" if body.len() >= 40 => {
                    meta.width = u32::from_le_bytes(body[32..36].try_into().expect("4 bytes"));
                    meta.height = u32::from_le_bytes(body[36..40].try_into().expect("4 bytes"));
                }
                b"IDIT" | b"ICRD" => {
                    meta.created_unix_secs = meta.created_unix_secs.or_else(|| riff_date(body));
                }
                _ => {}
            }
        }
    }
    let mut meta = VideoMeta::default();
    if let Some(body) = data.get(12..) {
        walk(body, &mut meta);
    }
    meta
}

/// The first MPEG-1/2 sequence header's frame size; program streams carry no date.
fn parse_mpeg(data: &[u8]) -> VideoMeta {
    let mut meta = VideoMeta::default();
    if let Some(i) = data.windows(4).position(|w| w == [0x00, 0x00, 0x01, 0xB3]) {
        if let Some(b) = data.get(i + 4..i + 7) {
            meta.width = (u32::from(b[0]) << 4) | (u32::from(b[1]) >> 4);
            meta.height = ((u32::from(b[1]) & 0x0F) << 8) | u32::from(b[2]);
        }
    }
    meta
}

/// An EBML variable-length integer: `(value, length)`; `keep_marker` for element ids.
fn ebml_vint(data: &[u8], keep_marker: bool) -> Option<(u64, usize)> {
    let first = *data.first()?;
    let len = first.leading_zeros() as usize + 1;
    if len > 8 {
        return None;
    }
    let mut v = u64::from(if keep_marker {
        first
    } else {
        first & 0xFFu8.checked_shr(len as u32).unwrap_or(0)
    });
    for b in data.get(1..len)? {
        v = (v << 8) | u64::from(*b);
    }
    Some((v, len))
}

/// Iterate `(id, body)` over EBML elements; an unknown size (all ones) runs to the end.
fn ebml_elements(data: &[u8]) -> impl Iterator<Item = (u64, &[u8])> {
    let mut pos = 0usize;
    std::iter::from_fn(move || {
        let rest = data.get(pos..)?;
        let (id, id_len) = ebml_vint(rest, true)?;
        let (size, size_len) = ebml_vint(rest.get(id_len..)?, false)?;
        let start = id_len + size_len;
        let unknown = size == (1u64 << (7 * size_len)) - 1;
        let end = if unknown {
            rest.len()
        } else {
            start
                .saturating_add(usize::try_from(size).ok()?)
                .min(rest.len())
        };
        pos += end;
        Some((id, rest.get(start..end)?))
    })
}

fn ebml_uint(body: &[u8]) -> u64 {
    body.iter().take(8).fold(0, |v, b| (v << 8) | u64::from(*b))
}

/// Seconds between the Matroska epoch (2001-01-01) and the Unix epoch.
const MKV_TO_UNIX: i64 = 978_307_200;

fn parse_matroska(data: &[u8]) -> VideoMeta {
    const SEGMENT: u64 = 0x1853_8067;
    const INFO: u64 = 0x1549_A966;
    const DATE_UTC: u64 = 0x4461;
    const TRACKS: u64 = 0x1654_AE6B;
    const TRACK_ENTRY: u64 = 0xAE;
    const VIDEO: u64 = 0xE0;
    let mut meta = VideoMeta::default();
    let Some((_, segment)) = ebml_elements(data).find(|(id, _)| *id == SEGMENT) else {
        return meta;
    };
    for (id, body) in ebml_elements(segment) {
        match id {
            INFO => {
                if let Some((_, d)) = ebml_elements(body).find(|(id, _)| *id == DATE_UTC) {
                    let ns = ebml_uint(d) as i64;
                    meta.created_unix_secs = Some(ns.div_euclid(1_000_000_000) + MKV_TO_UNIX);
                }
            }
            TRACKS => {
                let video = ebml_elements(body)
                    .filter(|(id, _)| *id == TRACK_ENTRY)
                    .find_map(|(_, entry)| ebml_elements(entry).find(|(id, _)| *id == VIDEO));
                if let Some((_, video)) = video {
                    for (id, v) in ebml_elements(video) {
                        match id {
                            0xB0 => meta.width = ebml_uint(v) as u32,
                            0xBA => meta.height = ebml_uint(v) as u32,
                            _ => {}
                        }
                    }
                }
                break;
            }
            _ => {}
        }
    }
    meta
}

/// Metadata of the containers Shoebox keeps but does not parse as MP4: what their headers say.
pub fn read_other_metadata(
    path: &Path,
    format: &crate::detect::Format,
) -> Result<VideoMeta, AnalyzeError> {
    use crate::detect::Format;
    let head = read_head(path)?;
    Ok(match format {
        Format::Avi => parse_avi(&head),
        Format::Mpeg => parse_mpeg(&head),
        Format::Matroska { .. } => parse_matroska(&head),
        _ => VideoMeta::default(),
    })
}
