//! `cargo xtask exports`: two small, anonymised exports, one Google Takeout and one iCloud
//! Photos data export, laid out like the real ones looked when we checked them (a 373 GB
//! Takeout from June 2026 and an iCloud export from November 2026), and each with `truth.json`
//! saying what an import must make of every file.
//!
//! Nothing in them comes from a real export: names follow the real patterns but are made up,
//! pixels are synthetic, people are "Person A", places are city centres. Every case says whether
//! it was **seen** in a real export or only documented by Google, Apple or other tools. What we
//! learnt from the real exports is in `docs/notes/takeout-format.md` and
//! `docs/notes/icloud-format.md`; the fixtures are checked in under `fixtures/exports/`.

use crate::exifgen::{civil_datetime, ExifSpec};
use crate::jpeg::{render_jpeg, render_plain_jpeg};
use crate::samples::{screenshot_png, synthetic_dng};
use crate::takeout::Truth;
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// The root of a Takeout's photos as the June 2026 (English) export spelled it: a plain space.
pub const GOOGLE_ROOT: &str = "Takeout/Google Photos";
/// The root of the iCloud export (`iCloud Photos.zip` unpacked).
pub const ICLOUD_ROOT: &str = "iCloud Photos";

/// Both real exports were made in India: cameras write local time (+05:30) with no zone, the
/// exports write UTC.
const IST: i64 = 19_800;

/// One media file of the Google fixture and what it must become.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct GoogleCase {
    /// The same fields as the zip-part fixture (`cargo xtask fixtures --preset takeout`); `part`
    /// is always 0 (one unpacked folder). `taken_at_ms` is `None` where the file's own date is
    /// not ours to know (videos, HEIC, RAW stand-ins).
    #[serde(flatten)]
    pub truth: Truth,
    /// Names from the sidecar's `people` (suggestions only, I1-11).
    #[serde(default)]
    pub people: Vec<String>,
    /// Another entry with these bytes under another name: the plan cannot know, the import
    /// dedupes them into one photo (so the photo ends up in this entry's albums too).
    #[serde(default)]
    pub same_bytes_as: Option<String>,
    /// What the case is about.
    pub case: String,
    /// Seen in the real export (true) or only reported by other tools (false).
    pub seen: bool,
}

/// One media file (or CSV row) of the iCloud fixture and what it must become.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct IcloudCase {
    pub path: String,
    /// The file is missing from the export: only its CSV row exists (reported, not fatal).
    #[serde(default)]
    pub missing: bool,
    pub favorite: bool,
    pub hidden: bool,
    /// Recently deleted: not imported.
    pub deleted: bool,
    /// Album titles it ends up in ("Hidden (from iCloud)" for a hidden photo).
    pub albums: Vec<String>,
    /// For the video of a Live Photo: the path of its picture.
    pub live_photo_of: Option<String>,
    /// The date it must get (UTC ms): the file's own, else `originalCreationDate`.
    pub taken_at_ms: Option<i64>,
    /// BLAKE3 of the bytes (empty when missing).
    pub blake3: String,
    pub case: String,
    /// Seen in the real export (true) or only documented by Apple's file guides (false).
    pub seen: bool,
}

#[derive(Debug)]
pub struct Summary {
    pub google_files: usize,
    pub icloud_files: usize,
    pub bytes: u64,
}

struct Tree {
    files: Vec<(String, Vec<u8>)>,
}

impl Tree {
    fn put(&mut self, path: impl Into<String>, bytes: impl Into<Vec<u8>>) {
        let path = path.into();
        assert!(
            !self.files.iter().any(|(p, _)| *p == path),
            "{path} written twice"
        );
        self.files.push((path, bytes.into()));
    }
    fn get(&self, path: &str) -> Vec<u8> {
        self.files
            .iter()
            .find(|(p, _)| p == path)
            .unwrap_or_else(|| panic!("{path}"))
            .1
            .clone()
    }
    fn write(&self, dir: &Path) -> io::Result<u64> {
        let mut total = 0;
        for (path, bytes) in &self.files {
            let p = dir.join(path);
            std::fs::create_dir_all(p.parent().unwrap())?;
            std::fs::write(p, bytes)?;
            total += bytes.len() as u64;
        }
        Ok(total)
    }
}

fn hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

/// Cut an ASCII name to `n` characters (Takeout cuts at UTF-16 units; every name here is ASCII).
fn cut(s: &str, n: usize) -> &str {
    &s[..s.len().min(n)]
}

/// The current sidecar name: `<name>.supplemental-metadata`, cut at 46, then `(n)` and `.json`.
fn supplemental(name: &str, n: Option<u32>) -> String {
    let base = format!("{name}.supplemental-metadata");
    let num = n.map(|n| format!("({n})")).unwrap_or_default();
    format!("{}{num}.json", cut(&base, 46))
}

