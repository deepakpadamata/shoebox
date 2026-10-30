//! The facts index (S1-03): what every device can answer from the records it already holds,
//! with no model and nothing synced. Dates, cameras, lenses, kinds of media, favourites and
//! albums; place names (S1-04), labels and text in photos (S1-13, S1-14) and people (S1-19) are
//! more words in the same index.
//!
//! Rows are photos in the order they were added. A changed photo marks its old row dead and gets
//! a new one, and the rows are compacted when a tenth of them are dead, so postings never need
//! editing in place.

use crate::calendar::Zone;
use crate::query::{DateFilter, FilterKind, MediaKind, NameKind, Query};
use crate::text::{fold, words};
use serde::{Deserialize, Serialize};
use shoebox_core::ids::Timestamp;
use shoebox_core::model::{AlbumId, PhotoId, PhotoRecord};
use std::collections::{HashMap, HashSet};

/// What the index keeps of a photo: derived from its record and nothing else, so every device
/// derives the same facts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhotoFacts {
    pub id: PhotoId,
    pub taken_at: Timestamp,
    pub camera: Option<String>,
    pub lens: Option<String>,
    pub mime: String,
    pub width: u32,
    pub height: u32,
    pub favourite: bool,
    pub deleted: bool,
    /// Latitude below the equator (for seasons); `None` without GPS.
    pub southern: Option<bool>,
    pub gps: Option<shoebox_core::model::Gps>,
}

impl PhotoFacts {
    pub fn of(rec: &PhotoRecord) -> Self {
        let e = &rec.exif;
        Self {
            id: rec.id,
            taken_at: rec.taken_at,
            camera: camera_name(e.camera_make.as_deref(), e.camera_model.as_deref()),
            lens: e
                .lens
                .as_ref()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty()),
            mime: rec.original.mime.clone(),
            width: e.width,
            height: e.height,
            favourite: rec.favorite.value,
            deleted: rec.deleted_at.value.is_some(),
            southern: e.gps.map(|g| g.lat_e7 < 0),
            gps: e.gps,
        }
    }

    pub fn is_video(&self) -> bool {
        self.mime.starts_with("video/")
    }

    pub fn is_raw(&self) -> bool {
        RAW_MIMES.contains(&self.mime.as_str())
    }

    /// No camera, and a PNG or exactly a common screen size.
    pub fn is_screenshot(&self) -> bool {
        if self.camera.is_some() || self.is_video() {
            return false;
        }
        self.mime == "image/png"
            || SCREEN_SIZES.iter().any(|&(w, h)| {
                (self.width, self.height) == (w, h) || (self.width, self.height) == (h, w)
            })
    }

    /// The lens says it is the front camera (iPhones write "… front camera …").
    pub fn is_selfie(&self) -> bool {
        self.lens
            .as_deref()
            .is_some_and(|l| fold(l).contains("front"))
    }

    fn has(&self, k: MediaKind) -> bool {
        match k {
            MediaKind::Photo => !self.is_video(),
            MediaKind::Video => self.is_video(),
            MediaKind::Screenshot => self.is_screenshot(),
            MediaKind::Selfie => self.is_selfie(),
            MediaKind::Raw => self.is_raw(),
            // Live photos are not recorded yet.
            MediaKind::Live => false,
        }
    }
}

const RAW_MIMES: &[&str] = &[
    "image/x-adobe-dng",
    "image/x-canon-cr2",
    "image/x-canon-cr3",
    "image/x-nikon-nef",
    "image/x-sony-arw",
    "image/x-fuji-raf",
    "image/x-olympus-orf",
    "image/x-panasonic-rw2",
];

/// Screens whose screenshots are common (phones, tablets, laptops), portrait or landscape.
const SCREEN_SIZES: &[(u32, u32)] = &[
    (750, 1334),
    (828, 1792),
    (1080, 1920),
    (1080, 2220),
    (1080, 2280),
    (1080, 2340),
    (1080, 2400),
    (1080, 2412),
    (1125, 2436),
    (1170, 2532),
    (1179, 2556),
    (1206, 2622),
    (1242, 2208),
    (1242, 2688),
    (1284, 2778),
    (1290, 2796),
    (1320, 2868),
    (1440, 2560),
    (1440, 3040),
    (1440, 3088),
    (1440, 3120),
    (1440, 3200),
    (1536, 2048),
    (1620, 2160),
    (1640, 2360),
    (1668, 2388),
    (2048, 2732),
    (768, 1366),
    (864, 1536),
    (900, 1440),
    (1080, 1920),
    (1200, 1920),
    (1440, 2560),
    (1600, 2560),
    (1800, 2880),
    (1964, 3024),
    (2234, 3456),
    (2160, 3840),
];

