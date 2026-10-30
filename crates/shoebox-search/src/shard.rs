//! Index shards (epic S1 §3): immutable blobs holding one kind of result for up to
//! [`MAX_PHOTOS`] photos, made with one model, and the rules that decide which shards count.
//!
//! Layout, little endian, version 1:
//!
//! ```text
//! magic "SBIX" · version u8 · kind u8 · model [32] · dims u16 · count u32
//! replaces: u16 n, then n × [32]            inside the blob, so its hash commits to it
//! photo ids: count × [32], strictly ascending
//! payload: kind-specific, one row per id in the same order
//!   vec8   scale f32 · dims × i8
//!   vec1   dims / 8 bytes (sign bits, bit i of byte i / 8 is dimension i)
//!   text   u8 n · n × (label u16 · score u8) · u32 len · len bytes of UTF-8 (text read from the photo)
//!   faces  u8 n · n × (box 4 × u16 · landmarks 10 × u16 · quality u8 · scale f32 · dims × i8)
//! ```
//!
//! Boxes and landmarks are fractions of the image's width and height in 1/65535.

use shoebox_core::ids::Blake3Hash;
use shoebox_core::model::{IndexShard, ModelId, PhotoId, ShardKind};
use std::collections::{BTreeMap, BTreeSet};

pub const MAGIC: &[u8; 4] = b"SBIX";
pub const VERSION: u8 = 1;
/// Most photos in one shard.
pub const MAX_PHOTOS: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShardError {
    BadMagic,
    UnknownVersion(u8),
    UnknownKind(u8),
    /// Photo ids not strictly ascending (or repeated).
    Unsorted,
    /// The bytes end early, run past the rows, or a row is malformed.
    Length,
    TooMany,
}

impl std::fmt::Display for ShardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadMagic => write!(f, "not an index shard"),
            Self::UnknownVersion(v) => write!(f, "index shard version {v} is not supported"),
            Self::UnknownKind(k) => write!(f, "unknown index shard kind {k}"),
            Self::Unsorted => write!(f, "index shard photo ids are not sorted"),
            Self::Length => write!(f, "index shard length does not match its contents"),
            Self::TooMany => write!(f, "index shard holds more than {MAX_PHOTOS} photos"),
        }
    }
}

impl std::error::Error for ShardError {}

/// An embedding at 8 bits: `value[i] ≈ v[i] as f32 * scale`.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Vec8 {
    pub scale: f32,
    pub v: Vec<i8>,
}

impl Vec8 {
    /// Quantise symmetrically so the largest magnitude maps to ±127.
    pub fn quantize(x: &[f32]) -> Self {
        let max = x.iter().fold(0f32, |m, v| m.max(v.abs()));
        let scale = if max > 0.0 { max / 127.0 } else { 1.0 };
        let v = x
            .iter()
            .map(|v| (v / scale).round().clamp(-127.0, 127.0) as i8)
            .collect();
        Self { scale, v }
    }

    pub fn to_f32(&self) -> Vec<f32> {
        self.v.iter().map(|&q| f32::from(q) * self.scale).collect()
    }
}

