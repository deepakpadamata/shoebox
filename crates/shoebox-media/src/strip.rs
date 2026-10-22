//! Remove location metadata from a file without re-encoding it (M5-03).
//!
//! Only metadata bytes change: pixel data (JPEG scans, PNG image chunks, video samples) is
//! copied untouched. Removed regions are zeroed or dropped, so the coordinates are gone from the
//! bytes, not just hidden. Supported: JPEG (EXIF GPS IFD, XMP `exif:GPS*`), PNG (`eXIf`, XMP),
//! MP4/MOV (`©xyz`, `loci`, QuickTime location keys). HEIC and anything else is refused rather
//! than shared with its location.

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum StripError {
    #[error("removing the location from this kind of file is not supported yet ({0})")]
    Unsupported(String),
    #[error("the file is damaged: {0}")]
    Malformed(String),
}

/// The file without its location, and whether anything was found to remove.
#[derive(Debug, PartialEq, Eq)]
pub struct Stripped {
    pub bytes: Vec<u8>,
    pub changed: bool,
}

pub fn strip_location(data: &[u8]) -> Result<Stripped, StripError> {
    if data.starts_with(&[0xFF, 0xD8]) {
        jpeg(data)
    } else if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        png(data)
    } else if data.len() >= 12 && &data[4..8] == b"ftyp" {
        match &data[8..12] {
            b"heic" | b"heix" | b"hevc" | b"hevx" | b"mif1" | b"msf1" | b"heim" | b"heis" | b"avif" => {
                Err(StripError::Unsupported("HEIC/HEIF/AVIF".into()))
            }
            _ => mp4(data),
        }
    } else {
        Err(StripError::Unsupported("unknown format".into()))
    }
}

fn bad(what: &str) -> StripError {
    StripError::Malformed(what.into())
}

// ---------- TIFF / EXIF ----------

struct Tiff<'a> {
    d: &'a mut [u8],
    le: bool,
}