/// "Canon" + "Canon EOS R5" → "Canon EOS R5"; "Apple" + "iPhone 15 Pro" → "Apple iPhone 15 Pro".
pub fn camera_name(make: Option<&str>, model: Option<&str>) -> Option<String> {
    let make = make.map(str::trim).filter(|s| !s.is_empty());
    let model = model.map(str::trim).filter(|s| !s.is_empty());
    match (make, model) {
        (Some(a), Some(b)) if fold(b).starts_with(&fold(a)) => Some(b.to_string()),
        (Some(a), Some(b)) => Some(format!("{a} {b}")),
        (None, Some(b)) => Some(b.to_string()),
        (Some(a), None) => Some(a.to_string()),
        (None, None) => None,
    }
}

/// Which text a word came from. Each field is scored on its own length.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Field {
    Album,
    Place,
    Camera,
    Label,
    Ocr,
    Person,
}

impl Field {
    /// How much a match in this field counts against the others.
    fn boost(self) -> f32 {
        match self {
            Field::Person => 2.0,
            Field::Album | Field::Place => 1.5,
            Field::Label => 1.2,
            Field::Ocr => 1.0,
            Field::Camera => 0.5,
        }
    }
}

/// Text attached to a photo from elsewhere (places, labels, OCR, people), by field.
pub type Extra = Vec<(Field, String)>;

struct Row {
    facts: PhotoFacts,
    alive: bool,
}

#[derive(Default)]
struct Album {
    name: String,
    folded: String,
    members: HashSet<PhotoId>,
}

/// Where a page of results continues.
pub type Offset = usize;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hits {
    /// Best first (by score when the query has free text, else newest first).
    pub ids: Vec<PhotoId>,
    pub total: usize,
    /// Ordered by relevance rather than by date.
    pub ranked: bool,
    pub next: Option<Offset>,
}

/// The facts index. Plain data; the node keeps it current and persists it.
#[derive(Default)]
pub struct FactsIndex {
    rows: Vec<Row>,
    by_id: HashMap<PhotoId, u32>,
    dead: usize,
    /// (field, word) → rows and how often the word occurs there.
    postings: HashMap<(Field, String), Vec<(u32, u16)>>,
    /// Per row and field, the number of words (for BM25's length normalisation).
    lengths: Vec<[u16; 6]>,
    /// (kind, folded name) → rows, for exact name filters.
    names: HashMap<(NameKind, String), Vec<u32>>,
    albums: HashMap<AlbumId, Album>,
    /// Text from other sources, kept so a row can be rebuilt when its record changes.
    extra: HashMap<PhotoId, Extra>,
}

const FIELDS: [Field; 6] = [
    Field::Album,
    Field::Place,
    Field::Camera,
    Field::Label,
    Field::Ocr,
    Field::Person,
];

fn field_ix(f: Field) -> usize {
    FIELDS.iter().position(|x| *x == f).unwrap()
}