/// `YYYYMMDD_HHMMSS` of a local time.
fn stamp(local: i64) -> String {
    let (y, mo, d, h, mi, s) = civil_datetime(local);
    format!("{y:04}{mo:02}{d:02}_{h:02}{mi:02}{s:02}")
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// `7 Feb 2019, 04:45:00 UTC`, as the English export formats it (never parsed).
fn google_formatted(utc: i64) -> String {
    let (y, mo, d, h, mi, s) = civil_datetime(utc);
    format!(
        "{d} {} {y}, {h:02}:{mi:02}:{s:02} UTC",
        &MONTHS[mo as usize - 1][..3]
    )
}

/// `Thursday September 24,2015 11:37 AM GMT`, as `Photo Details.csv` writes it (minutes only).
fn icloud_date(utc: i64) -> String {
    const DAYS: [&str; 7] = [
        "Thursday",
        "Friday",
        "Saturday",
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
    ];
    let (y, mo, d, h, mi, _) = civil_datetime(utc);
    let wd = DAYS[utc.div_euclid(86_400).rem_euclid(7) as usize];
    let (h12, ampm) = match h {
        0 => (12, "AM"),
        1..=11 => (h, "AM"),
        12 => (12, "PM"),
        _ => (h - 12, "PM"),
    };
    format!(
        "{wd} {} {d},{y} {h12}:{mi:02} {ampm} GMT",
        MONTHS[mo as usize - 1]
    )
}

/// A video sample with its `mvhd` creation and modification times set to `utc`: a valid file
/// with its own date and its own bytes.
fn retimed_video(sample: &[u8], utc: i64) -> Vec<u8> {
    let mut v = sample.to_vec();
    let at = v
        .windows(4)
        .position(|w| w == b"mvhd")
        .expect("sample has an mvhd box");
    assert_eq!(v[at + 4], 0, "version 0 mvhd");
    let mac = (utc + 2_082_844_800) as u32;
    v[at + 8..at + 12].copy_from_slice(&mac.to_be_bytes());
    v[at + 12..at + 16].copy_from_slice(&mac.to_be_bytes());
    v
}

fn exif(make: &str, model: &str, local: i64, gps: Option<(f64, f64)>) -> ExifSpec {
    ExifSpec {
        make: make.into(),
        model: model.into(),
        lens: None,
        taken_at_secs: local,
        gps,
        width: 64,
        height: 48,
        orientation: 1,
    }
}

// ------------------------------------------------------------------------------------------
// Google Takeout
// ------------------------------------------------------------------------------------------

const BENGALURU: (f64, f64) = (12.9716, 77.5946);
const MYSURU: (f64, f64) = (12.2958, 76.6394);

/// What goes into a sidecar besides the title and dates.
#[derive(Default, Clone)]
struct Side {
    /// `geoData` (and `geoDataExif` when `exif_geo`); `None` writes `0.0, 0.0`.
    geo: Option<(f64, f64)>,
    exif_geo: bool,
    people: Vec<&'static str>,
    /// `googlePhotosOrigin`, as raw JSON; `None` leaves the key out (old sidecars).
    origin: Option<&'static str>,
    app: Option<&'static str>,
    favorited: bool,
    archived: bool,
    trashed: bool,
}

fn geo_json(g: Option<(f64, f64)>) -> String {
    let (lat, lon) = g.unwrap_or((0.0, 0.0));
    let alt = if g.is_some() { 920.4 } else { 0.0 };
    format!(
        "{{\n    \"latitude\": {lat:?},\n    \"longitude\": {lon:?},\n    \"altitude\": {alt:?},\n    \"latitudeSpan\": 0.0,\n    \"longitudeSpan\": 0.0\n  }}"
    )
}

const PHONE: &str = r#"{
    "mobileUpload": {
      "deviceFolder": {
        "localFolderName": ""
      },
      "deviceType": "ANDROID_PHONE"
    }
  }"#;
const WEB: &str = r#"{
    "webUpload": {
      "computerUpload": {}
    }
  }"#;
const SHARED: &str = r#"{
    "fromSharedAlbum": {}
  }"#;
const COMPOSITION: &str = r#"{
    "composition": {
      "type": "AUTO"
    }
  }"#;

/// A sidecar in the key order and indentation of the real export.
fn sidecar(title: &str, taken_utc: i64, s: &Side) -> Vec<u8> {
    let q = |v: &str| serde_json::to_string(v).unwrap();
    let mut keys = vec![
        ("title", q(title)),
        ("description", q("")),
        ("imageViews", q("4")),
        (
            "creationTime",
            format!(
                "{{\n    \"timestamp\": \"{}\",\n    \"formatted\": {}\n  }}",
                taken_utc + 86_400,
                q(&google_formatted(taken_utc + 86_400))
            ),
        ),
        (
            "photoTakenTime",
            format!(
                "{{\n    \"timestamp\": \"{taken_utc}\",\n    \"formatted\": {}\n  }}",
                q(&google_formatted(taken_utc))
            ),
        ),
        ("geoData", geo_json(s.geo)),
    ];
    if s.exif_geo {
        keys.push(("geoDataExif", geo_json(s.geo)));
    }
    if !s.people.is_empty() {
        let p: Vec<String> = s
            .people
            .iter()
            .map(|n| format!("{{\n    \"name\": {}\n  }}", q(n)))
            .collect();
        keys.push(("people", format!("[{}]", p.join(", "))));
    }
    keys.push(("url", q("https://photos.google.com/photo/AF1QipFixture")));
    if let Some(o) = s.origin {
        keys.push(("googlePhotosOrigin", o.to_string()));
    }
    if let Some(a) = s.app {
        keys.push((
            "appSource",
            format!("{{\n    \"androidPackageName\": {}\n  }}", q(a)),
        ));
    }
    for (k, on) in [
        ("favorited", s.favorited),
        ("archived", s.archived),
        ("trashed", s.trashed),
    ] {
        if on {
            keys.push((k, "true".into()));
        }
    }
    let body: Vec<String> = keys
        .into_iter()
        .map(|(k, v)| format!("  \"{k}\": {v}"))
        .collect();
    format!("{{\n{}\n}}", body.join(",\n")).into_bytes()
}

struct Google {
    tree: Tree,
    cases: Vec<GoogleCase>,
}

impl Google {
    /// Write a media file (and its sidecar, if any) and record its case.
    #[allow(clippy::too_many_arguments)]
    fn add(
        &mut self,
        dir: &str,
        name: &str,
        bytes: Vec<u8>,
        sidecar_name: Option<String>,
        title: &str,
        taken_utc: i64,
        side: &Side,
        case: GoogleCase,
    ) -> usize {
        let dir = format!("{GOOGLE_ROOT}/{dir}");
        let path = format!("{dir}/{name}");
        let sidecar_path = sidecar_name.map(|s| format!("{dir}/{s}"));
        if let Some(s) = &sidecar_path {
            self.tree.put(s.clone(), sidecar(title, taken_utc, side));
        }
        let mut case = case;
        case.truth.path = path.clone();
        case.truth.sidecar = sidecar_path;
        case.truth.blake3 = hex(&bytes);
        case.truth.favorite = side.favorited;
        case.truth.archived = side.archived;
        case.truth.trashed = side.trashed;
        case.people = side.people.iter().map(|p| p.to_string()).collect();
        self.tree.put(path, bytes);
        self.cases.push(case);
        self.cases.len() - 1
    }

    /// A media file with no sidecar of its own (an edited copy, a Live Photo's video).
    fn add_bare(&mut self, dir: &str, name: &str, bytes: Vec<u8>, case: GoogleCase) -> usize {
        let path = format!("{GOOGLE_ROOT}/{dir}/{name}");
        let mut case = case;
        case.truth.path = path.clone();
        case.truth.blake3 = hex(&bytes);
        self.tree.put(path, bytes);
        self.cases.push(case);
        self.cases.len() - 1
    }

