//! Helpers for the library export.

/// `(year, month, day, hour, minute, second)` in UTC for milliseconds since the Unix epoch
/// (civil-from-days, Howard Hinnant's algorithm; no date library needed).
pub(crate) fn civil(ms: i64) -> (i64, u32, u32, u32, u32, u32) {
    let secs = ms.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (
        y,
        m,
        d,
        (rem / 3600) as u32,
        (rem % 3600 / 60) as u32,
        (rem % 60) as u32,
    )
}

/// A sensible file extension for a MIME type.
pub(crate) fn extension(mime: &str) -> &'static str {
    match mime {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/heic" | "image/heif" => "heic",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "image/tiff" => "tif",
        "image/x-adobe-dng" | "image/dng" => "dng",
        "video/mp4" => "mp4",
        "video/quicktime" => "mov",
        m if m.starts_with("image/") => "img",
        m if m.starts_with("video/") => "vid",
        _ => "bin",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_civil_utc() {
        assert_eq!(civil(0), (1970, 1, 1, 0, 0, 0));
        assert_eq!(civil(1_700_000_000_000), (2023, 11, 14, 22, 13, 20));
        assert_eq!(civil(-1000), (1969, 12, 31, 23, 59, 59));
        assert_eq!(civil(951_782_400_000), (2000, 2, 29, 0, 0, 0));
    }

    #[test]
    fn extensions() {
        assert_eq!(extension("image/jpeg"), "jpg");
        assert_eq!(extension("video/quicktime"), "mov");
        assert_eq!(extension("application/x-foo"), "bin");
    }
}
