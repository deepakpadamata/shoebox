//! Google Takeout reader (I1-05): a pure function from the entries of an export (every part, or an
//! unpacked folder) and the contents of its JSON files to a plan with one item per photo or video.
//!
//! Nothing here reads media or touches a library: the archive layer lists the entries and hands
//! over the JSON files' bytes, and the import applies the plan. The rules are the ones recorded in
//! `docs/notes/takeout-format.md`, each with the sample it was seen in.
//!
//! - **Folders** are classified by what they hold, not by their (localised) names: a folder with
//!   an album metadata file (`metadata.json`, `métadonnées.json`, …: any JSON with a `title` and
//!   no `photoTakenTime`) is an album; the others hold the library by year. Trash and Archive are
//!   recognised by name in the languages we know of and by the sidecars' `trashed` / `archived`.
//! - **Sidecars** are matched in the media file's own folder, strongest rule first, each rule over
//!   every file before the next is tried, so a weaker rule never takes a sidecar a stronger one
//!   gives to another file. Two files claiming one sidecar under the same rule are both left
//!   unmatched and reported, unless the sidecar's `title` names exactly one of them.
//! - **Copies**: the same photo appears in its year folder and in every album folder that holds
//!   it, each with its own sidecar. Copies with the same name and size (and CRC, when the archive
//!   gives one) are one item, in every one of those albums.

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// The longest a sidecar's name is before `.json`, in UTF-16 code units (seen: 46 ASCII characters
/// and 23 emoji, both 46 units).
pub const SIDECAR_STEM_UNITS: usize = 46;

/// The suffix newer exports put between the media file's name and `.json` (cut like the rest).
const SUPPLEMENTAL: &str = ".supplemental-metadata";

/// What Google appends to the name of a copy edited in Google Photos, by language. A name with
/// another language's word is not recognised and imports as a photo of its own (no harm: it keeps
/// its own bytes and, if it has one, its own sidecar).
const EDITED: &[&str] = &[
    "edited",       // en
    "modifié",      // fr (seen: IMG_8416-modifié.JPG)
    "bearbeitet",   // de
    "editado",      // es, pt
    "modificato",   // it
    "bewerkt",      // nl
    "redigeret",    // da
    "redigert",     // nb
    "redigerad",    // sv
    "muokattu",     // fi
    "edytowane",    // pl
    "upraveno",     // cs
    "szerkesztett", // hu
    "düzenlendi",   // tr
    "изменено",     // ru
    "編集済み",     // ja
    "已编辑",       // zh-Hans
    "已編輯",       // zh-Hant
    "편집됨",       // ko
];

/// Folder names of Google Photos' trash and archive, by language (lower case).
const TRASH_FOLDERS: &[&str] = &[
    "trash",
    "bin",
    "corbeille",
    "papierkorb",
    "papelera",
    "lixeira",
    "lixo",
    "cestino",
    "prullenbak",
    "papirkurv",
    "papperskorg",
    "roskakori",
    "kosz",
    "koš",
    "çöp kutusu",
    "корзина",
    "ゴミ箱",
    "回收站",
    "垃圾桶",
    "휴지통",
];
const ARCHIVE_FOLDERS: &[&str] = &[
    "archive",
    "archives",
    "archiv",
    "archivo",
    "arquivo",
    "archivio",
    "archief",
    "arkiv",
    "arkisto",
    "archiwum",
    "arşiv",
    "архив",
    "アーカイブ",
    "归档",
    "封存",
    "보관함",
];

/// One file of the export, as the archive layer lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Which part (zip, tgz or folder) it is in, as numbered by the caller.
    pub part: usize,
    /// Its path inside the part, `/`-separated (`Takeout/Google Photos/Photos from 2019/a.jpg`).
    pub path: String,
    pub size: u64,
    /// The CRC-32 the archive records, when it records one (zip does, a folder does not).
    pub crc32: Option<u32>,
}

