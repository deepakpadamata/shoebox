//! Minimal EXIF (TIFF) writer: Make/Model/DateTimeOriginal/LensModel/dimensions/GPS.

/// What to record in a photo's EXIF.
#[derive(Clone, Debug, PartialEq)]
pub struct ExifSpec {
    pub make: String,
    pub model: String,
    pub lens: Option<String>,
    /// Unix seconds (UTC); written as a zone-less `DateTimeOriginal`.
    pub taken_at_secs: i64,
    /// Decimal degrees (lat, lon); `None` = no GPS IFD.
    pub gps: Option<(f64, f64)>,
    pub width: u32,
    pub height: u32,
}

/// `(year, month, day, hour, minute, second)` of a unix timestamp, UTC.
pub fn civil_datetime(secs: i64) -> (i32, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400) as u32;
    // Howard Hinnant's civil-from-days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = (yoe + era * 400 + i64::from(month <= 2)) as i32;
    (year, month, day, rem / 3600, rem % 3600 / 60, rem % 60)
}

pub(crate) enum Val {
    Ascii(String),
    Bytes(Vec<u8>),
    Short(u16),
    Shorts(Vec<u16>),
    Long(u32),
    Rationals(Vec<(u32, u32)>),
    SRationals(Vec<(i32, i32)>),
}

pub(crate) struct Entry {
    pub tag: u16,
    pub val: Val,
}

impl Val {
    fn type_and_count(&self) -> (u16, u32) {
        match self {
            Val::Bytes(b) => (1, b.len() as u32),
            Val::Ascii(s) => (2, s.len() as u32 + 1),
            Val::Short(_) => (3, 1),
            Val::Shorts(v) => (3, v.len() as u32),
            Val::Long(_) => (4, 1),
            Val::Rationals(r) => (5, r.len() as u32),
            Val::SRationals(r) => (10, r.len() as u32),
        }
    }

    fn bytes(&self) -> Vec<u8> {
        match self {
            Val::Bytes(b) => b.clone(),
            Val::Ascii(s) => s.bytes().chain([0]).collect(),
            Val::Short(v) => v.to_le_bytes().to_vec(),
            Val::Shorts(v) => v.iter().flat_map(|x| x.to_le_bytes()).collect(),
            Val::SRationals(r) => r
                .iter()
                .flat_map(|(n, d)| n.to_le_bytes().into_iter().chain(d.to_le_bytes()))
                .collect(),
            Val::Long(v) => v.to_le_bytes().to_vec(),
            Val::Rationals(r) => r
                .iter()
                .flat_map(|(n, d)| n.to_le_bytes().into_iter().chain(d.to_le_bytes()))
                .collect(),
        }
    }
}

/// Serialise one IFD that will live at absolute TIFF offset `start`; out-of-line values follow it.
pub(crate) fn build_ifd(mut entries: Vec<Entry>, start: u32) -> Vec<u8> {
    entries.sort_by_key(|e| e.tag);
    let table_len = 2 + 12 * entries.len() as u32 + 4;
    let mut table = (entries.len() as u16).to_le_bytes().to_vec();
    let mut data: Vec<u8> = vec![];
    for e in &entries {
        let (ty, count) = e.val.type_and_count();
        let bytes = e.val.bytes();
        table.extend(e.tag.to_le_bytes());
        table.extend(ty.to_le_bytes());
        table.extend(count.to_le_bytes());
        if bytes.len() <= 4 {
            let mut inline = bytes;
            inline.resize(4, 0);
            table.extend(inline);
        } else {
            table.extend((start + table_len + data.len() as u32).to_le_bytes());
            data.extend(bytes);
            if data.len() % 2 == 1 {
                data.push(0); // keep values word-aligned
            }
        }
    }
    table.extend(0u32.to_le_bytes()); // no next IFD
    table.extend(data);
    table
}

fn dms(deg: f64) -> Val {
    let deg = deg.abs();
    let d = deg.floor();
    let m = ((deg - d) * 60.0).floor();
    let s = (deg - d - m / 60.0) * 3600.0;
    Val::Rationals(vec![
        (d as u32, 1),
        (m as u32, 1),
        ((s * 1000.0).round() as u32, 1000),
    ])
}

/// Little-endian TIFF structure (the payload after `Exif\0\0` in a JPEG APP1 segment).
pub fn build_exif_tiff(spec: &ExifSpec) -> Vec<u8> {
    let (y, mo, d, h, mi, s) = civil_datetime(spec.taken_at_secs);
    let datetime = format!("{y:04}:{mo:02}:{d:02} {h:02}:{mi:02}:{s:02}");

    let ifd0 = |exif_ptr: u32, gps_ptr: Option<u32>, start: u32| {
        let mut e = vec![
            Entry {
                tag: 0x010F,
                val: Val::Ascii(spec.make.clone()),
            },
            Entry {
                tag: 0x0110,
                val: Val::Ascii(spec.model.clone()),
            },
            Entry {
                tag: 0x0112,
                val: Val::Short(1),
            },
            Entry {
                tag: 0x8769,
                val: Val::Long(exif_ptr),
            },
        ];
        if let Some(p) = gps_ptr {
            e.push(Entry {
                tag: 0x8825,
                val: Val::Long(p),
            });
        }
        build_ifd(e, start)
    };
    let mut exif_entries = vec![
        Entry {
            tag: 0x9003,
            val: Val::Ascii(datetime),
        },
        Entry {
            tag: 0xA002,
            val: Val::Long(spec.width),
        },
        Entry {
            tag: 0xA003,
            val: Val::Long(spec.height),
        },
    ];
    if let Some(lens) = &spec.lens {
        exif_entries.push(Entry {
            tag: 0xA434,
            val: Val::Ascii(lens.clone()),
        });
    }

    // Layout: header (8) | IFD0 | Exif IFD | GPS IFD. Pointer values are 4 inline bytes, so the
    // size of IFD0 does not depend on them: measure with placeholders, then rebuild.
    let ifd0_len = ifd0(0, spec.gps.map(|_| 0), 8).len() as u32;
    let exif_off = 8 + ifd0_len;
    let exif_ifd = build_ifd(exif_entries, exif_off);
    let gps_off = exif_off + exif_ifd.len() as u32;

    let mut out = b"II*\0".to_vec();
    out.extend(8u32.to_le_bytes());
    out.extend(ifd0(exif_off, spec.gps.map(|_| gps_off), 8));
    out.extend(exif_ifd);
    if let Some((lat, lon)) = spec.gps {
        let gps = vec![
            Entry {
                tag: 0x0001,
                val: Val::Ascii(if lat >= 0.0 { "N" } else { "S" }.into()),
            },
            Entry {
                tag: 0x0002,
                val: dms(lat),
            },
            Entry {
                tag: 0x0003,
                val: Val::Ascii(if lon >= 0.0 { "E" } else { "W" }.into()),
            },
            Entry {
                tag: 0x0004,
                val: dms(lon),
            },
        ];
        out.extend(build_ifd(gps, gps_off));
    }
    out
}
