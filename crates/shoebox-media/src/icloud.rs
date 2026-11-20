//! iCloud Photos data export reader (I1-06): a pure function from the entries of an export (every
//! zip, or an unpacked folder) and the contents of its CSV files to the same plan the Takeout
//! reader makes, so one import applies both.
//!
//! The format is recorded in `docs/notes/icloud-format.md`; `fixtures/exports/icloud` has one case
//! per finding. In short:
//!
//! - **`Photo Details.csv`** (header `imgName,…`) lists every file of the folder it lies in, with
//!   `favorite`, `hidden` and `deleted` (`yes`/`no`) and `originalCreationDate`, which is UTC cut
//!   to the minute (`"Monday January 26,2015 12:53 PM GMT"`). Its rows are found by name, in the
//!   same folder first, so an export whose parts each have their own root still matches.
//! - Photos marked `deleted` (Recently Deleted) are left out, as Takeout's bin is. Hidden photos
//!   are imported into the album "Hidden (from iCloud)" (I1 decision): Shoebox has no hidden
//!   flag, and leaving them out would lose them.
//! - **Albums** are `Albums/<title>.csv`, one name per line under a header (`Images` in Apple's
//!   guides, `imgName` or `imageName` per other tools). `Favorites.csv` and `Hidden.csv` (header
//!   `Images`) add to the flags in the details.
//! - A **Live Photo** is a `.HEIC`/`.JPG` and a `.MOV` with the same name; the video joins its
//!   picture's albums.
//! - Memories, shared albums (`AlbumInfo.json`, `Subscribed Albums.json`) and anything outside the
//!   folders that hold the details are not imported (I1 decision).

use crate::takeout::{Entry, Left, PlanAlbum, PlanItem, Rule, Sidecar, Source, TakeoutPlan};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// The album hidden photos are imported into.
pub const HIDDEN_ALBUM: &str = "Hidden (from iCloud)";

const STILLS: &[&str] = &["heic", "heif", "jpg", "jpeg"];
const LIVE_VIDEOS: &[&str] = &["mov", "mp4"];

/// One row of `Photo Details.csv`.
#[derive(Clone, Debug, Default, PartialEq)]
struct Row {
    favorite: bool,
    hidden: bool,
    deleted: bool,
    taken_at_ms: Option<i64>,
}

fn split_dir(path: &str) -> (&str, &str) {
    match path.rsplit_once('/') {
        Some((d, n)) => (d, n),
        None => ("", path),
    }
}

fn last_component(dir: &str) -> &str {
    dir.rsplit('/').next().unwrap_or(dir)
}

fn ext_lower(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, e)| e.to_lowercase())
        .unwrap_or_default()
}

fn stem_lower(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(s, _)| s)
        .unwrap_or(name)
        .to_lowercase()
}