/// Where a photo's metadata came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Rule {
    /// `name.json` or `name.supplemental-metadata.json`, cut at 46 units as Google cuts it.
    Exact,
    /// `name(1).jpg` → `name.jpg(1).json` (and the supplemental forms of it).
    MovedNumber,
    /// A Pixel `.MP` file → `name.MP.jpg.json`.
    PixelMp,
    /// No name rule matched, but exactly one unclaimed sidecar in the folder has this `title`.
    Title,
    /// As `Exact`, ignoring case.
    IgnoringCase,
    /// An edited copy (`-edited`): the original's sidecar.
    EditOf,
    /// The video half of a Live Photo: its picture's sidecar.
    LivePhoto,
}

impl Rule {
    /// Whether the sidecar describes this very file (its date and place can be trusted for it).
    pub fn is_own(self) -> bool {
        !matches!(self, Rule::EditOf | Rule::LivePhoto)
    }
}

/// What a sidecar says, as far as the import uses it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sidecar {
    pub title: Option<String>,
    /// `photoTakenTime.timestamp`: UTC, in milliseconds.
    pub taken_at_ms: Option<i64>,
    /// `geoData`, else `geoDataExif`; `(0, 0)` means none.
    pub place: Option<(f64, f64)>,
    pub favorite: bool,
    pub archived: bool,
    pub trashed: bool,
    pub people: Vec<String>,
    pub description: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanAlbum {
    /// From the album's metadata file; the folder's name when that is empty.
    pub title: String,
    pub description: Option<String>,
    /// The album's folder (as in the entries' paths, without the part).
    pub folder: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlanItem {
    /// Every copy of this file in the export (indices into the entries); the first is the one to
    /// read. Copies have the same name and size (and CRC).
    pub entries: Vec<usize>,
    /// The sidecar the metadata came from, and by which rule.
    pub sidecar: Option<(usize, Rule)>,
    pub meta: Sidecar,
    /// Indices into [`TakeoutPlan::albums`].
    pub albums: Vec<usize>,
    /// For an edited copy: the item it is an edit of.
    pub edit_of: Option<usize>,
    /// For the video of a Live Photo: the item of its picture.
    pub live_photo_of: Option<usize>,
}

/// Why an entry is not part of any item.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Left {
    /// In the trash (by folder or by its sidecar).
    Trashed,
    /// Not under a folder that holds Google Photos' sidecars (another product, or the export's
    /// own `archive_browser.html`). An archive with no sidecar at all is not a Takeout: every
    /// file in it is taken.
    NotPhotos,
    /// A JSON file that is neither a sidecar nor an album's metadata, or that could not be read.
    UnreadableJson(String),
    /// A sidecar no media file in its folder took.
    UnusedSidecar,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TakeoutPlan {
    pub items: Vec<PlanItem>,
    pub albums: Vec<PlanAlbum>,
    /// Media entries no rule gave a sidecar (they import with their own metadata only).
    pub unmatched: Vec<usize>,
    /// Media entries left unmatched because they claimed a sidecar together with others, each with
    /// the sidecar.
    pub ambiguous: Vec<(usize, usize)>,
    /// Entries that are in no item, and why.
    pub left: Vec<(usize, Left)>,
}

impl TakeoutPlan {
    /// The item that holds an entry.
    pub fn item_of(&self, entry: usize) -> Option<usize> {
        self.items.iter().position(|i| i.entries.contains(&entry))
    }
}

/// Which date a photo gets, from its file's own and the export's (owner's decision 2026-10-31):
/// the file's own date wins; the export's fills in when the file has none; when both exist and
/// differ by something other than whole hours (a time zone), the user corrected it in Google
/// Photos and the export's wins.
pub fn resolve_taken_at(file_ms: Option<i64>, export_ms: Option<i64>) -> Option<i64> {
    match (file_ms, export_ms) {
        (Some(f), Some(e)) => {
            // The sidecar has whole seconds; EXIF may have sub-seconds. A zone is a multiple of
            // 15 minutes (India +5:30, Nepal +5:45), at most 14 hours.
            let diff = (e - f.div_euclid(1000) * 1000).abs();
            let zone = diff % (15 * 60 * 1000) == 0 && diff <= 14 * 3600 * 1000;
            Some(if zone { f } else { e })
        }
        (Some(f), None) => Some(f),
        (None, e) => e,
    }
}

/// The longest prefix of `s` that is at most `units` UTF-16 code units, not splitting a character.
pub fn cut_utf16(s: &str, units: usize) -> &str {
    let mut used = 0;
    for (i, c) in s.char_indices() {
        used += c.len_utf16();
        if used > units {
            return &s[..i];
        }
    }
    s
}

fn split_dir(path: &str) -> (&str, &str) {
    match path.rfind('/') {
        Some(i) => (&path[..i], &path[i + 1..]),
        None => ("", path),
    }
}

fn last_component(dir: &str) -> &str {
    split_dir(dir).1
}

/// `name(3).jpg` → `("name.jpg", 3)`; `None` without a number before the extension.
fn moved_number(name: &str) -> Option<(String, u32)> {
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    let open = stem.strip_suffix(')')?.rfind('(')?;
    let digits = &stem[open + 1..stem.len() - 1];
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((format!("{}{ext}", &stem[..open]), digits.parse().ok()?))
}

/// The sidecar names Google writes for a media file named `name` (or `name` with a copy number
/// `n` moved out of it), strongest first.
fn sidecar_names(name: &str, number: Option<u32>) -> Vec<String> {
    let n = number.map(|n| format!("({n})")).unwrap_or_default();
    let mut out = vec![
        format!(
            "{}{n}.json",
            cut_utf16(&format!("{name}{SUPPLEMENTAL}"), SIDECAR_STEM_UNITS)
        ),
        format!("{}{n}.json", cut_utf16(name, SIDECAR_STEM_UNITS)),
    ];
    if number.is_some() {
        // Also seen: the number before the suffix, and inside the cut.
        out.push(format!(
            "{}.json",
            cut_utf16(&format!("{name}{n}{SUPPLEMENTAL}"), SIDECAR_STEM_UNITS)
        ));
        out.push(format!(
            "{}.json",
            cut_utf16(&format!("{name}{n}"), SIDECAR_STEM_UNITS)
        ));
    }
    let mut seen = BTreeSet::new();
    out.retain(|s| seen.insert(s.clone()));
    out
}

/// `IMG_1-edited.jpg` / `IMG_1-modifié(1).jpg` → the original's name (`IMG_1.jpg`, `IMG_1(1).jpg`).
fn edited_original(name: &str) -> Option<String> {
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    let (stem, number) = match moved_number(&format!("{stem}.x")) {
        Some((s, n)) => (s.trim_end_matches(".x").to_owned(), format!("({n})")),
        None => (stem.to_owned(), String::new()),
    };
    let lower = stem.to_lowercase();
    EDITED.iter().find_map(|word| {
        let tail = format!("-{word}");
        lower
            .ends_with(&tail)
            .then(|| format!("{}{number}{ext}", &stem[..stem.len() - tail.len()]))
    })
}

const STILLS: &[&str] = &["heic", "heif", "jpg", "jpeg"];
const LIVE_VIDEOS: &[&str] = &["mov", "mp4"];

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

fn timestamp_ms(v: &Value) -> Option<i64> {
    let t = v.get("timestamp")?;
    let secs = match t {
        Value::String(s) => s.trim().parse::<i64>().ok()?,
        Value::Number(n) => n.as_i64()?,
        _ => return None,
    };
    secs.checked_mul(1000)
}

fn place(v: Option<&Value>) -> Option<(f64, f64)> {
    let v = v?;
    let lat = v.get("latitude")?.as_f64()?;
    let lon = v.get("longitude")?.as_f64()?;
    let valid = lat.is_finite() && lon.is_finite() && lat.abs() <= 90.0 && lon.abs() <= 180.0;
    (valid && (lat, lon) != (0.0, 0.0)).then_some((lat, lon))
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned)
}