    /// An album folder: its metadata file (raw JSON, or none) and copies of `members` (each
    /// with a copy of its sidecar, renamed for the album folder as Takeout names it there).
    fn album(&mut self, folder: &str, title: &str, meta: Option<String>, members: &[usize]) {
        let dir = format!("{GOOGLE_ROOT}/{folder}");
        if let Some(m) = meta {
            self.tree
                .put(format!("{dir}/metadata.json"), m.into_bytes());
        }
        for &m in members {
            let src = self.cases[m].clone();
            let name = src.truth.path.rsplit('/').next().unwrap().to_owned();
            let side_name = src
                .truth
                .sidecar
                .as_ref()
                .map(|s| s.rsplit('/').next().unwrap().to_owned());
            if let (Some(s), Some(n)) = (&src.truth.sidecar, &side_name) {
                let bytes = self.tree.get(s);
                self.tree.put(format!("{dir}/{n}"), bytes);
            }
            self.tree
                .put(format!("{dir}/{name}"), self.tree.get(&src.truth.path));
            self.cases[m].truth.albums.push(title.to_owned());
            self.cases.push(GoogleCase {
                truth: Truth {
                    path: format!("{dir}/{name}"),
                    sidecar: side_name.map(|n| format!("{dir}/{n}")),
                    copy_of: Some(src.truth.path.clone()),
                    ..src.truth.clone()
                },
                case: format!("copy in the album folder `{folder}`"),
                ..src.clone()
            });
        }
    }
}

fn case(text: &str, seen: bool, taken_local: Option<i64>) -> GoogleCase {
    GoogleCase {
        truth: Truth {
            taken_at_ms: taken_local.map(|t| t * 1000),
            ..Default::default()
        },
        case: text.into(),
        seen,
        ..Default::default()
    }
}

fn album_meta(title: Option<&str>, extra: &str) -> String {
    let mut keys = vec![];
    if let Some(t) = title {
        keys.push(format!(
            "  \"title\": {}",
            serde_json::to_string(t).unwrap()
        ));
    }
    if !extra.is_empty() {
        keys.push(extra.to_owned());
    }
    if keys.is_empty() {
        "{}".into()
    } else {
        format!("{{\n{}\n}}", keys.join(",\n"))
    }
}

