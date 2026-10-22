//! Format detection by magic bytes (the extension only disambiguates TIFF-based RAW).

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Format {
    Jpeg,
    Png,
    Heic,
    /// TIFF-based RAW (DNG, CR2, NEF, ARW, ...); handled through its embedded JPEG preview.
    Raw,
    Mp4,
    Mov,
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
            Format::Raw => "image/x-adobe-dng",
            Format::Mp4 => "video/mp4",
            Format::Mov => "video/quicktime",
            Format::Unknown(_) => "application/octet-stream",
        }
    }
}

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
    if head.len() >= 12 && &head[4..8] == b"ftyp" {
        return match &head[8..12] {
            b"heic" | b"heix" | b"hevc" | b"hevx" | b"heim" | b"heis" | b"hevm" | b"hevs"
            | b"mif1" | b"msf1" => Format::Heic,
            b"qt  " => Format::Mov,
            b"avif" | b"avis" => Format::Unknown("AVIF is not supported yet".into()),
            _ => Format::Mp4,
        };
    }
    if head.starts_with(b"II*\0") || head.starts_with(b"MM\0*") {
        return if ext.is_some_and(|e| RAW_EXTENSIONS.contains(&e)) {
            Format::Raw
        } else {
            Format::Unknown("TIFF files are not supported".into())
        };
    }
    Format::Unknown(format!(
        "unrecognised file type{}",
        ext.map(|e| format!(" (.{e})")).unwrap_or_default()
    ))
}