fn flag(v: &Value, key: &str) -> bool {
    match v.get(key) {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => s == "true",
        _ => false,
    }
}

enum Json {
    Sidecar(Sidecar),
    Album {
        title: Option<String>,
        description: Option<String>,
    },
}

fn parse_json(bytes: &[u8]) -> Result<Json, String> {
    let v: Value = serde_json::from_slice(bytes).map_err(|e| format!("not JSON: {e}"))?;
    if !v.is_object() {
        return Err("not a JSON object".into());
    }
    let is_sidecar = ["photoTakenTime", "creationTime", "imageViews", "url"]
        .iter()
        .any(|k| v.get(*k).is_some());
    if is_sidecar {
        let people = v
            .get("people")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|p| text(p, "name"))
            .collect();
        return Ok(Json::Sidecar(Sidecar {
            title: text(&v, "title"),
            taken_at_ms: v.get("photoTakenTime").and_then(timestamp_ms),
            place: place(v.get("geoData")).or_else(|| place(v.get("geoDataExif"))),
            favorite: flag(&v, "favorited"),
            archived: flag(&v, "archived"),
            trashed: flag(&v, "trashed"),
            people,
            description: text(&v, "description"),
        }));
    }
    if v.get("title").is_some() {
        return Ok(Json::Album {
            title: text(&v, "title"),
            description: text(&v, "description"),
        });
    }
    Err("neither a sidecar nor an album's metadata".into())
}

