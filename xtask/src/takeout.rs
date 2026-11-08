//! `cargo xtask fixtures --preset takeout`: a synthetic Google Takeout in several zip parts with
//! every quirk in `docs/notes/takeout-format.md`, and `truth.json` saying what an import must make
//! of it (I1-01, I1-05). Built from the fixture corpus; no real export is ever committed.

use crate::corpus::plan as corpus_plan;
use crate::exifgen::civil_datetime;
use crate::jpeg::render_jpeg;
use crate::samples::screenshot_png;
use serde::Serialize;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;

/// The localised root as the May 2024 export spelled it: a no-break space.
pub const ROOT: &str = "Takeout/Google\u{a0}Photos";

/// What one media entry of the export must become.
#[derive(Clone, Debug, Default, Serialize, serde::Deserialize, PartialEq)]
pub struct Truth {
    /// The media entry's path in its part.
    pub path: String,
    pub part: usize,
    /// The sidecar the reader must pick (its path), or none.
    pub sidecar: Option<String>,
    /// Titles of the albums the photo ends up in.
    pub albums: Vec<String>,
    pub favorite: bool,
    /// Trashed photos are not imported.
    pub trashed: bool,
    pub archived: bool,
    /// The date the photo must get, after the file's and the export's are weighed (UTC ms).
    pub taken_at_ms: Option<i64>,
    /// The place the photo must get from the export (it has none of its own), as (lat, lon).
    pub place: Option<(f64, f64)>,
    /// For an edited copy: the path of its original.
    pub edit_of: Option<String>,
    /// For the video of a Live Photo: the path of its picture.
    pub live_photo_of: Option<String>,
    /// The BLAKE3 of the bytes (the photo's id).
    pub blake3: String,
    /// Another entry with these same bytes (a copy in an album folder): same photo.
    pub copy_of: Option<String>,
}

#[derive(Debug)]
pub struct Summary {
    pub parts: Vec<PathBuf>,
    pub entries: usize,
    pub truth: PathBuf,
}

struct File {
    path: String,
    bytes: Vec<u8>,
}

fn sidecar_json(title: &str, taken_secs: i64, gps: Option<(f64, f64)>, extra: &str) -> Vec<u8> {
    let (lat, lon) = gps.unwrap_or((0.0, 0.0));
    format!(
        r#"{{
  "title": "{title}",
  "description": "",
  "imageViews": "3",
  "creationTime": {{ "timestamp": "1730000000", "formatted": "27 Oct 2024, 03:33:20 UTC" }},
  "photoTakenTime": {{ "timestamp": "{taken_secs}", "formatted": "(localised text, never parsed)" }},
  "geoData": {{ "latitude": {lat}, "longitude": {lon}, "altitude": 0.0, "latitudeSpan": 0.0, "longitudeSpan": 0.0 }},
  "geoDataExif": {{ "latitude": {lat}, "longitude": {lon}, "altitude": 0.0, "latitudeSpan": 0.0, "longitudeSpan": 0.0 }},
  "url": "https://photos.google.com/photo/fixture"{extra}
}}"#
    )
    .into_bytes()
}

fn album_json(title: &str) -> Vec<u8> {
    format!(
        r#"{{ "title": "{title}", "description": "", "access": "protected",
  "date": {{ "timestamp": "1730000000", "formatted": "x" }}, "location": "",
  "geoData": {{ "latitude": 0, "longitude": 0, "altitude": 0.0, "latitudeSpan": 0.0, "longitudeSpan": 0.0 }} }}"#
    )
    .into_bytes()
}

fn year_dir(secs: i64) -> String {
    format!("{ROOT}/Photos from {}", civil_datetime(secs).0)
}

fn hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