impl Tiff<'_> {
    fn u16(&self, o: usize) -> Option<u16> {
        let b: [u8; 2] = self.d.get(o..o + 2)?.try_into().ok()?;
        Some(if self.le { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) })
    }
    fn u32(&self, o: usize) -> Option<u32> {
        let b: [u8; 4] = self.d.get(o..o + 4)?.try_into().ok()?;
        Some(if self.le { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
    }
    fn zero(&mut self, from: usize, len: usize) -> Option<()> {
        self.d.get_mut(from..from.checked_add(len)?)?.fill(0);
        Some(())
    }
}

fn type_size(t: u16) -> usize {
    match t {
        1 | 2 | 6 | 7 => 1,
        3 | 8 => 2,
        4 | 9 | 11 => 4,
        5 | 10 | 12 => 8,
        _ => 1,
    }
}

/// Zero the GPS IFD (and its out-of-line values) and take its pointer out of IFD0.
/// `Ok(false)` if there was none.
fn strip_exif_gps(tiff: &mut [u8]) -> Result<bool, StripError> {
    if tiff.len() < 8 {
        return Err(bad("EXIF is too short"));
    }
    let le = match &tiff[..2] {
        b"II" => true,
        b"MM" => false,
        _ => return Err(bad("EXIF byte order")),
    };
    let mut t = Tiff { d: tiff, le };
    let ifd0 = t.u32(4).ok_or_else(|| bad("EXIF header"))? as usize;
    let count = t.u16(ifd0).ok_or_else(|| bad("IFD0"))? as usize;
    let mut at = None;
    for i in 0..count {
        let e = ifd0 + 2 + i * 12;
        if t.u16(e).ok_or_else(|| bad("IFD0 entry"))? == 0x8825 {
            at = Some((i, e));
            break;
        }
    }
    let Some((index, entry)) = at else { return Ok(false) };
    let gps = t.u32(entry + 8).ok_or_else(|| bad("GPS pointer"))? as usize;
    // Wipe the GPS IFD: its values first, then the entries.
    if let Some(n) = t.u16(gps).map(|n| n as usize).filter(|_| gps != 0) {
        for i in 0..n {
            let e = gps + 2 + i * 12;
            let (ty, cnt) = (
                t.u16(e + 2).ok_or_else(|| bad("GPS entry"))?,
                t.u32(e + 4).ok_or_else(|| bad("GPS entry"))? as usize,
            );
            let size = type_size(ty).saturating_mul(cnt);
            if size > 4 {
                let off = t.u32(e + 8).ok_or_else(|| bad("GPS value"))? as usize;
                t.zero(off, size).ok_or_else(|| bad("GPS value out of range"))?;
            }
        }
        t.zero(gps, 2 + n * 12).ok_or_else(|| bad("GPS IFD out of range"))?;
    }
    // Remove the pointer entry from IFD0: shift the rest up, keep the next-IFD offset in place.
    let first_after = entry + 12;
    let next_at = ifd0 + 2 + count * 12;
    let next: Vec<u8> = t.d.get(next_at..next_at + 4).ok_or_else(|| bad("IFD0 end"))?.to_vec();
    let tail: Vec<u8> = t.d[first_after..next_at].to_vec();
    t.d[entry..entry + tail.len()].copy_from_slice(&tail);
    let new_next = ifd0 + 2 + (count - 1) * 12;
    t.d[new_next..new_next + 4].copy_from_slice(&next);
    t.d[new_next + 4..new_next + 16].fill(0);
    let n = (count - 1) as u16;
    let bytes = if le { n.to_le_bytes() } else { n.to_be_bytes() };
    t.d[ifd0..ifd0 + 2].copy_from_slice(&bytes);
    let _ = index;
    Ok(true)
}

// ---------- XMP ----------

/// Blank out every `exif:GPS*` property (attribute or element) in place.
fn strip_xmp_gps(xmp: &mut [u8]) -> bool {
    let needle = b"exif:GPS";
    let mut changed = false;
    let mut i = 0;
    while i + needle.len() < xmp.len() {
        if &xmp[i..i + needle.len()] != needle {
            i += 1;
            continue;
        }
        let name_end = xmp[i..]
            .iter()
            .position(|b| matches!(b, b'=' | b'>' | b' ' | b'/'))
            .map(|p| i + p)
            .unwrap_or(xmp.len());
        let element = i > 0 && xmp[i - 1] == b'<';
        let end = if element {
            let close: Vec<u8> = [b"</".as_slice(), &xmp[i..name_end], b">"].concat();
            xmp[name_end..]
                .windows(close.len())
                .position(|w| w == close.as_slice())
                .map(|p| name_end + p + close.len())
        } else if xmp.get(name_end) == Some(&b'=') {
            let quote = xmp.get(name_end + 1).copied().unwrap_or(b'"');
            xmp[name_end + 2..]
                .iter()
                .position(|b| *b == quote)
                .map(|p| name_end + 2 + p + 1)
        } else {
            None
        };
        match end {
            Some(end) => {
                let start = if element { i - 1 } else { i };
                xmp[start..end].fill(b' ');
                changed = true;
                i = end;
            }
            None => i = name_end.max(i + 1),
        }
    }
    changed
}

// ---------- JPEG ----------

fn jpeg(data: &[u8]) -> Result<Stripped, StripError> {
    let mut out = data.to_vec();
    let mut changed = false;
    let mut pos = 2;
    while pos + 4 <= out.len() {
        if out[pos] != 0xFF {
            return Err(bad("JPEG marker"));
        }
        let marker = out[pos + 1];
        if marker == 0xFF {
            pos += 1;
            continue;
        }
        if marker == 0xDA || marker == 0xD9 {
            break; // the image data follows: untouched
        }
        if (0xD0..=0xD7).contains(&marker) || marker == 0x01 {
            pos += 2;
            continue;
        }
        let len = u16::from_be_bytes([out[pos + 2], out[pos + 3]]) as usize;
        if len < 2 || pos + 2 + len > out.len() {
            return Err(bad("JPEG segment length"));
        }
        let body = pos + 4..pos + 2 + len;
        if marker == 0xE1 {
            let seg = &mut out[body.clone()];
            if seg.starts_with(b"Exif\0\0") {
                changed |= strip_exif_gps(&mut seg[6..])?;
            } else if seg.starts_with(b"http://ns.adobe.com/xap/1.0/\0") {
                changed |= strip_xmp_gps(seg);
            }
        }
        pos += 2 + len;
    }
    Ok(Stripped { bytes: out, changed })
}

// ---------- PNG ----------

fn png(data: &[u8]) -> Result<Stripped, StripError> {
    let mut out = data[..8].to_vec();
    let mut changed = false;
    let mut pos = 8;
    while pos + 12 <= data.len() {
        let len = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        let end = pos + 12 + len;
        if end > data.len() {
            return Err(bad("PNG chunk length"));
        }
        let kind = &data[pos + 4..pos + 8];
        let body = &data[pos + 8..pos + 8 + len];
        let drop = match kind {
            b"eXIf" => {
                // Keep a chunk that has no location, drop the rest.
                let mut copy = body.to_vec();
                strip_exif_gps(&mut copy).unwrap_or(true)
            }
            b"iTXt" | b"tEXt" | b"zTXt" => {
                body.starts_with(b"XML:com.adobe.xmp") && {
                    let mut copy = body.to_vec();
                    // Compressed XMP cannot be inspected: treat it as possibly carrying GPS.
                    kind == b"zTXt" || body.get(18) == Some(&1) || strip_xmp_gps(&mut copy)
                }
            }
            _ => false,
        };
        if drop {
            changed = true;
        } else {
            out.extend_from_slice(&data[pos..end]);
        }
        pos = end;
        if kind == b"IEND" {
            break;
        }
    }
    out.extend_from_slice(&data[pos..]);
    Ok(Stripped { bytes: out, changed })
}

// ---------- MP4 / MOV ----------

fn be32(d: &[u8], o: usize) -> Option<usize> {
    Some(u32::from_be_bytes(d.get(o..o + 4)?.try_into().ok()?) as usize)
}

fn free_box(d: &mut [u8], start: usize, end: usize) {
    d[start + 4..start + 8].copy_from_slice(b"free");
    d[start + 8..end].fill(0);
}

const LOCATION_KEY: &[u8] = b"com.apple.quicktime.location.ISO6709";

fn mp4_walk(d: &mut [u8], from: usize, to: usize, changed: &mut bool) -> Result<(), StripError> {
    let mut pos = from;
    while pos + 8 <= to {
        let mut size = be32(d, pos).ok_or_else(|| bad("box size"))?;
        let header = if size == 1 { 16 } else { 8 };
        if size == 0 {
            size = to - pos;
        } else if size == 1 {
            size = u64::from_be_bytes(d.get(pos + 8..pos + 16).ok_or_else(|| bad("box size"))?.try_into().unwrap()) as usize;
        }
        if size < header || pos + size > to {
            return Err(bad("box does not fit"));
        }
        let end = pos + size;
        let kind: [u8; 4] = d[pos + 4..pos + 8].try_into().unwrap();
        match &kind {
            [0xA9, b'x', b'y', b'z'] | b"loci" | b"xyz " => {
                free_box(d, pos, end);
                *changed = true;
            }
            b"moov" | b"udta" | b"trak" | b"mdia" | b"minf" | b"stbl" => {
                mp4_walk(d, pos + header, end, changed)?;
            }
            b"meta" => {
                // ISO `meta` is a full box (4 bytes of version and flags); QuickTime's is not.
                let inner = if d.get(pos + header + 8..pos + header + 12) == Some(b"hdlr") {
                    pos + header
                } else {
                    pos + header + 4
                };
                // Keys first, so that the matching `ilst` item can be found.
                let mut index = None;
                let mut p = inner;
                while p + 8 <= end {
                    let s = be32(d, p).ok_or_else(|| bad("meta box"))?;
                    if s < 8 || p + s > end {
                        break;
                    }
                    if &d[p + 4..p + 8] == b"keys" {
                        let count = be32(d, p + 12).unwrap_or(0);
                        let mut k = p + 16;
                        for n in 1..=count {
                            let ks = be32(d, k).unwrap_or(0);
                            if ks < 8 || k + ks > p + s {
                                break;
                            }
                            if &d[k + 8..k + ks] == LOCATION_KEY {
                                index = Some(n);
                            }
                            k += ks;
                        }
                    }
                    p += s;
                }
                if let Some(n) = index {
                    let mut p = inner;
                    while p + 8 <= end {
                        let s = be32(d, p).ok_or_else(|| bad("meta box"))?;
                        if s < 8 || p + s > end {
                            break;
                        }
                        if &d[p + 4..p + 8] == b"ilst" {
                            let mut q = p + 8;
                            while q + 8 <= p + s {
                                let qs = be32(d, q).ok_or_else(|| bad("ilst item"))?;
                                if qs < 8 || q + qs > p + s {
                                    break;
                                }
                                if be32(d, q + 4) == Some(n) {
                                    free_box(d, q, q + qs);
                                    *changed = true;
                                }
                                q += qs;
                            }
                        }
                        p += s;
                    }
                }
            }
            _ => {}
        }
        pos = end;
    }
    Ok(())
}

fn mp4(data: &[u8]) -> Result<Stripped, StripError> {
    let mut out = data.to_vec();
    let mut changed = false;
    let len = out.len();
    mp4_walk(&mut out, 0, len, &mut changed)?;
    Ok(Stripped { bytes: out, changed })
}