/// The sidecars one naming rule finds for a media file's name.
type Finder<'a> = &'a dyn Fn(&str) -> Vec<usize>;

#[derive(Default)]
struct Folder {
    media: Vec<usize>,
    /// Sidecar entries by file name.
    sidecars: BTreeMap<String, usize>,
    album: Option<usize>,
}

/// Build the plan. `read_json(i)` returns the bytes of entry `i`; it is called once for every
/// entry whose name ends in `.json`, and for no other.
pub fn plan(entries: &[Entry], mut read_json: impl FnMut(usize) -> Option<Vec<u8>>) -> TakeoutPlan {
    let mut out = TakeoutPlan::default();
    let mut sidecars: HashMap<usize, Sidecar> = HashMap::new();
    let mut folders: BTreeMap<&str, Folder> = BTreeMap::new();

    // 1. Read every JSON file; sort entries into folders.
    let mut media_candidates = vec![];
    for (i, e) in entries.iter().enumerate() {
        let (dir, name) = split_dir(&e.path);
        if name.is_empty() {
            continue;
        }
        if name.to_lowercase().ends_with(".json") {
            match read_json(i).map(|b| parse_json(&b)) {
                Some(Ok(Json::Sidecar(s))) => {
                    sidecars.insert(i, s);
                    folders
                        .entry(dir)
                        .or_default()
                        .sidecars
                        .insert(name.to_owned(), i);
                }
                Some(Ok(Json::Album { title, description })) => {
                    let folder = folders.entry(dir).or_default();
                    if folder.album.is_none() {
                        folder.album = Some(out.albums.len());
                        out.albums.push(PlanAlbum {
                            title: title.unwrap_or_else(|| last_component(dir).to_owned()),
                            description,
                            folder: dir.to_owned(),
                        });
                    } else {
                        out.left.push((
                            i,
                            Left::UnreadableJson("a second album metadata file".into()),
                        ));
                    }
                }
                Some(Err(why)) => out.left.push((i, Left::UnreadableJson(why))),
                None => out
                    .left
                    .push((i, Left::UnreadableJson("could not be read".into()))),
            }
        } else {
            media_candidates.push(i);
        }
    }
    // Google Photos' folders are the siblings of folders that hold its JSON files.
    let photo_roots: BTreeSet<&str> = folders
        .iter()
        .filter(|(_, f)| !f.sidecars.is_empty() || f.album.is_some())
        .map(|(dir, _)| split_dir(dir).0)
        .collect();
    // No sidecar and no album anywhere: not a Takeout but a plain archive of files; all of them.
    let plain = photo_roots.is_empty();
    for i in media_candidates {
        let (dir, _) = split_dir(&entries[i].path);
        if plain || photo_roots.contains(split_dir(dir).0) {
            folders.entry(dir).or_default().media.push(i);
        } else {
            out.left.push((i, Left::NotPhotos));
        }
    }

    // 2. Match sidecars, rule by rule.
    let mut matched: HashMap<usize, (usize, Rule)> = HashMap::new();
    let mut claimed: BTreeSet<usize> = BTreeSet::new();
    let mut ambiguous: BTreeMap<usize, usize> = BTreeMap::new();
    for folder in folders.values() {
        let lower: HashMap<String, Vec<usize>> =
            folder
                .sidecars
                .iter()
                .fold(HashMap::new(), |mut m, (name, &i)| {
                    m.entry(name.to_lowercase()).or_default().push(i);
                    m
                });
        let name_of = |i: usize| split_dir(&entries[i].path).1;
        let rules: [(Rule, Finder); 5] = [
            (Rule::Exact, &|name: &str| {
                sidecar_names(name, None)
                    .iter()
                    .find_map(|s| folder.sidecars.get(s).copied())
                    .into_iter()
                    .collect()
            }),
            (Rule::MovedNumber, &|name: &str| {
                moved_number(name)
                    .map(|(base, n)| {
                        sidecar_names(&base, Some(n))
                            .iter()
                            .find_map(|s| folder.sidecars.get(s).copied())
                            .into_iter()
                            .collect()
                    })
                    .unwrap_or_default()
            }),
            (Rule::PixelMp, &|name: &str| {
                if ext_lower(name) != "mp" {
                    return vec![];
                }
                sidecar_names(&format!("{name}.jpg"), None)
                    .iter()
                    .find_map(|s| folder.sidecars.get(s).copied())
                    .into_iter()
                    .collect()
            }),
            (Rule::Title, &|name: &str| {
                folder
                    .sidecars
                    .values()
                    .copied()
                    .filter(|s| sidecars[s].title.as_deref() == Some(name))
                    .collect()
            }),
            (Rule::IgnoringCase, &|name: &str| {
                let mut names = sidecar_names(name, None);
                if let Some((base, n)) = moved_number(name) {
                    names.extend(sidecar_names(&base, Some(n)));
                }
                names
                    .iter()
                    .find_map(|s| lower.get(&s.to_lowercase()).filter(|v| v.len() == 1))
                    .cloned()
                    .unwrap_or_default()
            }),
        ];
        for (rule, find) in rules {
            // Sidecar → the files that claim it under this rule.
            let mut claims: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
            for &m in &folder.media {
                if matched.contains_key(&m) || ambiguous.contains_key(&m) {
                    continue;
                }
                let found: Vec<usize> = find(name_of(m))
                    .into_iter()
                    .filter(|s| !claimed.contains(s))
                    .collect();
                // A title shared by several sidecars decides nothing.
                if let [s] = found[..] {
                    claims.entry(s).or_default().push(m);
                }
            }
            for (s, files) in claims {
                let winner = match files[..] {
                    [m] => Some(m),
                    _ => {
                        // Several files, one sidecar: its title may say which.
                        let title = sidecars[&s].title.as_deref();
                        let named: Vec<usize> = files
                            .iter()
                            .copied()
                            .filter(|&m| Some(name_of(m)) == title)
                            .collect();
                        match named[..] {
                            [m] => Some(m),
                            _ => None,
                        }
                    }
                };
                match winner {
                    Some(m) => {
                        matched.insert(m, (s, rule));
                        claimed.insert(s);
                    }
                    None => {
                        for m in files {
                            ambiguous.insert(m, s);
                        }
                    }
                }
            }
        }
    }

    // 3. Edited copies and Live Photo videos borrow another file's sidecar.
    let mut edit_of_entry: HashMap<usize, usize> = HashMap::new();
    let mut live_of_entry: HashMap<usize, usize> = HashMap::new();
    for folder in folders.values() {
        let by_name: HashMap<&str, usize> = folder
            .media
            .iter()
            .map(|&m| (split_dir(&entries[m].path).1, m))
            .collect();
        for &m in &folder.media {
            let name = split_dir(&entries[m].path).1;
            if let Some(original) =
                edited_original(name).and_then(|o| by_name.get(o.as_str()).copied())
            {
                if original != m {
                    edit_of_entry.insert(m, original);
                    if !matched.contains_key(&m) && !ambiguous.contains_key(&m) {
                        if let Some(&(s, _)) = matched.get(&original) {
                            matched.insert(m, (s, Rule::EditOf));
                        }
                    }
                    continue;
                }
            }
            if LIVE_VIDEOS.contains(&ext_lower(name).as_str()) {
                let stem = stem_lower(name);
                let still = STILLS.iter().find_map(|ext| {
                    folder.media.iter().copied().find(|&o| {
                        let n = split_dir(&entries[o].path).1;
                        stem_lower(n) == stem && ext_lower(n) == *ext
                    })
                });
                if let Some(still) = still {
                    // Whether it really is a Live Photo's video (a few seconds) is the import's
                    // call, once it has read it.
                    live_of_entry.insert(m, still);
                    if !matched.contains_key(&m) && !ambiguous.contains_key(&m) {
                        if let Some(&(s, _)) = matched.get(&still) {
                            matched.insert(m, (s, Rule::LivePhoto));
                        }
                    }
                }
            }
        }
    }

    // 4. Copies of one file become one item. Without a CRC (an unpacked folder), name and size
    // alone could join two different photos, so the date their own sidecars give must agree too;
    // copies left apart still end up as one photo when imported (same bytes, same id).
    let key = |i: usize| {
        let e = &entries[i];
        let date = match e.crc32 {
            Some(_) => None,
            None => matched
                .get(&i)
                .filter(|(_, r)| r.is_own())
                .map(|(s, _)| sidecars[s].taken_at_ms),
        };
        (split_dir(&e.path).1.to_owned(), e.size, e.crc32, date)
    };
    type Key = (String, u64, Option<u32>, Option<Option<i64>>);
    let mut groups: BTreeMap<Key, Vec<usize>> = BTreeMap::new();
    for folder in folders.values() {
        for &m in &folder.media {
            groups.entry(key(m)).or_default().push(m);
        }
    }
    let album_of = |i: usize| {
        let (dir, _) = split_dir(&entries[i].path);
        folders.get(dir).and_then(|f| f.album)
    };
    let in_folder = |i: usize, names: &[&str]| {
        let (dir, _) = split_dir(&entries[i].path);
        names.contains(&last_component(dir).to_lowercase().as_str())
    };
    let mut groups: Vec<Vec<usize>> = groups.into_values().collect();
    for g in &mut groups {
        // The copy to read: one with its own sidecar, outside albums (its year folder), first.
        g.sort_by_key(|&m| {
            let own = matched.get(&m).is_some_and(|(_, r)| r.is_own());
            (!own, album_of(m).is_some(), m)
        });
    }
    groups.sort_by_key(|g| g.iter().min().copied());
    let mut item_of_entry: HashMap<usize, usize> = HashMap::new();
    for g in groups {
        let first = g[0];
        let sidecar = matched.get(&first).copied();
        let mut meta = sidecar
            .map(|(s, _)| sidecars[&s].clone())
            .unwrap_or_default();
        meta.favorite = g
            .iter()
            .any(|m| matched.get(m).is_some_and(|(s, _)| sidecars[s].favorite));
        meta.archived |= g.iter().any(|&m| in_folder(m, ARCHIVE_FOLDERS));
        let trashed = meta.trashed || g.iter().all(|&m| in_folder(m, TRASH_FOLDERS));
        if trashed {
            out.left.extend(g.iter().map(|&m| (m, Left::Trashed)));
            continue;
        }
        let mut albums: Vec<usize> = g.iter().filter_map(|&m| album_of(m)).collect();
        albums.sort_unstable();
        albums.dedup();
        for &m in &g {
            if !matched.contains_key(&m) {
                match ambiguous.get(&m) {
                    Some(&s) => out.ambiguous.push((m, s)),
                    None if sidecar.is_none() => out.unmatched.push(m),
                    None => {}
                }
            }
            item_of_entry.insert(m, out.items.len());
        }
        out.items.push(PlanItem {
            entries: g,
            sidecar,
            meta,
            albums,
            edit_of: None,
            live_photo_of: None,
        });
    }
    for item in &mut out.items {
        let first = item.entries[0];
        item.edit_of = edit_of_entry
            .get(&first)
            .and_then(|o| item_of_entry.get(o))
            .copied();
        item.live_photo_of = live_of_entry
            .get(&first)
            .and_then(|o| item_of_entry.get(o))
            .copied();
    }
    // An edited copy is in the albums its original is in (owner's decision).
    for i in 0..out.items.len() {
        if let Some(o) = out.items[i].edit_of {
            let more = out.items[o].albums.clone();
            let albums = &mut out.items[i].albums;
            albums.extend(more);
            albums.sort_unstable();
            albums.dedup();
        }
    }
    for folder in folders.values() {
        for &s in folder.sidecars.values() {
            if !claimed.contains(&s) && !ambiguous.values().any(|&a| a == s) {
                out.left.push((s, Left::UnusedSidecar));
            }
        }
    }
    out.unmatched.sort_unstable();
    out.ambiguous.sort_unstable();
    out.left.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cuts_like_google() {
        let names = |n: &str| sidecar_names(n, None);
        // Seen in a May 2024 export (docs/notes/takeout-format.md). A name of 46 units or more
        // gets the same sidecar name in the old and the supplemental form.
        assert_eq!(
            names("PXL_20230814_201154491.LONG_EXPOSURE-01.COVER.jpg"),
            ["PXL_20230814_201154491.LONG_EXPOSURE-01.COVER..json"]
        );
        assert_eq!(
            names("PXL_20230814_201205358.ACTION_PAN-01.COVER.jpg"),
            ["PXL_20230814_201205358.ACTION_PAN-01.COVER.jpg.json"]
        );
        let emoji = "😀😃😄😁😆😅😂🤣🥲☺️😊😇🙂🙃😉😌😍🥰😘😗😙😚😋😛.jpg";
        assert_eq!(
            names(emoji),
            ["😀😃😄😁😆😅😂🤣🥲☺️😊😇🙂🙃😉😌😍🥰😘😗😙😚😋.json"]
        );
        assert_eq!(
            names("IMG_2367.jpg"),
            [
                "IMG_2367.jpg.supplemental-metadata.json",
                "IMG_2367.jpg.json"
            ]
        );
        // A cut supplemental suffix, as other tools report it (`.supplemental-me.json`).
        assert_eq!(
            names("PXL_20231231_235959123_HDR.jpg")[0],
            "PXL_20231231_235959123_HDR.jpg.supplemental-me.json"
        );
        assert_eq!(
            sidecar_names("IMG_2367.jpg", Some(1)),
            [
                "IMG_2367.jpg.supplemental-metadata(1).json",
                "IMG_2367.jpg(1).json",
                "IMG_2367.jpg(1).supplemental-metadata.json",
            ]
        );
    }

    #[test]
    fn numbers_and_edits() {
        assert_eq!(
            moved_number("DSC_0238(1).JPG"),
            Some(("DSC_0238.JPG".into(), 1))
        );
        assert_eq!(
            moved_number("photo (3)(5).jpg"),
            Some(("photo (3).jpg".into(), 5))
        );
        assert_eq!(moved_number("DSC_0238.JPG"), None);
        assert_eq!(moved_number("a().jpg"), None);
        assert_eq!(
            edited_original("IMG_8416-modifié.JPG").as_deref(),
            Some("IMG_8416.JPG")
        );
        assert_eq!(
            edited_original("IMG_1-edited(1).jpg").as_deref(),
            Some("IMG_1(1).jpg")
        );
        assert_eq!(
            edited_original("IMG_1-EDITED.jpg").as_deref(),
            Some("IMG_1.jpg")
        );
        assert_eq!(edited_original("IMG_1.jpg"), None);
    }

    #[test]
    fn dates() {
        let f = 1_696_574_349_250; // with milliseconds
        assert_eq!(resolve_taken_at(Some(f), None), Some(f));
        assert_eq!(resolve_taken_at(None, Some(5)), Some(5));
        // Two hours apart (a zone): the file's.
        assert_eq!(
            resolve_taken_at(Some(f), Some(1_696_574_349_000 + 7_200_000)),
            Some(f)
        );
        // India: +5:30.
        assert_eq!(
            resolve_taken_at(Some(f), Some(1_696_574_349_000 - 19_800_000)),
            Some(f)
        );
        // Re-dated by hand: the export's.
        let e = 1_500_000_000_000;
        assert_eq!(resolve_taken_at(Some(f), Some(e)), Some(e));
    }
}