fn google(samples: &Path) -> io::Result<(Tree, Vec<GoogleCase>)> {
    let clip = std::fs::read(samples.join("clip.mp4"))?;
    let heic = std::fs::read(samples.join("photo.heic"))?;
    let gif = std::fs::read(samples.join("anim.gif"))?;
    let mut g = Google {
        tree: Tree { files: vec![] },
        cases: vec![],
    };
    let y19 = "Photos from 2019";
    let y23 = "Photos from 2023";
    // 2019-02-07 10:15:00 and 2023-06-15 10:10:00 local time (+05:30).
    let t19 = 1_549_514_700;
    let t23 = 1_686_804_000;
    let local = |utc: i64| utc + IST;
    let phone = Side {
        origin: Some(PHONE),
        ..Default::default()
    };

    // ---- Photos from 2019 ----
    // A plain phone photo: current sidecar name, place in the file and the export, people.
    let t = t19;
    let n = format!("IMG_{}.jpg", stamp(local(t)));
    let s = Side {
        geo: Some(BENGALURU),
        exif_geo: true,
        people: vec!["Person A", "Person B"],
        ..phone.clone()
    };
    let beach_a = g.add(
        y19,
        &n,
        render_jpeg(
            &exif("samsung", "SM-G960F", local(t), Some(BENGALURU)),
            0x6001,
        ),
        Some(supplemental(&n, None)),
        &n,
        t,
        &s,
        case(
            "plain photo, `.supplemental-metadata.json`; EXIF in local time, export in UTC (+05:30: the file's date stays); `people`",
            true,
            Some(local(t)),
        ),
    );
    // The old sidecar name, `<name>.json` (51 of 27,127 sidecars), no googlePhotosOrigin.
    let t = t19 + 60;
    let n = format!("IMG_{}.jpg", stamp(local(t)));
    let untitled = g.add(
        y19,
        &n,
        render_jpeg(&exif("samsung", "SM-G960F", local(t), None), 0x6002),
        Some(format!("{n}.json")),
        &n,
        t,
        &Side::default(),
        case(
            "old sidecar name `<name>.json`, no `googlePhotosOrigin`",
            true,
            Some(local(t)),
        ),
    );
    // Two different photos called DSC_0101.JPG in one year: `(1)` goes after the suffix.
    let t = t19 + 120;
    let mom_a = g.add(
        y19,
        "DSC_0101.JPG",
        render_jpeg(
            &exif("NIKON CORPORATION", "NIKON D5300", local(t), None),
            0x6003,
        ),
        Some(supplemental("DSC_0101.JPG", None)),
        "DSC_0101.JPG",
        t,
        &Side {
            origin: Some(WEB),
            ..Default::default()
        },
        case("same name twice in a year: the first", true, Some(local(t))),
    );
    let t2 = t19 + 86_400 * 30;
    g.add(
        y19,
        "DSC_0101(1).JPG",
        render_jpeg(
            &exif("NIKON CORPORATION", "NIKON D5300", local(t2), None),
            0x6004,
        ),
        Some(supplemental("DSC_0101.JPG", Some(1))),
        "DSC_0101.JPG",
        t2,
        &Side {
            origin: Some(WEB),
            ..Default::default()
        },
        case(
            "same name twice in a year: `DSC_0101(1).JPG` → `DSC_0101.JPG.supplemental-metadata(1).json`, title without the number (216 such)",
            true,
            Some(local(t2)),
        ),
    );
    // Samsung names: the number stays in the stem, and the title has it too (56 such).
    let t = t19 + 180;
    let n = format!("{}.jpg", stamp(local(t)));
    let n1 = format!("{}(1).jpg", stamp(local(t)));
    g.add(
        y19,
        &n,
        render_jpeg(&exif("samsung", "SM-G960F", local(t), None), 0x6005),
        Some(supplemental(&n, None)),
        &n,
        t,
        &phone,
        case(
            "Samsung camera name, the first of two",
            true,
            Some(local(t)),
        ),
    );
    g.add(
        y19,
        &n1,
        render_jpeg(&exif("samsung", "SM-G960F", local(t), None), 0x6006),
        Some(supplemental(&n1, None)),
        &n1,
        t,
        &phone,
        case(
            "`<stamp>(1).jpg` → `<stamp>(1).jpg.supplemental-metadata.json`: the number stays in the stem and in the title",
            true,
            Some(local(t)),
        ),
    );
    // Edited in Google Photos: `-edited` copy with no sidecar of its own.
    let t = t19 + 240;
    let n = format!("IMG_{}.jpg", stamp(local(t)));
    let mom_b = g.add(
        y19,
        &n,
        render_jpeg(&exif("samsung", "SM-G960F", local(t), None), 0x6007),
        Some(supplemental(&n, None)),
        &n,
        t,
        &phone,
        case("original of an edited copy", true, Some(local(t))),
    );
    let mut c = case(
        "`-edited` copy (lower case extension): no sidecar, takes the original's",
        true,
        Some(local(t)),
    );
    c.truth.sidecar = g.cases[mom_b].truth.sidecar.clone();
    c.truth.edit_of = Some(g.cases[mom_b].truth.path.clone());
    let edited_b = g.add_bare(
        y19,
        &format!("IMG_{}-edited.jpg", stamp(local(t))),
        render_jpeg(&exif("samsung", "SM-G960F", local(t), None), 0x6008),
        c,
    );
    let t = t19 + 300;
    let orig = g.add(
        y19,
        "DSC_0102.JPG",
        render_jpeg(
            &exif("NIKON CORPORATION", "NIKON D5300", local(t), None),
            0x6009,
        ),
        Some(supplemental("DSC_0102.JPG", None)),
        "DSC_0102.JPG",
        t,
        &Side {
            origin: Some(WEB),
            ..Default::default()
        },
        case("original of an edited copy", true, Some(local(t))),
    );
    let mut c = case(
        "`DSC_0102-edited.JPG` (the most common form, 732 such)",
        true,
        Some(local(t)),
    );
    c.truth.sidecar = g.cases[orig].truth.sidecar.clone();
    c.truth.edit_of = Some(g.cases[orig].truth.path.clone());
    g.add_bare(
        y19,
        "DSC_0102-edited.JPG",
        render_jpeg(
            &exif("NIKON CORPORATION", "NIKON D5300", local(t), None),
            0x600A,
        ),
        c,
    );
    // A video exported with no extension; the sidecar and title still say `.mp4`.
    let t = t19 + 360;
    let n = stamp(local(t));
    g.add(
        y19,
        &n,
        retimed_video(&clip, t),
        Some(supplemental(&format!("{n}.mp4"), None)),
        &format!("{n}.mp4"),
        t,
        &phone,
        case(
            "video with no extension (MP4 bytes); sidecar and title name it `.mp4` (16 such)",
            true,
            None,
        ),
    );
    // `:` in the title is `_` in the file and sidecar names.
    let t = t19 + 420;
    let title = format!("IMG_{}:temp:.jpg", stamp(local(t)));
    let n = title.replace(':', "_");
    g.add(
        y19,
        &n,
        render_jpeg(&exif("samsung", "SM-G960F", local(t), None), 0x600B),
        Some(supplemental(&n, None)),
        &title,
        t,
        &phone,
        case(
            "the title has `:`, the file and sidecar names `_` (the sidecar is no longer `<title>.…`)",
            true,
            Some(local(t)),
        ),
    );
    // A long name: the file is cut at 47 characters plus its extension, the sidecar is the
    // title cut at 46 with no suffix left, and only the title has the whole name.
    let t = t19 + 480;
    let title = "Screenshot_2019-02-07-10-23-00-123_com.example.messenger.lite.jpg";
    let n = format!("{}.jpg", cut(title.trim_end_matches(".jpg"), 47));
    g.add(
        y19,
        &n,
        render_plain_jpeg(64, 112, 0x600C),
        Some(format!("{}.json", cut(title, 46))),
        title,
        t,
        &phone,
        case(
            "a 65-character name: file cut to 47 + `.jpg`, sidecar = title cut at 46 + `.json`; no EXIF, so the export's date",
            true,
            Some(t),
        ),
    );
    // A 51-character name: the file keeps it, the sidecar is cut at 46.
    let t = t19 + 540;
    let n = "Screenshot_2019-02-07-10-24-00-456_com.example1.jpg";
    g.add(
        y19,
        n,
        render_plain_jpeg(64, 112, 0x600D),
        Some(format!("{}.json", cut(n, 46))),
        n,
        t,
        &phone,
        case(
            "a 51-character name: the file keeps it, the sidecar is cut at 46 with no suffix left",
            true,
            Some(t),
        ),
    );

    // ---- Photos from 2023 ----
    // A Pixel name of 26 characters: the suffix is cut to `.supplemental-metada`.
    let t = t23;
    let n = format!("PXL_{}123.jpg", stamp(local(t)));
    let beach_b = g.add(
        y23,
        &n,
        render_jpeg(&exif("Google", "Pixel 7", local(t), Some(MYSURU)), 0x7001),
        Some(supplemental(&n, None)),
        &n,
        t,
        &Side {
            geo: Some(MYSURU),
            exif_geo: true,
            ..phone.clone()
        },
        case(
            "Pixel name: `.supplemental-metada.json` (cut at 46)",
            true,
            Some(local(t)),
        ),
    );
    // A Pixel motion photo: `.MP.jpg` with a sidecar and `.MP` (the MP4) without one.
    let t = t23 + 60;
    let n = format!("PXL_{}456.MP.jpg", stamp(local(t)));
    let mp = g.add(
        y23,
        &n,
        render_jpeg(&exif("Google", "Pixel 7", local(t), None), 0x7002),
        Some(supplemental(&n, None)),
        &n,
        t,
        &phone,
        case(
            "Pixel motion photo: `.MP.jpg` (a real one carries MotionPhoto XMP and its video at the end; this one is a plain JPEG), sidecar `.MP.jpg.supplemental-met.json`",
            true,
            Some(local(t)),
        ),
    );
    let mut c = case(
        "Pixel motion photo: `.MP` is its video as MP4 (`ftypisom`), no sidecar (3,476 such)",
        true,
        None,
    );
    c.truth.sidecar = g.cases[mp].truth.sidecar.clone();
    c.truth.live_photo_of = Some(g.cases[mp].truth.path.clone());
    let mp_video = g.add_bare(
        y23,
        &format!("PXL_{}456.MP", stamp(local(t))),
        retimed_video(&clip, t),
        c,
    );
    // An iPhone Live Photo: HEIC with a sidecar, MP4 without one.
    let t = t23 + 120;
    let still = g.add(
        y23,
        "IMG_0420.HEIC",
        heic.clone(),
        Some(supplemental("IMG_0420.HEIC", None)),
        "IMG_0420.HEIC",
        t,
        &Side {
            origin: Some(WEB),
            ..Default::default()
        },
        case("iPhone Live Photo: the HEIC", true, None),
    );
    let mut c = case(
        "iPhone Live Photo: the `.MP4` beside a `.HEIC`, no sidecar (195 such)",
        true,
        None,
    );
    c.truth.sidecar = g.cases[still].truth.sidecar.clone();
    c.truth.live_photo_of = Some(g.cases[still].truth.path.clone());
    g.add_bare(y23, "IMG_0420.MP4", retimed_video(&clip, t), c);
    let t = t23 + 180;
    let road = g.add(
        y23,
        "IMG_0421.JPG",
        render_jpeg(&exif("Apple", "iPhone 12", local(t), None), 0x7003),
        Some(supplemental("IMG_0421.JPG", None)),
        "IMG_0421.JPG",
        t,
        &Side {
            origin: Some(WEB),
            ..Default::default()
        },
        case(
            "Live Photo with a JPEG still (1,027 such)",
            true,
            Some(local(t)),
        ),
    );
    let mut c = case("the `.MP4` beside a `.JPG`, no sidecar", true, None);
    c.truth.sidecar = g.cases[road].truth.sidecar.clone();
    c.truth.live_photo_of = Some(g.cases[road].truth.path.clone());
    let road_video = g.add_bare(y23, "IMG_0421.MP4", retimed_video(&clip, t), c);
    // Google's own creations.
    let t = t23 + 240;
    let n = format!("IMG_{}-EFFECTS.jpg", stamp(local(t)));
    g.add(
        y23,
        &n,
        render_plain_jpeg(64, 48, 0x7004),
        Some(supplemental(&n, None)),
        &n,
        t,
        &Side {
            origin: Some(COMPOSITION),
            ..Default::default()
        },
        case(
            "made by Google Photos (`-EFFECTS`, `composition: AUTO`): an ordinary photo",
            true,
            Some(t),
        ),
    );
    let t = t23 + 300;
    let n = format!("IMG_{}-ANIMATION.gif", stamp(local(t)));
    g.add(
        y23,
        &n,
        gif.clone(),
        Some(supplemental(&n, None)),
        &n,
        t,
        &Side {
            origin: Some(COMPOSITION),
            ..Default::default()
        },
        case(
            "made by Google Photos (`-ANIMATION.gif`), sidecar cut to `.supplemental.json`",
            true,
            Some(t),
        ),
    );
    // A favourite with no place anywhere: `geoData` 0,0 and no `geoDataExif` key.
    let t = t23 + 360;
    let n = format!("IMG_{}.jpg", stamp(local(t)));
    let hike = g.add(
        y23,
        &n,
        render_jpeg(&exif("samsung", "SM-S911B", local(t), None), 0x7005),
        Some(supplemental(&n, None)),
        &n,
        t,
        &Side {
            favorited: true,
            ..phone.clone()
        },
        case(
            "`favorited: true` (the key is absent otherwise); `geoData` 0,0 and no `geoDataExif`",
            true,
            Some(local(t)),
        ),
    );
    // A screenshot: no date or place of its own, both from the export; `appSource`.
    let t = t23 + 420;
    let n = format!("Screenshot_{}.png", stamp(local(t)).replace('_', "-"));
    let mut c = case(
        "screenshot (PNG, no EXIF): date and place from the export; `appSource`",
        true,
        Some(t),
    );
    c.truth.place = Some(BENGALURU);
    g.add(
        y23,
        &n,
        screenshot_png(),
        Some(supplemental(&n, None)),
        &n,
        t,
        &Side {
            geo: Some(BENGALURU),
            app: Some("com.example.app"),
            ..phone.clone()
        },
        c,
    );
    // RAW: two different NEFs with one name. The album folder's `DSC_0200.NEF` is the year's
    // `DSC_0200(1).NEF` (same bytes): Takeout numbered it in the year folder only.
    let t = t23 + 480;
    let raw_a = synthetic_dng(64, 48);
    let raw_b = synthetic_dng(72, 48);
    g.add(
        y23,
        "DSC_0200.NEF",
        raw_a,
        Some(supplemental("DSC_0200.NEF", None)),
        "DSC_0200.NEF",
        t,
        &Side {
            origin: Some(WEB),
            ..Default::default()
        },
        case(
            "RAW (`.NEF`; the bytes are the repository's synthetic DNG stand-in)",
            true,
            None,
        ),
    );
    let t2 = t + 3600;
    let raw_b_case = g.add(
        y23,
        "DSC_0200(1).NEF",
        raw_b.clone(),
        Some(supplemental("DSC_0200.NEF", Some(1))),
        "DSC_0200.NEF",
        t2,
        &Side {
            origin: Some(WEB),
            ..Default::default()
        },
        case(
            "a second `DSC_0200.NEF`, numbered in the year folder only",
            true,
            None,
        ),
    );
    // In a shared album, and in the year folder too (3,302 of 3,643 such).
    let t = t23 + 540;
    let n = format!("IMG_{}.jpg", stamp(local(t)));
    let picnic = g.add(
        y23,
        &n,
        render_jpeg(&exif("samsung", "SM-S911B", local(t), None), 0x7006),
        Some(supplemental(&n, None)),
        &n,
        t,
        &Side {
            origin: Some(SHARED),
            ..Default::default()
        },
        case(
            "from a shared album (`fromSharedAlbum`), in the year folder too",
            true,
            Some(local(t)),
        ),
    );

    // ---- Trash, Archive, Failed Videos: nowhere else ----
    let t = t23 + 600;
    let n = format!("IMG_{}.jpg", stamp(local(t)));
    g.add(
        "Trash",
        &n,
        render_jpeg(&exif("samsung", "SM-S911B", local(t), None), 0x7007),
        Some(supplemental(&n, None)),
        &n,
        t,
        &Side {
            trashed: true,
            ..phone.clone()
        },
        case(
            "in `Trash` only, `trashed: true`: not imported",
            true,
            Some(local(t)),
        ),
    );
    let t = t23 + 660;
    let n = format!("IMG_{}.jpg", stamp(local(t)));
    g.add(
        "Archive",
        &n,
        render_jpeg(&exif("samsung", "SM-S911B", local(t), None), 0x7008),
        Some(supplemental(&n, None)),
        &n,
        t,
        &Side {
            archived: true,
            ..phone.clone()
        },
        case(
            "in `Archive` only, `archived: true`: imported as an ordinary photo",
            true,
            Some(local(t)),
        ),
    );
    let t = t23 + 720;
    let n = format!("VID_{}.mp4", stamp(local(t)));
    g.add(
        "Failed Videos",
        &n,
        retimed_video(&clip, t),
        Some(supplemental(&n, None)),
        &n,
        t,
        &phone,
        case(
            "in Google's `Failed Videos` folder (no metadata.json): an ordinary video, no album",
            true,
            None,
        ),
    );

    // ---- Albums ----
    let date = |utc: i64| {
        format!(
            "  \"date\": {{\n    \"timestamp\": \"{utc}\",\n    \"formatted\": \"{}\"\n  }}",
            google_formatted(utc)
        )
    };
    // Full metadata, with a location enrichment.
    g.album(
        "Beach Day",
        "Beach Day",
        Some(album_meta(
            Some("Beach Day"),
            &format!(
                "  \"description\": \"\",\n  \"access\": \"protected\",\n{},\n  \"enrichments\": [{{\n    \"locationEnrichment\": {{\n      \"location\": [{{\n        \"name\": \"Mysuru\",\n        \"description\": \"Karnataka\",\n        \"latitudeE7\": 122958000,\n        \"longitudeE7\": 766394000\n      }}]\n    }}\n  }}]",
                date(t23 + 7200)
            ),
        )),
        &[beach_a, beach_b],
    );
    // The NEF trap: same name as the year's DSC_0200.NEF, bytes of DSC_0200(1).NEF.
    {
        let dir = format!("{GOOGLE_ROOT}/Beach Day");
        let side = format!("{dir}/{}", supplemental("DSC_0200.NEF", None));
        g.tree.put(
            side.clone(),
            sidecar(
                "DSC_0200.NEF",
                t23 + 480 + 3600,
                &Side {
                    origin: Some(WEB),
                    ..Default::default()
                },
            ),
        );
        let path = format!("{dir}/DSC_0200.NEF");
        g.tree.put(path.clone(), raw_b.clone());
        let twin = g.cases[raw_b_case].truth.path.clone();
        g.cases.push(GoogleCase {
            truth: Truth {
                path,
                sidecar: Some(side),
                albums: vec!["Beach Day".into()],
                blake3: hex(&raw_b),
                ..Default::default()
            },
            same_bytes_as: Some(twin),
            case: "album copy named like the year's `DSC_0200.NEF` but holding `DSC_0200(1).NEF`'s bytes (18 such): not a copy of the same-named file; its sidecar's date tells them apart".into(),
            seen: true,
            ..Default::default()
        });
    }
    // Only a title (54 albums), with an escaped name: `'` → `_`.
    g.album(
        "Mom_s 60th",
        "Mom's 60th",
        Some(album_meta(Some("Mom's 60th"), "")),
        &[mom_a, mom_b],
    );
    // `/` → `-`, with a map enrichment.
    g.album(
        "Trip 6-10-23",
        "Trip 6/10/23",
        Some(album_meta(
            Some("Trip 6/10/23"),
            &format!(
                "  \"description\": \"\",\n  \"access\": \"protected\",\n{},\n  \"enrichments\": [{{\n    \"mapEnrichment\": {{\n      \"origin\": [{{\n        \"name\": \"Bengaluru\",\n        \"description\": \"Karnataka\",\n        \"latitudeE7\": 129716000,\n        \"longitudeE7\": 775946000\n      }}],\n      \"destination\": [{{\n        \"name\": \"Mysuru\",\n        \"description\": \"Karnataka\",\n        \"latitudeE7\": 122958000,\n        \"longitudeE7\": 766394000\n      }}]\n    }}\n  }}]",
                date(t23 + 9000)
            ),
        )),
        &[mp],
    );
    // `{}` (29 albums): the folder's name is the title.
    g.album("Weekend Hike", "Weekend Hike", Some("{}".into()), &[hike]);
    // `"title": ""` (5 albums): the folder's name.
    g.album(
        "Untitled(3)",
        "Untitled(3)",
        Some(album_meta(Some(""), "")),
        &[untitled],
    );
    // A shared album: comments, a title, and one photo that is in no year folder.
    g.album(
        "Shared Picnic",
        "Shared Picnic",
        Some(album_meta(
            Some("Shared Picnic"),
            &format!(
                "  \"sharedAlbumComments\": [{{\n    \"text\": \"Lovely day!\",\n    \"creationTime\": {{\n      \"timestamp\": \"{}\",\n      \"formatted\": \"{}\"\n    }},\n    \"contentOwnerName\": \"Person B\"\n  }}]",
                t23 + 10_000,
                google_formatted(t23 + 10_000)
            ),
        )),
        &[picnic],
    );
    let t = t23 + 780;
    let mut c = case(
        "in a shared album only (1,119 album files are in no year folder); a messaging-app name, no EXIF",
        true,
        Some(t),
    );
    c.truth.albums = vec!["Shared Picnic".into()];
    g.add(
        "Shared Picnic",
        "IMG-20230615-WA0001.jpg",
        render_plain_jpeg(48, 64, 0x7009),
        Some(supplemental("IMG-20230615-WA0001.jpg", None)),
        "IMG-20230615-WA0001.jpg",
        t,
        &Side {
            origin: Some(SHARED),
            ..Default::default()
        },
        c,
    );
    // Comments (a like) and no title: the folder's name.
    g.album(
        "Road Trip",
        "Road Trip",
        Some(album_meta(
            None,
            &format!(
                "  \"sharedAlbumComments\": [{{\n    \"creationTime\": {{\n      \"timestamp\": \"{}\",\n      \"formatted\": \"{}\"\n    }},\n    \"contentOwnerName\": \"Person C\",\n    \"liked\": true\n  }}]",
                t23 + 11_000,
                google_formatted(t23 + 11_000)
            ),
        )),
        &[road],
    );

    // Edited copies and Live Photo videos end up in their picture's albums.
    for (copy, of) in [(edited_b, mom_b), (mp_video, mp), (road_video, road)] {
        g.cases[copy].truth.albums = g.cases[of].truth.albums.clone();
    }
    // Album copies carry the whole album list of their photo.
    let by_path: std::collections::HashMap<String, Vec<String>> = g
        .cases
        .iter()
        .map(|c| (c.truth.path.clone(), c.truth.albums.clone()))
        .collect();
    for c in g.cases.iter_mut() {
        if let Some(of) = &c.truth.copy_of {
            c.truth.albums = by_path[of].clone();
        }
    }

    // ---- Not media, or not Google Photos ----
    g.tree.put(
        format!("{GOOGLE_ROOT}/shared_album_comments.json"),
        format!(
            "{{\n  \"sharedAlbumComments\": [{{\n    \"text\": \"Lovely day!\",\n    \"creationTime\": {{\n      \"timestamp\": \"{}\",\n      \"formatted\": \"{}\"\n    }},\n    \"contentUrl\": \"https://photos.google.com/share/fixture\"\n  }}]\n}}",
            t23 + 10_000,
            google_formatted(t23 + 10_000)
        ),
    );
    g.tree.put(
        format!("{GOOGLE_ROOT}/user-generated-memory-titles.json"),
        "{\n  \"title\": [\"A weekend away\"]\n}".to_string(),
    );
    // Other products in the same Takeout: a photo in Drive is not part of Google Photos.
    g.tree.put(
        "Takeout/Drive/Scans/IMG_0001.jpg",
        render_jpeg(&exif("Apple", "iPhone 12", local(t23), None), 0x7F01),
    );
    g.tree.put(
        "Takeout/YouTube and YouTube Music/subscriptions/subscriptions.csv",
        "Channel Id,Channel Url,Channel Title\n".to_string(),
    );
    Ok((g.tree, g.cases))
}