/// The records of a CSV file: comma-separated, fields optionally in double quotes (`""` for a
/// quote), LF or CRLF line ends, a UTF-8 BOM skipped. Blank lines are dropped.
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut rows = vec![];
    let mut row: Vec<String> = vec![];
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => quoted = false,
                _ => field.push(c),
            }
            continue;
        }
        match c {
            '"' => quoted = true,
            ',' => row.push(std::mem::take(&mut field)),
            '\r' => {}
            '\n' => {
                row.push(std::mem::take(&mut field));
                if !(row.len() == 1 && row[0].is_empty()) {
                    rows.push(std::mem::take(&mut row));
                }
                row.clear();
            }
            _ => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

/// Days since 1970-01-01 of a date in the proleptic Gregorian calendar.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `"Monday January 26,2015 12:53 PM GMT"` → UTC milliseconds. The weekday is ignored and may be
/// missing, a space after the comma is allowed, and the zone must be GMT or UTC (or absent).
pub fn parse_date(s: &str) -> Option<i64> {
    const MONTHS: [&str; 12] = [
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ];
    let cleaned = s.replace(',', " ");
    let mut words: Vec<String> = cleaned.split_whitespace().map(str::to_lowercase).collect();
    let month_at = words.iter().position(|w| {
        MONTHS
            .iter()
            .any(|m| m == w || (w.len() >= 3 && m.starts_with(w.as_str())))
    })?;
    words.drain(..month_at);
    if let Some(zone) = words.last() {
        if zone == "gmt" || zone == "utc" {
            words.pop();
        } else if zone != "am" && zone != "pm" {
            return None;
        }
    }
    let [month, day, year, time, half] = &words[..] else {
        return None;
    };
    let month = MONTHS
        .iter()
        .position(|m| m == month || m.starts_with(month.as_str()))? as i64
        + 1;
    let day: i64 = day.parse().ok().filter(|d| (1..=31).contains(d))?;
    let year: i64 = year.parse().ok().filter(|y| (1800..=9999).contains(y))?;
    let (h, min) = time.split_once(':')?;
    let (h, min): (i64, i64) = (h.parse().ok()?, min.parse().ok()?);
    if !(1..=12).contains(&h) || !(0..60).contains(&min) {
        return None;
    }
    let h = match half.as_str() {
        "am" => h % 12,
        "pm" => h % 12 + 12,
        _ => return None,
    };
    let secs = (days_from_civil(year, month, day) * 24 + h) * 3600 + min * 60;
    Some(secs * 1000)
}

fn yes(s: &str) -> bool {
    s.trim().eq_ignore_ascii_case("yes")
}

/// A details file's rows by name, if `records` is one (its header starts with `imgName` and has
/// the details' columns).
fn details(records: &[Vec<String>]) -> Option<BTreeMap<String, Row>> {
    let (header, rows) = records.split_first()?;
    let col = |name: &str| header.iter().position(|h| h.trim() == name);
    let name = col("imgName").filter(|&c| c == 0)?;
    // An album's list may also be headed `imgName`; the details have more columns than that.
    if ![
        "fileChecksum",
        "favorite",
        "hidden",
        "deleted",
        "originalCreationDate",
    ]
    .iter()
    .any(|c| col(c).is_some())
    {
        return None;
    }
    let (fav, hid, del, date) = (
        col("favorite"),
        col("hidden"),
        col("deleted"),
        col("originalCreationDate"),
    );
    fn get(r: &[String], c: Option<usize>) -> Option<&str> {
        c.and_then(|c| r.get(c)).map(String::as_str)
    }
    Some(
        rows.iter()
            .filter_map(|r| {
                let n = r.get(name)?.trim();
                (!n.is_empty()).then(|| {
                    (
                        n.to_owned(),
                        Row {
                            favorite: get(r, fav).is_some_and(yes),
                            hidden: get(r, hid).is_some_and(yes),
                            deleted: get(r, del).is_some_and(yes),
                            taken_at_ms: get(r, date).and_then(parse_date),
                        },
                    )
                })
            })
            .collect(),
    )
}

/// The names in a one-column list (an album, `Favorites.csv`, `Hidden.csv`), if `records` is one.
fn list(records: &[Vec<String>]) -> Option<Vec<String>> {
    let (header, rows) = records.split_first()?;
    let known = ["images", "imgname", "imagename"];
    if !known.contains(&header.first()?.trim().to_lowercase().as_str()) {
        return None;
    }
    Some(
        rows.iter()
            .filter_map(|r| r.first())
            .map(|n| n.trim().to_owned())
            .filter(|n| !n.is_empty())
            .collect(),
    )
}

/// What a CSV file of the export is.
enum Csv {
    Details(BTreeMap<String, Row>),
    Album(String, Vec<String>),
    Favorites(Vec<String>),
    Hidden(Vec<String>),
    /// Memories, or something we do not use.
    Other,
}

fn classify(path: &str, bytes: &[u8]) -> Csv {
    let records = parse_csv(&String::from_utf8_lossy(bytes));
    if let Some(rows) = details(&records) {
        return Csv::Details(rows);
    }
    let (dir, name) = split_dir(path);
    let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name).trim();
    let Some(names) = list(&records) else {
        return Csv::Other;
    };
    match stem.to_lowercase().as_str() {
        "favorites" | "favourites" => return Csv::Favorites(names),
        "hidden" => return Csv::Hidden(names),
        _ => {}
    }
    if last_component(dir).eq_ignore_ascii_case("albums") && !stem.is_empty() {
        Csv::Album(stem.to_owned(), names)
    } else {
        Csv::Other
    }
}