/// Write the export: `parts` zips into `out_dir`, and `truth.json`. `samples` is the checked-in
/// `fixtures/samples` folder (for a short video).
pub fn generate(out_dir: &Path, samples: &Path, seed: u64, parts: usize) -> io::Result<Summary> {
    let parts = parts.max(2);
    let specs = corpus_plan(30, seed, 0);
    let clip = std::fs::read(samples.join("clip.mp4"))?;
    let mut files: Vec<File> = vec![];
    let mut truth: Vec<Truth> = vec![];
    let add = |files: &mut Vec<File>, path: String, bytes: Vec<u8>| {
        files.push(File { path, bytes });
    };

    // Photo `i` of the corpus, in its year folder, with a sidecar named by `sidecar` (from the
    // media file's name) unless that is None.
    let photo = |files: &mut Vec<File>,
                 truth: &mut Vec<Truth>,
                 i: usize,
                 name: &str,
                 sidecar: Option<&dyn Fn(&str) -> String>,
                 taken_secs: Option<i64>,
                 extra: &str| {
        let spec = &specs[i];
        let bytes = render_jpeg(&spec.exif, spec.content_seed);
        let dir = year_dir(spec.exif.taken_at_secs);
        let path = format!("{dir}/{name}");
        let export_secs = taken_secs.unwrap_or(spec.exif.taken_at_secs);
        let sidecar_path = sidecar.map(|f| format!("{dir}/{}", f(name)));
        if let Some(s) = &sidecar_path {
            files.push(File {
                path: s.clone(),
                bytes: sidecar_json(name, export_secs, spec.exif.gps, extra),
            });
        }
        let t = Truth {
            path: path.clone(),
            sidecar: sidecar_path,
            taken_at_ms: Some(
                if taken_secs.is_some_and(|t| (t - spec.exif.taken_at_secs).rem_euclid(900) != 0) {
                    export_secs * 1000
                } else {
                    spec.exif.taken_at_secs * 1000
                },
            ),
            favorite: extra.contains("\"favorited\": true"),
            trashed: extra.contains("\"trashed\": true"),
            archived: extra.contains("\"archived\": true"),
            blake3: hex(&bytes),
            ..Default::default()
        };
        files.push(File { path, bytes });
        truth.push(t);
        truth.len() - 1
    };
    let supplemental: &dyn Fn(&str) -> String = &|n| format!("{n}.supplemental-metadata.json");
    let old: &dyn Fn(&str) -> String = &|n| format!("{n}.json");

    // 0–1: plain, current and old sidecar names.
    photo(
        &mut files,
        &mut truth,
        0,
        "IMG_20240101_101010.jpg",
        Some(supplemental),
        None,
        "",
    );
    photo(
        &mut files,
        &mut truth,
        1,
        "DSC_0001.JPG",
        Some(old),
        None,
        "",
    );
    // 2: a long name; the sidecar is cut at 46 UTF-16 units (the cut leaves a trailing dot).
    let long = "PXL_20230814_201154491.LONG_EXPOSURE-01.COVER.jpg";
    photo(
        &mut files,
        &mut truth,
        2,
        long,
        Some(&|_| "PXL_20230814_201154491.LONG_EXPOSURE-01.COVER..json".into()),
        None,
        "",
    );
    // 3: emoji: 23 of them are 46 units.
    let emoji = "😀😃😄😁😆😅😂🤣🥲☺️😊😇🙂🙃😉😌😍🥰😘😗😙😚😋😛.jpg";
    photo(
        &mut files,
        &mut truth,
        3,
        emoji,
        Some(&|_| "😀😃😄😁😆😅😂🤣🥲☺️😊😇🙂🙃😉😌😍🥰😘😗😙😚😋.json".into()),
        None,
        "",
    );
    // 4–5: the same name twice in one year: the number moves.
    let a = photo(
        &mut files,
        &mut truth,
        4,
        "IMG_0042.jpg",
        Some(supplemental),
        None,
        "",
    );
    let year4 = year_dir(specs[4].exif.taken_at_secs);
    // Put photo 5 into photo 4's year folder under the same name.
    {
        let spec = &specs[5];
        let bytes = render_jpeg(&spec.exif, spec.content_seed);
        let path = format!("{year4}/IMG_0042(1).jpg");
        let sidecar = format!("{year4}/IMG_0042.jpg.supplemental-metadata(1).json");
        add(
            &mut files,
            sidecar.clone(),
            sidecar_json("IMG_0042.jpg", spec.exif.taken_at_secs, spec.exif.gps, ""),
        );
        truth.push(Truth {
            path: path.clone(),
            sidecar: Some(sidecar),
            taken_at_ms: Some(spec.exif.taken_at_secs * 1000),
            blake3: hex(&bytes),
            ..Default::default()
        });
        add(&mut files, path, bytes);
    }
    // 6: edited in Google Photos: other bytes, no sidecar, the original's albums.
    let orig = photo(
        &mut files,
        &mut truth,
        6,
        "IMG_0600.jpg",
        Some(supplemental),
        None,
        "",
    );
    {
        let spec = &specs[6];
        let bytes = render_jpeg(&spec.exif, spec.content_seed ^ 0xED17);
        let dir = year_dir(spec.exif.taken_at_secs);
        let path = format!("{dir}/IMG_0600-edited.jpg");
        truth.push(Truth {
            path: path.clone(),
            sidecar: Some(truth[orig].sidecar.clone().unwrap()),
            taken_at_ms: Some(spec.exif.taken_at_secs * 1000),
            edit_of: Some(truth[orig].path.clone()),
            blake3: hex(&bytes),
            ..Default::default()
        });
        add(&mut files, path, bytes);
    }
    // 7: a favourite.
    let fav = photo(
        &mut files,
        &mut truth,
        7,
        "IMG_0700.jpg",
        Some(supplemental),
        None,
        ",\n  \"favorited\": true",
    );
    // 8: re-dated by hand (three days and some minutes off): the export's date wins.
    let redated = specs[8].exif.taken_at_secs - 3 * 86_400 - 17 * 60;
    photo(
        &mut files,
        &mut truth,
        8,
        "SCAN_0001.jpg",
        Some(supplemental),
        Some(redated),
        "",
    );
    // 9: two hours off (a time zone): the file's date stays.
    let zoned = specs[9].exif.taken_at_secs + 2 * 3600;
    photo(
        &mut files,
        &mut truth,
        9,
        "IMG_0900.jpg",
        Some(supplemental),
        Some(zoned),
        "",
    );
    // 10: trashed, by its sidecar.
    photo(
        &mut files,
        &mut truth,
        10,
        "IMG_1000.jpg",
        Some(supplemental),
        None,
        ",\n  \"trashed\": true",
    );
    // 11: archived, by its sidecar.
    photo(
        &mut files,
        &mut truth,
        11,
        "IMG_1100.jpg",
        Some(supplemental),
        None,
        ",\n  \"archived\": true",
    );
    // 12: a screenshot with no date of its own: the export's.
    {
        let bytes = screenshot_png();
        let secs = 1_700_000_000;
        let dir = year_dir(secs);
        let path = format!("{dir}/Screenshot_20231114-221320.png");
        let sidecar = format!("{dir}/Screenshot_20231114-221320.png.supplemental-metadata.json");
        // No date and no place of its own: both come from the export.
        let place = (12.9716, 77.5946);
        add(
            &mut files,
            sidecar.clone(),
            sidecar_json("Screenshot_20231114-221320.png", secs, Some(place), ""),
        );
        truth.push(Truth {
            path: path.clone(),
            sidecar: Some(sidecar),
            taken_at_ms: Some(secs * 1000),
            place: Some(place),
            blake3: hex(&bytes),
            ..Default::default()
        });
        add(&mut files, path, bytes);
    }
    // 13: in the Trash folder (an older export's layout).
    {
        let spec = &specs[13];
        let bytes = render_jpeg(&spec.exif, spec.content_seed);
        let path = format!("{ROOT}/Trash/IMG_1300.jpg");
        let sidecar = format!("{ROOT}/Trash/IMG_1300.jpg.json");
        add(
            &mut files,
            sidecar.clone(),
            sidecar_json("IMG_1300.jpg", spec.exif.taken_at_secs, None, ""),
        );
        truth.push(Truth {
            path: path.clone(),
            sidecar: Some(sidecar),
            trashed: true,
            taken_at_ms: Some(spec.exif.taken_at_secs * 1000),
            blake3: hex(&bytes),
            ..Default::default()
        });
        add(&mut files, path, bytes);
    }
    // 14: a Live Photo: a picture and a short video with no sidecar of its own.
    let still = photo(
        &mut files,
        &mut truth,
        14,
        "IMG_1400.jpg",
        Some(supplemental),
        None,
        "",
    );
    {
        let dir = year_dir(specs[14].exif.taken_at_secs);
        let path = format!("{dir}/IMG_1400.MP4");
        truth.push(Truth {
            path: path.clone(),
            sidecar: truth[still].sidecar.clone(),
            live_photo_of: Some(truth[still].path.clone()),
            blake3: hex(&clip),
            ..Default::default()
        });
        add(&mut files, path, clip.clone());
    }
    // 15–29: plain photos, some of them in albums.
    let mut plain = vec![];
    for i in 15..30 {
        plain.push(photo(
            &mut files,
            &mut truth,
            i,
            &format!("IMG_{i:04}.jpg"),
            Some(supplemental),
            None,
            "",
        ));
    }
    // Albums: copies of their photos with their own sidecars; the folder name is cut and escaped,
    // or (untitled) the title is empty.
    let albums: [(&str, &str, Vec<usize>); 3] = [
        (
            "Kerala trip",
            "Kerala trip",
            vec![plain[0], plain[1], plain[2], a, orig],
        ),
        (
            "Trip 6-10-23",
            "Trip 6/10/23",
            vec![plain[2], plain[3], fav],
        ),
        ("Untitled(2)", "", vec![plain[4]]),
    ];
    for (folder, title, members) in albums {
        let dir = format!("{ROOT}/{folder}");
        let meta = if folder.starts_with("Untitled") {
            "metadata.json"
        } else {
            "métadonnées.json"
        };
        add(&mut files, format!("{dir}/{meta}"), album_json(title));
        let shown = if title.is_empty() { folder } else { title };
        for m in members {
            let t = truth[m].clone();
            let name = t.path.rsplit('/').next().unwrap().to_owned();
            let bytes = files
                .iter()
                .find(|f| f.path == t.path)
                .unwrap()
                .bytes
                .clone();
            let sidecar = format!("{dir}/{name}.supplemental-metadata.json");
            let src_sidecar = files
                .iter()
                .find(|f| Some(&f.path) == t.sidecar.as_ref())
                .unwrap()
                .bytes
                .clone();
            add(&mut files, sidecar, src_sidecar);
            add(&mut files, format!("{dir}/{name}"), bytes);
            truth[m].albums.push(shown.to_owned());
            truth.push(Truth {
                path: format!("{dir}/{name}"),
                sidecar: Some(format!("{dir}/{name}.supplemental-metadata.json")),
                copy_of: Some(t.path.clone()),
                ..t.clone()
            });
        }
    }
    // The edited copy joins its original's albums.
    let orig_albums = truth[orig].albums.clone();
    for t in truth.iter_mut().filter(|t| t.edit_of.is_some()) {
        t.albums = orig_albums.clone();
    }
    // Album copies carry the full album list of their photo.
    let by_path: std::collections::HashMap<String, Vec<String>> = truth
        .iter()
        .map(|t| (t.path.clone(), t.albums.clone()))
        .collect();
    for t in truth.iter_mut() {
        if let Some(of) = &t.copy_of {
            t.albums = by_path[of].clone();
        }
    }
    // A file that is not part of Google Photos.
    add(
        &mut files,
        "Takeout/archive_browser.html".into(),
        b"<!doctype html><title>Takeout</title>".to_vec(),
    );

    // Parts: deal the files out so many sidecars land in another part than their file; the
    // archive_browser.html goes in every part, as Takeout does.
    let mut part_files: Vec<Vec<usize>> = vec![vec![]; parts];
    for (i, f) in files.iter().enumerate() {
        if f.path.ends_with("archive_browser.html") {
            continue;
        }
        // A file and its sidecar are written one after the other; about half the sidecars go
        // one part further on (160 of 483 did in the May 2024 export).
        let shift = if f.path.ends_with(".json") {
            (i / 6) % 2
        } else {
            0
        };
        let p = (i / 2 + shift) % parts;
        part_files[p].push(i);
    }
    for t in &mut truth {
        t.part = part_files
            .iter()
            .position(|p| p.iter().any(|&i| files[i].path == t.path))
            .unwrap();
    }
    let html = files
        .iter()
        .position(|f| f.path.ends_with("archive_browser.html"))
        .unwrap();

    std::fs::create_dir_all(out_dir)?;
    let mut written = vec![];
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(
            zip::DateTime::from_date_and_time(2024, 10, 27, 3, 33, 20).map_err(io::Error::other)?,
        );
    for (p, members) in part_files.iter().enumerate() {
        let path = out_dir.join(format!("takeout-20241027T033320Z-{:03}.zip", p + 1));
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path)?);
        for &i in members.iter().chain([&html]) {
            zip.start_file(files[i].path.as_str(), options)
                .map_err(io::Error::other)?;
            zip.write_all(&files[i].bytes)?;
        }
        zip.finish().map_err(io::Error::other)?;
        written.push(path);
    }
    let truth_path = out_dir.join("truth.json");
    std::fs::write(
        &truth_path,
        serde_json::to_vec_pretty(&truth).map_err(io::Error::other)?,
    )?;
    Ok(Summary {
        parts: written,
        entries: files.len(),
        truth: truth_path,
    })
}