// ------------------------------------------------------------------------------------------
// iCloud
// ------------------------------------------------------------------------------------------

/// Apple's `fileChecksum`: 28 base64 characters starting with `A` (its own hash; not ours to
/// recompute, only to carry).
fn checksum(bytes: &[u8]) -> String {
    const B64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let h = blake3::hash(bytes);
    let mut s = String::from("A");
    let mut bits = 0u32;
    let mut n = 0;
    for &b in h.as_bytes() {
        bits = bits << 8 | u32::from(b);
        n += 8;
        while n >= 6 && s.len() < 28 {
            n -= 6;
            s.push(B64[(bits >> n & 63) as usize] as char);
        }
    }
    s
}

fn icloud(samples: &Path) -> io::Result<(Tree, Vec<IcloudCase>)> {
    let mov = std::fs::read(samples.join("clip.mov"))?;
    let heic = std::fs::read(samples.join("photo.heic"))?;
    let mut tree = Tree { files: vec![] };
    let mut cases: Vec<IcloudCase> = vec![];
    // (file name, bytes or None for a missing file, utc, favorite, hidden, deleted)
    struct Row {
        name: &'static str,
        bytes: Option<Vec<u8>>,
        utc: i64,
        favorite: bool,
        hidden: bool,
        deleted: bool,
    }
    // 2015-01-26 12:53:37 UTC: an iPad's camera roll, local time +05:30 in the file. The CSV
    // has the minute only (12:53), so file and CSV differ by 5:30 plus 37 s, as in the real export
    // (every photo with EXIF: 330 or 331 minutes).
    let t = 1_422_276_817;
    let ipad =
        |utc: i64, seed: u64| render_jpeg(&exif("Apple", "iPad mini 2", utc + IST, None), seed);
    let row = |name, bytes, utc, f, h, d| Row {
        name,
        bytes,
        utc,
        favorite: f,
        hidden: h,
        deleted: d,
    };
    let rows = vec![
        row(
            "IMG_0001.JPG",
            Some(ipad(t, 0x1C01)),
            t,
            false,
            false,
            false,
        ),
        row(
            "IMG_0002.JPG",
            Some(ipad(t + 60, 0x1C02)),
            t + 60,
            true,
            false,
            false,
        ),
        row(
            "IMG_0003.JPG",
            Some(render_plain_jpeg(64, 48, 0x1C03)),
            t + 120,
            false,
            false,
            false,
        ),
        row(
            "IMG_0004.PNG",
            Some(screenshot_png()),
            t + 180,
            false,
            false,
            false,
        ),
        row(
            "IMG_0005.MOV",
            Some(retimed_video(&mov, t + 240)),
            t + 240,
            false,
            false,
            false,
        ),
        row(
            "IMG_0006.HEIC",
            Some(heic.clone()),
            t + 300,
            false,
            false,
            false,
        ),
        row(
            "IMG_0006.MOV",
            Some(retimed_video(&mov, t + 300)),
            t + 300,
            false,
            false,
            false,
        ),
        row(
            "IMG_0007.JPG",
            Some(ipad(t + 360, 0x1C07)),
            t + 360,
            false,
            true,
            false,
        ),
        row(
            "IMG_0008.JPG",
            Some(ipad(t + 420, 0x1C08)),
            t + 420,
            false,
            false,
            true,
        ),
        row(
            "IMG_0009.JPG",
            Some(ipad(t + 480, 0x1C09)),
            t + 480,
            false,
            false,
            false,
        ),
        row(
            "IMG_0010.JPG",
            Some(ipad(t + 540, 0x1C0A)),
            t + 540,
            false,
            false,
            false,
        ),
        row("IMG_0011.JPG", None, t + 600, false, false, false),
    ];
    let album = (
        "Trip 2015",
        ["IMG_0009.JPG", "IMG_0010.JPG", "IMG_0006.HEIC"],
    );
    let notes = |name: &str| -> (&'static str, bool) {
        match name {
            "IMG_0001.JPG" => ("camera photo: EXIF in local time with no zone, the CSV in GMT cut to the minute (5:30 and 37 s apart: a zone, so the file's date stays)", true),
            "IMG_0002.JPG" => ("`favorite` = `yes` (every row of the real export said `no`)", false),
            "IMG_0003.JPG" => ("JPEG with no EXIF (50 of 365 in the real export): the CSV's date", true),
            "IMG_0004.PNG" => ("screenshot (PNG, no EXIF): the CSV's date", true),
            "IMG_0005.MOV" => ("a video on its own", true),
            "IMG_0006.HEIC" => ("Live Photo: the HEIC (the real export, from an iPad of 2015, had no HEIC or Live Photo)", false),
            "IMG_0006.MOV" => ("Live Photo: the MOV with the same name; joins the HEIC's albums", false),
            "IMG_0007.JPG" => ("`hidden` = `yes`: imported into \"Hidden (from iCloud)\"", false),
            "IMG_0008.JPG" => ("`deleted` = `yes` (Recently Deleted): not imported", false),
            "IMG_0009.JPG" | "IMG_0010.JPG" => ("in `Albums/Trip 2015.csv` (Apple's guides; the real export had no albums) and in a Memory (ignored)", false),
            "IMG_0011.JPG" => ("a CSV row whose file is in no part: reported, not fatal", false),
            _ => unreachable!(),
        }
    };
    for r in &rows {
        let path = format!("{ICLOUD_ROOT}/Photos/{}", r.name);
        let has_exif = r.name.ends_with(".JPG") && !matches!(r.name, "IMG_0003.JPG");
        let is_photo = r.name.ends_with(".JPG") || r.name.ends_with(".PNG");
        let mut albums = vec![];
        if album.1.contains(&r.name) || r.name == "IMG_0006.MOV" {
            albums.push(album.0.to_owned());
        }
        if r.hidden {
            albums.push("Hidden (from iCloud)".to_owned());
        }
        let (note, seen) = notes(r.name);
        cases.push(IcloudCase {
            path: path.clone(),
            missing: r.bytes.is_none(),
            favorite: r.favorite,
            hidden: r.hidden,
            deleted: r.deleted,
            albums,
            live_photo_of: (r.name == "IMG_0006.MOV")
                .then(|| format!("{ICLOUD_ROOT}/Photos/IMG_0006.HEIC")),
            taken_at_ms: match (r.bytes.is_some(), has_exif, is_photo) {
                (true, true, _) => Some((r.utc + IST) * 1000),
                (true, false, true) => Some(r.utc / 60 * 60 * 1000),
                _ => None,
            },
            blake3: r.bytes.as_deref().map(hex).unwrap_or_default(),
            case: note.into(),
            seen,
        });
        if let Some(b) = &r.bytes {
            tree.put(path, b.clone());
        }
    }
    // Photo Details.csv: LF, no BOM, quoted dates with a comma and no space after it; the
    // rows are in no particular order.
    let mut csv = String::from(
        "imgName,fileChecksum,favorite,hidden,deleted,originalCreationDate,viewCount,importDate\n",
    );
    let yn = |b: bool| if b { "yes" } else { "no" };
    let mut order: Vec<usize> = (0..rows.len()).collect();
    order.sort_by_key(|&i| (i * 7) % rows.len());
    for i in order {
        let r = &rows[i];
        let sum = checksum(r.bytes.as_deref().unwrap_or(r.name.as_bytes()));
        let d = icloud_date(r.utc);
        csv.push_str(&format!(
            "{},{sum},{},{},{},\"{d}\",0,\"{d}\"\n",
            r.name,
            yn(r.favorite),
            yn(r.hidden),
            yn(r.deleted)
        ));
    }
    tree.put(format!("{ICLOUD_ROOT}/Photos/Photo Details.csv"), csv);
    // A Memory: one CSV named by its date, header `imageName`.
    tree.put(
        format!("{ICLOUD_ROOT}/Memories/26 January 2015.csv"),
        "imageName\nIMG_0010.JPG\nIMG_0009.JPG\n".to_string(),
    );
    // An album, as Apple's file guides describe it (header `Images`); not seen in a real export.
    tree.put(
        format!("{ICLOUD_ROOT}/Albums/{}.csv", album.0),
        format!("Images\n{}\n", album.1.join("\n")),
    );
    Ok((tree, cases))
}