/// Build the plan, or `None` when the entries hold no `Photo Details.csv` (not an iCloud export).
/// `read_csv(i)` returns the bytes of entry `i`; it is called once for every entry whose name ends
/// in `.csv`, and for no other.
pub fn plan(
    entries: &[Entry],
    mut read_csv: impl FnMut(usize) -> Option<Vec<u8>>,
) -> Option<TakeoutPlan> {
    let mut csvs: Vec<(usize, Csv)> = vec![];
    for (i, e) in entries.iter().enumerate() {
        if e.path.to_lowercase().ends_with(".csv") {
            let csv = read_csv(i)
                .map(|b| classify(&e.path, &b))
                .unwrap_or(Csv::Other);
            csvs.push((i, csv));
        }
    }
    // Name → (folder, details entry, row), every details file's rows.
    let mut rows: HashMap<&str, Vec<(&str, usize, &Row)>> = HashMap::new();
    let mut photo_dirs: BTreeSet<&str> = BTreeSet::new();
    let mut photo_dir_names: BTreeSet<String> = BTreeSet::new();
    for (i, csv) in &csvs {
        if let Csv::Details(r) = csv {
            let (dir, _) = split_dir(&entries[*i].path);
            photo_dirs.insert(dir);
            photo_dir_names.insert(last_component(dir).to_lowercase());
            for (name, row) in r {
                rows.entry(name.as_str()).or_default().push((dir, *i, row));
            }
        }
    }
    if photo_dirs.is_empty() {
        return None;
    }
    let mut out = TakeoutPlan {
        source: Source::Icloud,
        ..Default::default()
    };
    let mut favorites: BTreeSet<&str> = BTreeSet::new();
    let mut hidden: BTreeSet<&str> = BTreeSet::new();
    let mut album_names: Vec<&Vec<String>> = vec![];
    for (i, csv) in &csvs {
        match csv {
            Csv::Details(_) => {}
            Csv::Album(title, names) => {
                out.albums.push(PlanAlbum {
                    title: title.clone(),
                    description: None,
                    folder: entries[*i].path.clone(),
                });
                album_names.push(names);
            }
            Csv::Favorites(names) => favorites.extend(names.iter().map(String::as_str)),
            Csv::Hidden(names) => hidden.extend(names.iter().map(String::as_str)),
            Csv::Other => out.left.push((*i, Left::NotPhotos)),
        }
    }

    // The media: every other file in a folder that holds the details, or in a folder of the same
    // name in another part ("Photos").
    let mut media: Vec<usize> = vec![];
    for (i, e) in entries.iter().enumerate() {
        let (dir, name) = split_dir(&e.path);
        if name.is_empty() || name.to_lowercase().ends_with(".csv") {
            continue;
        }
        let in_photos = photo_dirs.contains(dir)
            || photo_dir_names.contains(&last_component(dir).to_lowercase());
        if in_photos && !name.starts_with('.') {
            media.push(i);
        } else {
            out.left.push((i, Left::NotPhotos));
        }
    }

    // Each file's row: the one in its own folder, else the only one with its name.
    let mut found: HashMap<&str, usize> = HashMap::new();
    let mut hidden_any = false;
    let mut kept: Vec<usize> = vec![];
    let mut sidecar_of: HashMap<usize, (usize, Row)> = HashMap::new();
    for &m in &media {
        let (dir, name) = split_dir(&entries[m].path);
        let candidates = rows.get(name).map(Vec::as_slice).unwrap_or(&[]);
        let row = candidates
            .iter()
            .find(|(d, _, _)| *d == dir)
            .or(match candidates {
                [one] => Some(one),
                _ => None,
            });
        match row {
            Some(&(_, csv, row)) => {
                *found.entry(name).or_default() += 1;
                let mut row = row.clone();
                row.favorite |= favorites.contains(name);
                row.hidden |= hidden.contains(name);
                if row.deleted {
                    out.left.push((m, Left::Trashed));
                    continue;
                }
                hidden_any |= row.hidden;
                sidecar_of.insert(m, (csv, row));
            }
            None if candidates.len() > 1 => {
                out.ambiguous.push((m, candidates[0].1));
            }
            None => out.unmatched.push(m),
        }
        kept.push(m);
    }
    out.missing = rows
        .iter()
        .filter(|(name, _)| !found.contains_key(*name))
        .filter(|(_, r)| r.iter().any(|(_, _, row)| !row.deleted))
        .map(|(name, _)| name.to_string())
        .collect();
    out.missing.sort();
    let hidden_album = hidden_any.then(|| {
        out.albums.push(PlanAlbum {
            title: HIDDEN_ALBUM.to_owned(),
            description: None,
            folder: String::new(),
        });
        out.albums.len() - 1
    });

    // Items, one per file (an iCloud export has no copies).
    let mut item_of: HashMap<usize, usize> = HashMap::new();
    for &m in &kept {
        let name = split_dir(&entries[m].path).1;
        let (sidecar, meta, is_hidden) = match sidecar_of.get(&m) {
            Some((csv, row)) => (
                Some((*csv, Rule::Listed)),
                Sidecar {
                    title: Some(name.to_owned()),
                    taken_at_ms: row.taken_at_ms,
                    favorite: row.favorite,
                    ..Default::default()
                },
                row.hidden,
            ),
            None => (
                None,
                Sidecar {
                    favorite: favorites.contains(name),
                    ..Default::default()
                },
                hidden.contains(name),
            ),
        };
        let mut albums: Vec<usize> = album_names
            .iter()
            .enumerate()
            .filter(|(_, names)| names.iter().any(|n| n == name))
            .map(|(a, _)| a)
            .collect();
        if is_hidden {
            albums.extend(hidden_album);
        }
        item_of.insert(m, out.items.len());
        out.items.push(PlanItem {
            entries: vec![m],
            sidecar,
            meta,
            albums,
            edit_of: None,
            live_photo_of: None,
        });
    }

    // Live Photos: a video beside a picture with its name; it joins the picture's albums.
    let mut by_dir: HashMap<&str, Vec<usize>> = HashMap::new();
    for &m in &kept {
        by_dir
            .entry(split_dir(&entries[m].path).0)
            .or_default()
            .push(m);
    }
    for &m in &kept {
        let (dir, name) = split_dir(&entries[m].path);
        if !LIVE_VIDEOS.contains(&ext_lower(name).as_str()) {
            continue;
        }
        let stem = stem_lower(name);
        let still = STILLS.iter().find_map(|ext| {
            by_dir[dir].iter().copied().find(|&o| {
                let n = split_dir(&entries[o].path).1;
                stem_lower(n) == stem && ext_lower(n) == *ext
            })
        });
        if let Some(still) = still {
            let (v, p) = (item_of[&m], item_of[&still]);
            out.items[v].live_photo_of = Some(p);
            let more = out.items[p].albums.clone();
            let albums = &mut out.items[v].albums;
            albums.extend(more);
            albums.sort_unstable();
            albums.dedup();
        }
    }
    out.unmatched.sort_unstable();
    out.ambiguous.sort_unstable();
    out.left.sort();
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        // 2015-01-26 12:53 UTC.
        let t = 1_422_276_780_000;
        assert_eq!(parse_date("Monday January 26,2015 12:53 PM GMT"), Some(t));
        assert_eq!(parse_date("Monday January 26, 2015 12:53 PM GMT"), Some(t));
        assert_eq!(parse_date("January 26,2015 12:53 PM"), Some(t));
        assert_eq!(
            parse_date("Thursday September 24,2015 11:37 AM GMT"),
            Some(1_443_094_620_000)
        );
        assert_eq!(
            parse_date("Monday January 26,2015 12:05 AM GMT"),
            Some(t - 12 * 3_600_000 - 48 * 60_000)
        );
        assert_eq!(parse_date("Monday January 26,2015 12:53 PM PST"), None);
        assert_eq!(parse_date("26/01/2015 12:53"), None);
        assert_eq!(parse_date(""), None);
    }

    #[test]
    fn csv() {
        let rows = parse_csv("\u{feff}a,b\r\n\"x, y\",\"say \"\"hi\"\"\"\r\n\r\nlast,\n");
        assert_eq!(
            rows,
            vec![
                vec!["a".to_owned(), "b".into()],
                vec!["x, y".into(), "say \"hi\"".into()],
                vec!["last".into(), "".into()],
            ]
        );
    }
}