impl FactsIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Photos indexed (alive rows), deleted ones included.
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    pub fn facts(&self, id: &PhotoId) -> Option<&PhotoFacts> {
        self.by_id.get(id).map(|&r| &self.rows[r as usize].facts)
    }

    pub fn ids(&self) -> impl Iterator<Item = &PhotoId> {
        self.by_id.keys()
    }

    pub fn extra(&self, id: &PhotoId) -> Option<&Extra> {
        self.extra.get(id)
    }

    /// Add or update a photo. Unchanged facts are a no-op. Returns whether anything changed.
    pub fn upsert(&mut self, facts: PhotoFacts) -> bool {
        if self.facts(&facts.id) == Some(&facts) {
            return false;
        }
        self.kill(&facts.id);
        self.push(facts);
        self.maybe_compact();
        true
    }

    /// Forget a photo (purged).
    pub fn remove(&mut self, id: &PhotoId) -> bool {
        self.extra.remove(id);
        let had = self.kill(id);
        self.maybe_compact();
        had
    }

    /// Replace one field's text for a photo (places, labels, OCR, people). The photo need not be
    /// indexed yet; the text is used once it is.
    pub fn set_extra(&mut self, id: &PhotoId, field: Field, text: Option<String>) {
        let e = self.extra.entry(*id).or_default();
        let before = e.clone();
        e.retain(|(f, _)| *f != field);
        if let Some(t) = text.filter(|t| !t.trim().is_empty()) {
            e.push((field, t));
            e.sort();
        }
        if *e == before {
            return;
        }
        if e.is_empty() {
            self.extra.remove(id);
        }
        if let Some(facts) = self.facts(id).cloned() {
            self.kill(id);
            self.push(facts);
            self.maybe_compact();
        }
    }

    /// Set an album's name and members (replacing what was known).
    pub fn set_album(
        &mut self,
        id: AlbumId,
        name: &str,
        members: impl IntoIterator<Item = PhotoId>,
    ) {
        self.albums.insert(
            id,
            Album {
                name: name.to_string(),
                folded: fold(name),
                members: members.into_iter().collect(),
            },
        );
    }

    pub fn remove_album(&mut self, id: &AlbumId) {
        self.albums.remove(id);
    }

    pub fn album_ids(&self) -> impl Iterator<Item = &AlbumId> {
        self.albums.keys()
    }

    fn kill(&mut self, id: &PhotoId) -> bool {
        match self.by_id.remove(id) {
            Some(r) => {
                self.rows[r as usize].alive = false;
                self.dead += 1;
                true
            }
            None => false,
        }
    }

    fn push(&mut self, facts: PhotoFacts) {
        let row = self.rows.len() as u32;
        let mut lengths = [0u16; 6];
        let mut texts: Vec<(Field, String)> = Vec::new();
        if let Some(c) = &facts.camera {
            self.names
                .entry((NameKind::Camera, fold(c)))
                .or_default()
                .push(row);
            texts.push((Field::Camera, c.clone()));
        }
        if let Some(l) = &facts.lens {
            self.names
                .entry((NameKind::Lens, fold(l)))
                .or_default()
                .push(row);
            texts.push((Field::Camera, l.clone()));
        }
        if let Some(extra) = self.extra.get(&facts.id) {
            for (f, t) in extra {
                texts.push((*f, t.clone()));
                // Places and people are also exact names ("Lisbon", "Ana"); one name per line.
                let kind = match f {
                    Field::Place => Some(NameKind::Place),
                    Field::Person => Some(NameKind::Person),
                    _ => None,
                };
                if let Some(kind) = kind {
                    for name in t.lines().map(str::trim).filter(|n| !n.is_empty()) {
                        self.names.entry((kind, fold(name))).or_default().push(row);
                    }
                }
            }
        }
        let mut counts: HashMap<(Field, String), u16> = HashMap::new();
        for (f, t) in texts {
            for w in words(&t) {
                lengths[field_ix(f)] = lengths[field_ix(f)].saturating_add(1);
                *counts.entry((f, w)).or_default() += 1;
            }
        }
        for (k, n) in counts {
            self.postings.entry(k).or_default().push((row, n));
        }
        self.by_id.insert(facts.id, row);
        self.rows.push(Row { facts, alive: true });
        self.lengths.push(lengths);
    }

    fn maybe_compact(&mut self) {
        if self.dead > 64 && self.dead * 10 > self.rows.len() {
            self.compact();
        }
    }

    /// Rebuild without dead rows.
    pub fn compact(&mut self) {
        let rows = std::mem::take(&mut self.rows);
        self.by_id.clear();
        self.postings.clear();
        self.names.clear();
        self.lengths.clear();
        self.dead = 0;
        for r in rows.into_iter().filter(|r| r.alive) {
            self.push(r.facts);
        }
    }

    /// The names this library contains, for the parser and for suggestions.
    pub fn vocabulary(&self) -> crate::query::Vocabulary {
        let mut v = crate::query::Vocabulary::new();
        let mut seen: HashSet<(NameKind, &str)> = HashSet::new();
        for row in self.rows.iter().filter(|r| r.alive && !r.facts.deleted) {
            if let Some(c) = &row.facts.camera {
                if seen.insert((NameKind::Camera, c)) {
                    v.add(NameKind::Camera, c);
                }
            }
            if let Some(l) = &row.facts.lens {
                if seen.insert((NameKind::Lens, l)) {
                    v.add(NameKind::Lens, l);
                }
            }
            if let Some(extra) = self.extra.get(&row.facts.id) {
                for (f, t) in extra {
                    let kind = match f {
                        Field::Place => NameKind::Place,
                        Field::Person => NameKind::Person,
                        Field::Label => NameKind::Label,
                        _ => continue,
                    };
                    for name in t.lines().map(str::trim).filter(|n| !n.is_empty()) {
                        if seen.insert((kind, name)) {
                            v.add(kind, name);
                        }
                    }
                }
            }
        }
        for a in self.albums.values() {
            v.add(NameKind::Album, &a.name);
        }
        v
    }

    /// Rows (alive, not deleted) passing every filter in `q`.
    pub fn filter(&self, q: &Query, zone: Zone, southern_default: bool) -> Vec<u32> {
        let mut rows: Vec<u32> = self
            .by_id
            .values()
            .copied()
            .filter(|&r| !self.rows[r as usize].facts.deleted)
            .collect();
        rows.sort_unstable();
        for f in &q.filters {
            match &f.what {
                FilterKind::Name {
                    kind: NameKind::Album,
                    name,
                } => {
                    let folded = fold(name);
                    let members: HashSet<&PhotoId> = self
                        .albums
                        .values()
                        .filter(|a| a.folded == folded)
                        .flat_map(|a| a.members.iter())
                        .collect();
                    rows.retain(|&r| members.contains(&self.rows[r as usize].facts.id));
                }
                FilterKind::Name { kind, name } => {
                    let set: HashSet<u32> = self
                        .names
                        .get(&(*kind, fold(name)))
                        .map(|v| v.iter().copied().collect())
                        .unwrap_or_default();
                    rows.retain(|r| set.contains(r));
                }
                FilterKind::Date(d) => rows.retain(|&r| {
                    let facts = &self.rows[r as usize].facts;
                    date_matches(d, facts, zone, southern_default)
                }),
                FilterKind::Media(k) => rows.retain(|&r| self.rows[r as usize].facts.has(*k)),
                FilterKind::Favourite => rows.retain(|&r| self.rows[r as usize].facts.favourite),
            }
        }
        rows
    }

    /// BM25 scores of `rows` for `text` over every text field, best first. Rows that match no
    /// word are left out. Album names count for their members.
    pub fn keyword(&self, rows: &[u32], text: &str) -> Vec<(PhotoId, f32)> {
        let terms = words(text);
        if terms.is_empty() {
            return vec![];
        }
        let allowed: HashSet<u32> = rows.iter().copied().collect();
        let n = self.by_id.len().max(1) as f32;
        let alive = |r: &u32| self.rows[*r as usize].alive;
        let mut avg = [0f32; 6];
        for (i, total) in avg.iter_mut().enumerate() {
            let sum: f32 = self
                .by_id
                .values()
                .map(|&r| f32::from(self.lengths[r as usize][i]))
                .sum();
            *total = (sum / n).max(1.0);
        }
        let (k1, b) = (1.2f32, 0.75f32);
        let mut scores: HashMap<u32, f32> = HashMap::new();
        for term in &terms {
            for f in FIELDS {
                let Some(list) = self.postings.get(&(f, term.clone())) else {
                    continue;
                };
                let df = list.iter().filter(|(r, _)| alive(r)).count() as f32;
                let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
                let fi = field_ix(f);
                for &(r, tf) in list.iter().filter(|(r, _)| allowed.contains(r)) {
                    let tf = f32::from(tf);
                    let len = f32::from(self.lengths[r as usize][fi]);
                    let s = idf * tf * (k1 + 1.0) / (tf + k1 * (1.0 - b + b * len / avg[fi]));
                    *scores.entry(r).or_default() += s * f.boost();
                }
            }
            // Album names: every member matches, scored like a short field.
            let albums: Vec<&Album> = self
                .albums
                .values()
                .filter(|a| words(&a.name).contains(term))
                .collect();
            if !albums.is_empty() {
                let members: HashSet<&PhotoId> =
                    albums.iter().flat_map(|a| a.members.iter()).collect();
                let df = members.len() as f32;
                let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
                for &r in rows {
                    if members.contains(&self.rows[r as usize].facts.id) {
                        *scores.entry(r).or_default() += idf * Field::Album.boost();
                    }
                }
            }
        }
        let mut out: Vec<(PhotoId, f32, Timestamp)> = scores
            .into_iter()
            .map(|(r, s)| {
                let f = &self.rows[r as usize].facts;
                (f.id, s, f.taken_at)
            })
            .collect();
        out.sort_by(|a, b| b.1.total_cmp(&a.1).then(b.2.cmp(&a.2)).then(a.0.cmp(&b.0)));
        out.into_iter().map(|(id, s, _)| (id, s)).collect()
    }

    /// Rows newest first (ties by id), as the timeline orders them.
    /// The photos of `rows`.
    pub fn ids_of(&self, rows: &[u32]) -> Vec<PhotoId> {
        rows.iter()
            .map(|&r| self.rows[r as usize].facts.id)
            .collect()
    }

    pub fn by_date(&self, rows: &[u32]) -> Vec<PhotoId> {
        let mut v: Vec<&PhotoFacts> = rows.iter().map(|&r| &self.rows[r as usize].facts).collect();
        v.sort_by(|a, b| b.taken_at.cmp(&a.taken_at).then(a.id.cmp(&b.id)));
        v.into_iter().map(|f| f.id).collect()
    }

    /// Facts-only search: filters, then keyword ranking of the free text (or date order when
    /// there is none). Search by meaning is fused on top of this (S1-12).
    pub fn search(
        &self,
        q: &Query,
        zone: Zone,
        southern: bool,
        offset: Offset,
        limit: usize,
    ) -> Hits {
        let rows = self.filter(q, zone, southern);
        let (ids, ranked) = if q.free_text.is_empty() {
            (self.by_date(&rows), false)
        } else {
            (
                self.keyword(&rows, &q.free_text)
                    .into_iter()
                    .map(|(id, _)| id)
                    .collect(),
                true,
            )
        };
        page(ids, ranked, offset, limit)
    }

    /// For persistence: every photo's facts, the extra text and the albums.
    pub fn snapshot(&self) -> Snapshot {
        let mut photos: Vec<PhotoFacts> = self
            .by_id
            .values()
            .map(|&r| self.rows[r as usize].facts.clone())
            .collect();
        photos.sort_by_key(|p| p.id);
        let mut extra: Vec<(PhotoId, Extra)> =
            self.extra.iter().map(|(k, v)| (*k, v.clone())).collect();
        extra.sort_by_key(|e| e.0);
        let mut albums: Vec<(AlbumId, String, Vec<PhotoId>)> = self
            .albums
            .iter()
            .map(|(id, a)| {
                let mut m: Vec<PhotoId> = a.members.iter().copied().collect();
                m.sort();
                (*id, a.name.clone(), m)
            })
            .collect();
        albums.sort_by_key(|a| a.0);
        Snapshot {
            photos,
            extra,
            albums,
        }
    }

    pub fn from_snapshot(s: Snapshot) -> Self {
        let mut ix = Self::new();
        ix.extra = s.extra.into_iter().collect();
        for f in s.photos {
            ix.push(f);
        }
        for (id, name, members) in s.albums {
            ix.set_album(id, &name, members);
        }
        ix
    }
}