/// Write `google-takeout/` and `icloud/` into `out` (each replaced whole), with `truth.json`.
pub fn generate(out: &Path, samples: &Path) -> io::Result<Summary> {
    let (gt, gc) = google(samples)?;
    let (it, ic) = icloud(samples)?;
    let mut bytes = 0;
    for (name, tree, truth) in [
        (
            "google-takeout",
            &gt,
            serde_json::to_vec_pretty(&gc).map_err(io::Error::other)?,
        ),
        (
            "icloud",
            &it,
            serde_json::to_vec_pretty(&ic).map_err(io::Error::other)?,
        ),
    ] {
        let dir = out.join(name);
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
        }
        bytes += tree.write(&dir)?;
        std::fs::write(dir.join("truth.json"), truth)?;
    }
    Ok(Summary {
        google_files: gt.files.len(),
        icloud_files: it.files.len(),
        bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecar_names_are_cut_like_takeout() {
        assert_eq!(
            supplemental("IMG_20190207_101500.jpg", None),
            "IMG_20190207_101500.jpg.supplemental-metadata.json"
        );
        assert_eq!(
            supplemental("PXL_20230615_101000123.jpg", None),
            "PXL_20230615_101000123.jpg.supplemental-metada.json"
        );
        assert_eq!(
            supplemental("PXL_20230615_101100456.MP.jpg", None),
            "PXL_20230615_101100456.MP.jpg.supplemental-met.json"
        );
        assert_eq!(
            supplemental("DSC_0101.JPG", Some(1)),
            "DSC_0101.JPG.supplemental-metadata(1).json"
        );
    }

    #[test]
    fn dates_are_formatted_like_the_exports() {
        assert_eq!(
            icloud_date(1_443_094_620),
            "Thursday September 24,2015 11:37 AM GMT"
        );
        assert_eq!(google_formatted(1_549_514_700), "7 Feb 2019, 04:45:00 UTC");
        assert_eq!(stamp(1_549_514_700 + IST), "20190207_101500");
    }

    #[test]
    fn checksums_look_like_apples() {
        let c = checksum(b"x");
        assert_eq!(c.len(), 28);
        assert!(c.starts_with('A'));
    }
}
