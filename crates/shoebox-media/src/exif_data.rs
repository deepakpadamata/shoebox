//! The EXIF fields the library records, read with kamadak-exif (JPEG, PNG, TIFF/RAW, HEIF).

use exif::{In, Reader, Tag, Value};
use shoebox_core::model::Gps;
use std::io::Cursor;

#[derive(Debug, Default)]
pub struct Meta {
    pub make: Option<String>,
    pub model: Option<String>,
    pub lens: Option<String>,
    /// `DateTimeOriginal` as unix milliseconds (UTC, shifted by `OffsetTimeOriginal` if present).
    pub taken_at_ms: Option<i64>,
    pub orientation: u16,
    pub gps: Option<Gps>,
}

fn ascii(exif: &exif::Exif, tag: Tag) -> Option<String> {
    match &exif.get_field(tag, In::PRIMARY)?.value {
        Value::Ascii(parts) => {
            let s = String::from_utf8_lossy(parts.first()?)
                .trim_matches(['\0', ' '])
                .to_string();
            (!s.is_empty()).then_some(s)
        }
        _ => None,
    }
}

/// Days since 1970-01-01 of a civil date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn taken_at_ms(exif: &exif::Exif) -> Option<i64> {
    let field = exif
        .get_field(Tag::DateTimeOriginal, In::PRIMARY)
        .or_else(|| exif.get_field(Tag::DateTimeDigitized, In::PRIMARY))
        .or_else(|| exif.get_field(Tag::DateTime, In::PRIMARY))?;
    let Value::Ascii(parts) = &field.value else {
        return None;
    };
    let mut dt = exif::DateTime::from_ascii(parts.first()?).ok()?;
    if let Some(Value::Ascii(off)) = exif
        .get_field(Tag::OffsetTimeOriginal, In::PRIMARY)
        .map(|f| &f.value)
    {
        if let Some(o) = off.first() {
            let _ = dt.parse_offset(o);
        }
    }
    if dt.month == 0 || dt.month > 12 || dt.day == 0 || dt.day > 31 {
        return None; // EXIF uses all-zero dates for "unknown"
    }
    let days = days_from_civil(i64::from(dt.year), i64::from(dt.month), i64::from(dt.day));
    let secs = days * 86_400
        + i64::from(dt.hour) * 3600
        + i64::from(dt.minute) * 60
        + i64::from(dt.second);
    let offset_secs = dt.offset.map(|m| i64::from(m) * 60).unwrap_or(0);
    Some((secs - offset_secs) * 1000)
}

fn degrees(exif: &exif::Exif, value: Tag, reference: Tag) -> Option<f64> {
    let Value::Rational(r) = &exif.get_field(value, In::PRIMARY)?.value else {
        return None;
    };
    if r.len() < 3 {
        return None;
    }
    let deg = r[0].to_f64() + r[1].to_f64() / 60.0 + r[2].to_f64() / 3600.0;
    if !deg.is_finite() {
        return None;
    }
    let negative = matches!(ascii(exif, reference).as_deref(), Some("S" | "W"));
    Some(if negative { -deg } else { deg })
}

fn gps(exif: &exif::Exif) -> Option<Gps> {
    let lat = degrees(exif, Tag::GPSLatitude, Tag::GPSLatitudeRef)?;
    let lon = degrees(exif, Tag::GPSLongitude, Tag::GPSLongitudeRef)?;
    if lat.abs() > 90.0 || lon.abs() > 180.0 {
        return None;
    }
    Some(Gps {
        lat_e7: (lat * 1e7).round() as i32,
        lon_e7: (lon * 1e7).round() as i32,
    })
}

/// Read what is there; a file without (or with unreadable) EXIF yields empty metadata.
pub fn read(bytes: &[u8]) -> Meta {
    let Ok(exif) = Reader::new().read_from_container(&mut Cursor::new(bytes)) else {
        return Meta {
            orientation: 1,
            ..Default::default()
        };
    };
    let orientation = exif
        .get_field(Tag::Orientation, In::PRIMARY)
        .and_then(|f| f.value.get_uint(0))
        .filter(|o| (1..=8).contains(o))
        .unwrap_or(1) as u16;
    Meta {
        make: ascii(&exif, Tag::Make),
        model: ascii(&exif, Tag::Model),
        lens: ascii(&exif, Tag::LensModel),
        taken_at_ms: taken_at_ms(&exif),
        orientation,
        gps: gps(&exif),
    }
}