/// Sign bits of an embedding (bit `i % 8` of byte `i / 8` is dimension `i`).
pub fn sign_bits(x: &[f32]) -> Vec<u8> {
    let mut out = vec![0u8; x.len().div_ceil(8)];
    for (i, v) in x.iter().enumerate() {
        if *v > 0.0 {
            out[i / 8] |= 1 << (i % 8);
        }
    }
    out
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextRow {
    /// (index in the label list, score 0–255), best first.
    pub labels: Vec<(u16, u8)>,
    /// Text read from the photo.
    pub ocr: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    /// left, top, right, bottom in 1/65535 of the image.
    pub bbox: [u16; 4],
    /// Eyes, nose, mouth corners: (x, y) pairs in 1/65535 of the image.
    pub landmarks: [u16; 10],
    /// Detector confidence and sharpness, 0–255.
    pub quality: u8,
    pub embedding: Vec8,
}

/// The rows of a shard, one per photo.
#[derive(Clone, Debug, PartialEq)]
pub enum Rows {
    Vec8(Vec<Vec8>),
    Vec1(Vec<Vec<u8>>),
    Text(Vec<TextRow>),
    Faces(Vec<Vec<Face>>),
}

impl Rows {
    pub fn kind(&self) -> ShardKind {
        match self {
            Rows::Vec8(_) => ShardKind::Vec8,
            Rows::Vec1(_) => ShardKind::Vec1,
            Rows::Text(_) => ShardKind::Text,
            Rows::Faces(_) => ShardKind::Faces,
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Rows::Vec8(r) => r.len(),
            Rows::Vec1(r) => r.len(),
            Rows::Text(r) => r.len(),
            Rows::Faces(r) => r.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A decoded shard.
#[derive(Clone, Debug, PartialEq)]
pub struct Shard {
    pub model: ModelId,
    /// Embedding dimensions (picture embeddings for vec8/vec1, face embeddings for faces; 0 for
    /// text).
    pub dims: u16,
    pub replaces: Vec<Blake3Hash>,
    /// Strictly ascending.
    pub ids: Vec<PhotoId>,
    pub rows: Rows,
}

impl Shard {
    /// A shard from unsorted `(photo, row)` pairs; the later of two rows for one photo is kept.
    pub fn build(
        model: ModelId,
        dims: u16,
        replaces: Vec<Blake3Hash>,
        rows: Rows,
        ids: Vec<PhotoId>,
    ) -> Result<Shard, ShardError> {
        if ids.len() != rows.len() {
            return Err(ShardError::Length);
        }
        let mut order: Vec<usize> = (0..ids.len()).collect();
        order.sort_by_key(|&i| (ids[i], std::cmp::Reverse(i)));
        order.dedup_by_key(|i| ids[*i]);
        fn pick<T: Clone>(v: &[T], order: &[usize]) -> Vec<T> {
            order.iter().map(|&i| v[i].clone()).collect()
        }
        let rows = match &rows {
            Rows::Vec8(r) => Rows::Vec8(pick(r, &order)),
            Rows::Vec1(r) => Rows::Vec1(pick(r, &order)),
            Rows::Text(r) => Rows::Text(pick(r, &order)),
            Rows::Faces(r) => Rows::Faces(pick(r, &order)),
        };
        let mut replaces = replaces;
        replaces.sort();
        replaces.dedup();
        let s = Shard {
            model,
            dims,
            replaces,
            ids: pick(&ids, &order),
            rows,
        };
        if s.ids.len() > MAX_PHOTOS {
            return Err(ShardError::TooMany);
        }
        Ok(s)
    }

    pub fn kind(&self) -> ShardKind {
        self.rows.kind()
    }

    /// Row index of `photo`.
    pub fn position(&self, photo: &PhotoId) -> Option<usize> {
        self.ids.binary_search(photo).ok()
    }

    pub fn encode(&self) -> Vec<u8> {
        let d = usize::from(self.dims);
        let mut out = Vec::with_capacity(48 + self.ids.len() * (32 + d + 4));
        out.extend_from_slice(MAGIC);
        out.push(VERSION);
        out.push(self.kind().byte());
        out.extend_from_slice(&self.model.0);
        out.extend_from_slice(&self.dims.to_le_bytes());
        out.extend_from_slice(&(self.ids.len() as u32).to_le_bytes());
        out.extend_from_slice(&(self.replaces.len() as u16).to_le_bytes());
        for r in &self.replaces {
            out.extend_from_slice(&r.0);
        }
        for id in &self.ids {
            out.extend_from_slice(&id.0);
        }
        let put_vec8 = |out: &mut Vec<u8>, v: &Vec8| {
            out.extend_from_slice(&v.scale.to_le_bytes());
            let mut q = v.v.clone();
            q.resize(d, 0);
            out.extend(q.iter().map(|b| *b as u8));
        };
        match &self.rows {
            Rows::Vec8(rows) => rows.iter().for_each(|r| put_vec8(&mut out, r)),
            Rows::Vec1(rows) => {
                for r in rows {
                    let mut b = r.clone();
                    b.resize(d.div_ceil(8), 0);
                    out.extend_from_slice(&b);
                }
            }
            Rows::Text(rows) => {
                for r in rows {
                    let n = r.labels.len().min(255);
                    out.push(n as u8);
                    for (label, score) in &r.labels[..n] {
                        out.extend_from_slice(&label.to_le_bytes());
                        out.push(*score);
                    }
                    out.extend_from_slice(&(r.ocr.len() as u32).to_le_bytes());
                    out.extend_from_slice(r.ocr.as_bytes());
                }
            }
            Rows::Faces(rows) => {
                for faces in rows {
                    let n = faces.len().min(255);
                    out.push(n as u8);
                    for f in &faces[..n] {
                        for x in f.bbox.iter().chain(&f.landmarks) {
                            out.extend_from_slice(&x.to_le_bytes());
                        }
                        out.push(f.quality);
                        put_vec8(&mut out, &f.embedding);
                    }
                }
            }
        }
        out
    }

    /// Decode and check a shard: magic, version, kind, sorted ids and an exact length.
    pub fn decode(bytes: &[u8]) -> Result<Shard, ShardError> {
        let mut r = Reader(bytes);
        if r.take(4)? != MAGIC {
            return Err(ShardError::BadMagic);
        }
        let version = r.u8()?;
        if version != VERSION {
            return Err(ShardError::UnknownVersion(version));
        }
        let kind_byte = r.u8()?;
        let kind = ShardKind::from_byte(kind_byte).ok_or(ShardError::UnknownKind(kind_byte))?;
        let model = ModelId(r.array()?);
        let dims = r.u16()?;
        let count = r.u32()? as usize;
        if count > MAX_PHOTOS {
            return Err(ShardError::TooMany);
        }
        let n = usize::from(r.u16()?);
        let replaces = (0..n)
            .map(|_| r.array().map(Blake3Hash))
            .collect::<Result<Vec<_>, _>>()?;
        let ids = (0..count)
            .map(|_| r.array().map(Blake3Hash))
            .collect::<Result<Vec<_>, _>>()?;
        if ids.windows(2).any(|w| w[0] >= w[1]) {
            return Err(ShardError::Unsorted);
        }
        let d = usize::from(dims);
        let vec8 = |r: &mut Reader| -> Result<Vec8, ShardError> {
            let scale = f32::from_le_bytes(r.array()?);
            let v = r.take(d)?.iter().map(|b| *b as i8).collect();
            Ok(Vec8 { scale, v })
        };
        let rows = match kind {
            ShardKind::Vec8 => {
                Rows::Vec8((0..count).map(|_| vec8(&mut r)).collect::<Result<_, _>>()?)
            }
            ShardKind::Vec1 => Rows::Vec1(
                (0..count)
                    .map(|_| r.take(d.div_ceil(8)).map(<[u8]>::to_vec))
                    .collect::<Result<_, _>>()?,
            ),
            ShardKind::Text => Rows::Text(
                (0..count)
                    .map(|_| {
                        let n = r.u8()?;
                        let labels =
                            (0..n)
                                .map(|_| Ok((r.u16()?, r.u8()?)))
                                .collect::<Result<_, ShardError>>()?;
                        let len = r.u32()? as usize;
                        let ocr = std::str::from_utf8(r.take(len)?)
                            .map_err(|_| ShardError::Length)?
                            .to_string();
                        Ok(TextRow { labels, ocr })
                    })
                    .collect::<Result<_, ShardError>>()?,
            ),
            ShardKind::Faces => Rows::Faces(
                (0..count)
                    .map(|_| {
                        let n = r.u8()?;
                        (0..n)
                            .map(|_| {
                                let mut xs = [0u16; 14];
                                for x in &mut xs {
                                    *x = r.u16()?;
                                }
                                let quality = r.u8()?;
                                let embedding = vec8(&mut r)?;
                                Ok(Face {
                                    bbox: xs[..4].try_into().expect("4"),
                                    landmarks: xs[4..].try_into().expect("10"),
                                    quality,
                                    embedding,
                                })
                            })
                            .collect::<Result<Vec<_>, ShardError>>()
                    })
                    .collect::<Result<_, ShardError>>()?,
            ),
        };
        if !r.0.is_empty() {
            return Err(ShardError::Length);
        }
        Ok(Shard {
            model,
            dims,
            replaces,
            ids,
            rows,
        })
    }

    /// Whether this blob is what `record` says it is (model, kind, count and `replaces`).
    pub fn matches(&self, record: &IndexShard) -> bool {
        let mut claimed = record.replaces.clone();
        claimed.sort();
        claimed.dedup();
        self.model == record.model
            && self.kind() == record.kind
            && self.ids.len() == record.count as usize
            && self.replaces == claimed
    }
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], ShardError> {
        if self.0.len() < n {
            return Err(ShardError::Length);
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], ShardError> {
        Ok(self.take(N)?.try_into().expect("N bytes"))
    }
    fn u8(&mut self) -> Result<u8, ShardError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, ShardError> {
        Ok(u16::from_le_bytes(self.array()?))
    }
    fn u32(&mut self) -> Result<u32, ShardError> {
        Ok(u32::from_le_bytes(self.array()?))
    }
}

// ---------- which shards count (epic S1 §3.2) ----------

/// What this device knows of a shard's blob.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fetched {
    /// Not fetched (yet).
    Unknown,
    /// Fetched, matches its record; its photo ids, ascending.
    Valid(Vec<PhotoId>),
    /// Fetched and does not decode or does not match its record: ignored entirely.
    Invalid,
    /// Not fetched, assumed valid (for planning what to fetch): replaces what it lists without
    /// a coverage check, and its own photos are unknown.
    Assumed,
}

/// The shards that count, from every record and what is known of their blobs.
///
/// 1. A shard is **live** unless a live shard of the same model and kind replaces it.
/// 2. Only a fetched, valid shard replaces anything, and only if it covers no photo the replaced
///    shards (those whose ids are known) did not hold. Until then both are searched.
/// 3. An invalid shard is never live.
///
/// Replacements form a graph whose edges point back in time (a blob can only name blobs that
/// existed before it), so there are no cycles among honest shards; any that remain from forged
/// records are left live, which is harmless (one result per photo, see [`winners`]). The result
/// depends only on the set of records, not their order or repetition.
pub fn live_shards<'a>(
    records: impl IntoIterator<Item = &'a IndexShard>,
    fetched: impl Fn(&Blake3Hash) -> Fetched,
) -> BTreeSet<Blake3Hash> {
    // One record per blob, as `LibraryState` keeps it (the later HLC, then the greater author).
    let mut by_blob: BTreeMap<Blake3Hash, &IndexShard> = BTreeMap::new();
    for r in records {
        let keep = by_blob
            .get(&r.blob.hash)
            .is_none_or(|e| (e.hlc, e.made_by) < (r.hlc, r.made_by));
        if keep {
            by_blob.insert(r.blob.hash, r);
        }
    }
    let state: BTreeMap<Blake3Hash, Fetched> = by_blob.keys().map(|h| (*h, fetched(h))).collect();
    // Edges: replacer → replaced, only for valid replacers covering their photos.
    let mut replaced_by: BTreeMap<Blake3Hash, Vec<Blake3Hash>> = BTreeMap::new();
    for (h, r) in &by_blob {
        let ids = match state.get(h) {
            Some(Fetched::Valid(ids)) => Some(ids),
            Some(Fetched::Assumed) => None,
            _ => continue,
        };
        let targets: Vec<Blake3Hash> = r
            .replaces
            .iter()
            .filter(|t| *t != h)
            .filter(|t| {
                by_blob
                    .get(t)
                    .is_some_and(|o| o.model == r.model && o.kind == r.kind)
            })
            .copied()
            .collect();
        let mut covered: BTreeSet<&PhotoId> = BTreeSet::new();
        let mut all_known = true;
        for t in &targets {
            match state.get(t) {
                Some(Fetched::Valid(ids)) => covered.extend(ids),
                _ => all_known = false,
            }
        }
        if let Some(ids) = ids {
            if all_known && !targets.is_empty() && !ids.iter().all(|i| covered.contains(i)) {
                continue;
            }
        }
        for t in targets {
            replaced_by.entry(t).or_default().push(*h);
        }
    }
    #[derive(Clone, Copy, PartialEq)]
    enum S {
        Open,
        Live,
        Dead,
    }
    let mut s: BTreeMap<Blake3Hash, S> = by_blob
        .keys()
        .map(|h| {
            let v = if state[h] == Fetched::Invalid {
                S::Dead
            } else {
                S::Open
            };
            (*h, v)
        })
        .collect();
    loop {
        let mut changed = false;
        for h in by_blob.keys() {
            if s[h] != S::Open {
                continue;
            }
            let by = replaced_by.get(h).map(Vec::as_slice).unwrap_or(&[]);
            let next = if by.iter().any(|r| s[r] == S::Live) {
                S::Dead
            } else if by.iter().all(|r| s[r] == S::Dead) {
                S::Live
            } else {
                continue;
            };
            s.insert(*h, next);
            changed = true;
        }
        if !changed {
            break;
        }
    }
    s.into_iter()
        .filter(|(_, v)| *v != S::Dead)
        .map(|(h, _)| h)
        .collect()
}

/// One result per photo: among live shards holding it, the one with the lowest blob hash.
pub fn winners<'a>(
    live: impl IntoIterator<Item = (Blake3Hash, &'a [PhotoId])>,
) -> BTreeMap<PhotoId, Blake3Hash> {
    let mut out: BTreeMap<PhotoId, Blake3Hash> = BTreeMap::new();
    for (h, ids) in live {
        for id in ids {
            out.entry(*id).and_modify(|w| *w = (*w).min(h)).or_insert(h);
        }
    }
    out
}
