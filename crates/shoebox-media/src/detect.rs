//! Format detection by magic bytes (the extension only disambiguates TIFF-based RAW).

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Format {
    Jpeg,
    Png,
    Heic,
    /// Decoded wherever HEIC is.
    Avif,
    /// TIFF-based RAW (DNG, CR2, NEF, ARW, ...); handled through its embedded JPEG preview.
    Raw,
    /// First frame for the renditions; the original plays as it is.
    Gif,
    /// Still or animated (first frame).
    Webp,
    Bmp,
    Tiff,
    /// The ISO base media family; `mime` is the brand's (MP4, M4V, 3GP).
    Mp4 {
        mime: &'static str,
    },
    Mov,
    /// Containers read for metadata only (dimensions, date when they carry one).
    Avi,
    Mpeg,
    /// Matroska; `webm` when the EBML doc type says so.
    Matroska {
        webm: bool,
    },
    Unknown(String),
}

const RAW_EXTENSIONS: &[&str] = &[
    "dng", "cr2", "nef", "nrw", "arw", "srf", "sr2", "orf", "rw2", "raf", "pef", "srw", "3fr",
];

impl Format {
    pub fn mime(&self) -> &'static str {
        match self {
            Format::Jpeg => "image/jpeg",
            Format::Png => "image/png",
            Format::Heic => "image/heic",
            Format::Avif => "image/avif",
            Format::Raw => "image/x-adobe-dng",
            Format::Gif => "image/gif",
            Format::Webp => "image/webp",
            Format::Bmp => "image/bmp",
            Format::Tiff => "image/tiff",
            Format::Mp4 { mime } => mime,
            Format::Mov => "video/quicktime",
            Format::Avi => "video/x-msvideo",
            Format::Mpeg => "video/mpeg",
            Format::Matroska { webm: true } => "video/webm",
            Format::Matroska { webm: false } => "video/x-matroska",
            Format::Unknown(_) => "application/octet-stream",
        }
    }

    pub fn is_video(&self) -> bool {
        matches!(
            self,
            Format::Mp4 { .. } | Format::Mov | Format::Avi | Format::Mpeg | Format::Matroska { .. }
        )
    }
}

/// How many leading bytes [`sniff`] wants (the Matroska doc type sits a little way in).
pub const HEAD_LEN: usize = 64;

pub fn sniff(head: &[u8], ext: Option<&str>) -> Format {
    if head.len() < 4 {
        return Format::Unknown("empty or truncated file".into());
    }
    if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Format::Jpeg;
    }
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Format::Png;
    }
    if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        return Format::Gif;
    }
    if head.len() >= 12 && head.starts_with(b"RIFF") {
        match &head[8..12] {
            b"WEBP" => return Format::Webp,
            b"AVI " => return Format::Avi,
            _ => {}
        }
    }
    // "BM", then a DIB header whose size is one of the known versions (so text starting "BM" is not one).
    if head.len() >= 18
        && head.starts_with(b"BM")
        && [12, 40, 52, 56, 64, 108, 124].contains(&u32::from_le_bytes([
            head[14], head[15], head[16], head[17],
        ]))
    {
        return Format::Bmp;
    }
    if head.starts_with(&[0x00, 0x00, 0x01, 0xBA]) || head.starts_with(&[0x00, 0x00, 0x01, 0xB3]) {
        return Format::Mpeg;
    }
    if head.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        let webm = head.windows(4).any(|w| w == b"webm");
        return Format::Matroska { webm };
    }
    if head.len() >= 12 && &head[4..8] == b"ftyp" {
        return match &head[8..12] {
            b"heic" | b"heix" | b"hevc" | b"hevx" | b"heim" | b"heis" | b"hevm" | b"hevs"
            | b"mif1" | b"msf1" => Format::Heic,
            b"qt  " => Format::Mov,
            b"avif" | b"avis" => Format::Avif,
            b"M4V " | b"M4VH" | b"M4VP" => Format::Mp4 {
                mime: "video/x-m4v",
            },
            brand if brand.starts_with(b"3gp") || brand.starts_with(b"3gg") => {
                Format::Mp4 { mime: "video/3gpp" }
            }
            brand if brand.starts_with(b"3g2") => Format::Mp4 {
                mime: "video/3gpp2",
            },
            _ => Format::Mp4 { mime: "video/mp4" },
        };
    }
    if head.starts_with(b"II*\0") || head.starts_with(b"MM\0*") {
        return if ext.is_some_and(|e| RAW_EXTENSIONS.contains(&e)) {
            Format::Raw
        } else {
            Format::Tiff
        };
    }
    Format::Unknown(format!(
        "unrecognised file type{}",
        ext.map(|e| format!(" (.{e})")).unwrap_or_default()
    ))
}