/// A page of `ids` from `offset`.
pub fn page(ids: Vec<PhotoId>, ranked: bool, offset: Offset, limit: usize) -> Hits {
    let total = ids.len();
    let end = offset.saturating_add(limit).min(total);
    let slice = if offset < total {
        ids[offset..end].to_vec()
    } else {
        vec![]
    };
    Hits {
        ids: slice,
        total,
        ranked,
        next: (end < total).then_some(end),
    }
}

fn date_matches(d: &DateFilter, f: &PhotoFacts, zone: Zone, southern_default: bool) -> bool {
    d.matches(f.taken_at, zone, f.southern.unwrap_or(southern_default))
}

/// What [`FactsIndex::snapshot`] returns; the node encodes it to `facts.idx`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub photos: Vec<PhotoFacts>,
    pub extra: Vec<(PhotoId, Extra)>,
    pub albums: Vec<(AlbumId, String, Vec<PhotoId>)>,
}

const MAGIC: &[u8; 4] = b"SBFX";
/// Bumped whenever the encoding or what is derived changes; another version is rebuilt.
pub const FACTS_VERSION: u8 = 1;

/// `facts.idx`: magic, version, BLAKE3 of the payload, then the postcard-encoded snapshot.
pub fn encode(s: &Snapshot) -> Vec<u8> {
    let payload = postcard::to_stdvec(s).expect("snapshot encodes");
    let mut out = Vec::with_capacity(payload.len() + 37);
    out.extend_from_slice(MAGIC);
    out.push(FACTS_VERSION);
    out.extend_from_slice(blake3::hash(&payload).as_bytes());
    out.extend_from_slice(&payload);
    out
}

/// `None` when the file is of another version, truncated or corrupt (then rebuild).
pub fn decode(bytes: &[u8]) -> Option<Snapshot> {
    if bytes.len() < 37 || &bytes[..4] != MAGIC || bytes[4] != FACTS_VERSION {
        return None;
    }
    let payload = &bytes[37..];
    if blake3::hash(payload).as_bytes() != &bytes[5..37] {
        return None;
    }
    postcard::from_bytes(payload).ok()
}
