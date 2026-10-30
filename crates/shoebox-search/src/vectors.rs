//! Picture embeddings on this device (S1-12): one row per photo, built from the live `vec8`
//! and `vec1` shards of one model, and scored against a query vector.
//!
//! Memory stays small: ids, per-row scales and the binary codes are in memory (about 10 MB per
//! 100k photos at 512 dimensions); the 8-bit vectors (about 52 MB per 100k) stay in a file that
//! is read in a stream for each query, so the operating system's page cache holds them, not this
//! process. (Memory-mapping would need `unsafe`, which the workspace forbids outside two
//! modules.) Above [`SHORTLIST_ABOVE`] candidates the binary codes pick the best
//! [`SHORTLIST`] first and only those rows are read.

use crate::shard::Vec8;
use shoebox_core::model::{ModelId, PhotoId};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub const SHORTLIST_ABOVE: usize = 200_000;
pub const SHORTLIST: usize = 2_000;
const MAGIC: &[u8; 4] = b"SBVX";
const VERSION: u8 = 1;

/// One row in, as taken from the shards.
pub struct RowIn<'a> {
    pub id: PhotoId,
    pub vec8: Option<&'a Vec8>,
    pub vec1: Option<&'a [u8]>,
}

/// The vectors of one model on this device.
pub struct VectorIndex {
    pub model: ModelId,
    pub dims: usize,
    ids: Vec<PhotoId>,
    by_id: HashMap<PhotoId, u32>,
    scales: Vec<f32>,
    /// `dims / 8` bytes per row; rows without a binary code are all zero.
    bits: Vec<u8>,
    /// Rows with an 8-bit vector (the rest only have a binary code: a Viewer of a huge library).
    has8: Vec<bool>,
    path: PathBuf,
    /// Byte offset of row 0's vector in the file.
    base: u64,
}

impl VectorIndex {
    /// Write the vector file at `path` from `rows` (later rows for the same photo are dropped)
    /// and open it.
    pub fn build<'a>(
        path: &Path,
        model: ModelId,
        dims: usize,
        rows: impl IntoIterator<Item = RowIn<'a>>,
    ) -> std::io::Result<Self> {
        let mut ids = vec![];
        let mut by_id = HashMap::new();
        let mut scales = vec![];
        let mut bits = vec![];
        let mut has8 = vec![];
        let nb = dims.div_ceil(8);
        let tmp = path.with_extension("part");
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // Vectors are written after the header, whose size is known only at the end: write them
        // to the temporary file first, then the final file.
        let mut body = std::io::BufWriter::new(File::create(&tmp)?);
        let zero = vec![0u8; dims];
        for r in rows {
            if by_id.contains_key(&r.id) {
                continue;
            }
            by_id.insert(r.id, ids.len() as u32);
            ids.push(r.id);
            match r.vec8 {
                Some(v) => {
                    scales.push(v.scale);
                    let mut q: Vec<u8> = v.v.iter().map(|b| *b as u8).collect();
                    q.resize(dims, 0);
                    body.write_all(&q)?;
                    has8.push(true);
                }
                None => {
                    scales.push(0.0);
                    body.write_all(&zero)?;
                    has8.push(false);
                }
            }
            let mut b = r.vec1.map(<[u8]>::to_vec).unwrap_or_else(|| match r.vec8 {
                Some(v) => crate::shard::sign_bits(&v.to_f32()),
                None => vec![0; nb],
            });
            b.resize(nb, 0);
            bits.extend_from_slice(&b);
        }
        body.flush()?;
        drop(body);
        let mut out = std::io::BufWriter::new(File::create(path)?);
        out.write_all(MAGIC)?;
        out.write_all(&[VERSION])?;
        out.write_all(&model.0)?;
        out.write_all(&(dims as u32).to_le_bytes())?;
        out.write_all(&(ids.len() as u32).to_le_bytes())?;
        for id in &ids {
            out.write_all(&id.0)?;
        }
        for s in &scales {
            out.write_all(&s.to_le_bytes())?;
        }
        out.write_all(&bits)?;
        for h in &has8 {
            out.write_all(&[u8::from(*h)])?;
        }
        let base = 4 + 1 + 32 + 4 + 4 + ids.len() as u64 * (32 + 4 + 1) + bits.len() as u64;
        std::io::copy(&mut File::open(&tmp)?, &mut out)?;
        out.flush()?;
        drop(out);
        let _ = std::fs::remove_file(&tmp);
        Ok(Self {
            model,
            dims,
            ids,
            by_id,
            scales,
            bits,
            has8,
            path: path.to_path_buf(),
            base,
        })
    }

    /// Open a file written by [`VectorIndex::build`]; `None` if it is missing or not usable.
    pub fn open(path: &Path) -> Option<Self> {
        let mut f = File::open(path).ok()?;
        let mut head = [0u8; 4 + 1 + 32 + 4 + 4];
        f.read_exact(&mut head).ok()?;
        if &head[..4] != MAGIC || head[4] != VERSION {
            return None;
        }
        let model = ModelId(head[5..37].try_into().ok()?);
        let dims = u32::from_le_bytes(head[37..41].try_into().ok()?) as usize;
        let n = u32::from_le_bytes(head[41..45].try_into().ok()?) as usize;
        let nb = dims.div_ceil(8);
        let mut rest = vec![0u8; n * (32 + 4 + nb + 1)];
        f.read_exact(&mut rest).ok()?;
        let base = head.len() as u64 + rest.len() as u64;
        if f.metadata().ok()?.len() != base + (n * dims) as u64 {
            return None;
        }
        let (idb, rest) = rest.split_at(n * 32);
        let (sb, rest) = rest.split_at(n * 4);
        let (bits, hb) = rest.split_at(n * nb);
        let ids: Vec<PhotoId> = idb
            .as_chunks::<32>()
            .0
            .iter()
            .map(|c| shoebox_core::ids::Blake3Hash(*c))
            .collect();
        let by_id = ids
            .iter()
            .enumerate()
            .map(|(i, id)| (*id, i as u32))
            .collect();
        Some(Self {
            model,
            dims,
            by_id,
            ids,
            scales: sb
                .as_chunks::<4>()
                .0
                .iter()
                .map(|c| f32::from_le_bytes(*c))
                .collect(),
            bits: bits.to_vec(),
            has8: hb.iter().map(|b| *b != 0).collect(),
            path: path.to_path_buf(),
            base,
        })
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn contains(&self, id: &PhotoId) -> bool {
        self.by_id.contains_key(id)
    }

    pub fn ids(&self) -> &[PhotoId] {
        &self.ids
    }

    /// A photo's vector (dequantised), for "similar photos".
    pub fn vector(&self, id: &PhotoId) -> std::io::Result<Option<Vec<f32>>> {
        let Some(&row) = self.by_id.get(id) else {
            return Ok(None);
        };
        let r = row as usize;
        if !self.has8[r] {
            let nb = self.dims.div_ceil(8);
            let b = &self.bits[r * nb..(r + 1) * nb];
            return Ok(Some(
                (0..self.dims)
                    .map(|i| {
                        if b[i / 8] >> (i % 8) & 1 == 1 {
                            1.0
                        } else {
                            -1.0
                        }
                    })
                    .collect(),
            ));
        }
        let mut f = File::open(&self.path)?;
        f.seek(SeekFrom::Start(self.base + (r * self.dims) as u64))?;
        let mut q = vec![0u8; self.dims];
        f.read_exact(&mut q)?;
        Ok(Some(
            q.iter()
                .map(|b| f32::from(*b as i8) * self.scales[r])
                .collect(),
        ))
    }

    /// Cosine-like scores of `allowed` photos (all when `None`) against `query`, best first.
    /// Photos without a vector are left out.
    pub fn score(
        &self,
        query: &[f32],
        allowed: Option<&HashSet<PhotoId>>,
    ) -> std::io::Result<Vec<(PhotoId, f32)>> {
        let rows: Vec<u32> = match allowed {
            None => (0..self.ids.len() as u32).collect(),
            Some(set) => {
                let mut v: Vec<u32> = set
                    .iter()
                    .filter_map(|id| self.by_id.get(id).copied())
                    .collect();
                v.sort_unstable();
                v
            }
        };
        let q8 = Vec8::quantize(query);
        let rows = if rows.len() > SHORTLIST_ABOVE {
            self.shortlist(query, &rows, SHORTLIST)
        } else {
            rows
        };
        let mut out = Vec::with_capacity(rows.len());
        let only_bits: Vec<u32> = rows
            .iter()
            .copied()
            .filter(|r| !self.has8[*r as usize])
            .collect();
        if !only_bits.is_empty() {
            // A Viewer of a huge library has binary codes only: score by Hamming distance.
            let qb = crate::shard::sign_bits(query);
            let d = self.dims as f32;
            for r in only_bits {
                let h = self.hamming(r as usize, &qb) as f32;
                out.push((self.ids[r as usize], 1.0 - 2.0 * h / d));
            }
        }
        let wanted: Vec<u32> = rows
            .into_iter()
            .filter(|r| self.has8[*r as usize])
            .collect();
        self.scan(&wanted, |r, v| {
            let dot: i32 = v
                .iter()
                .zip(&q8.v)
                .map(|(a, b)| i32::from(*a as i8) * i32::from(*b))
                .sum();
            out.push((self.ids[r], dot as f32 * self.scales[r] * q8.scale));
        })?;
        out.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        Ok(out)
    }

    fn hamming(&self, r: usize, q: &[u8]) -> u32 {
        let nb = self.dims.div_ceil(8);
        self.bits[r * nb..(r + 1) * nb]
            .iter()
            .zip(q)
            .map(|(a, b)| (a ^ b).count_ones())
            .sum()
    }

    /// The `k` rows of `rows` closest to `query` by Hamming distance on the binary codes.
    fn shortlist(&self, query: &[f32], rows: &[u32], k: usize) -> Vec<u32> {
        let qb = crate::shard::sign_bits(query);
        let mut scored: Vec<(u32, u32)> = rows
            .iter()
            .map(|&r| (self.hamming(r as usize, &qb), r))
            .collect();
        let k = k.min(scored.len());
        if k > 0 && k < scored.len() {
            scored.select_nth_unstable(k - 1);
        }
        let mut out: Vec<u32> = scored[..k].iter().map(|(_, r)| *r).collect();
        out.sort_unstable();
        out
    }

    /// Read the vectors of `rows` (ascending) and hand each to `f`: in one sequential pass when
    /// many rows are wanted, else row by row.
    fn scan(&self, rows: &[u32], mut f: impl FnMut(usize, &[u8])) -> std::io::Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        let d = self.dims;
        let mut file = File::open(&self.path)?;
        if rows.len() * 8 < self.ids.len() {
            let mut buf = vec![0u8; d];
            for &r in rows {
                file.seek(SeekFrom::Start(self.base + (r as usize * d) as u64))?;
                file.read_exact(&mut buf)?;
                f(r as usize, &buf);
            }
            return Ok(());
        }
        let first = rows[0] as usize;
        let last = *rows.last().expect("non-empty") as usize;
        file.seek(SeekFrom::Start(self.base + (first * d) as u64))?;
        let mut reader = std::io::BufReader::with_capacity(1 << 20, file);
        let mut buf = vec![0u8; d];
        let mut next = rows.iter().peekable();
        for r in first..=last {
            reader.read_exact(&mut buf)?;
            if next.peek().is_some_and(|w| **w as usize == r) {
                next.next();
                f(r, &buf);
            }
        }
        Ok(())
    }
}

/// Reciprocal-rank fusion of ranked lists (k = 60): photos high in several lists come first.
pub fn fuse(lists: &[&[PhotoId]]) -> Vec<PhotoId> {
    let mut score: HashMap<PhotoId, f64> = HashMap::new();
    for list in lists {
        for (rank, id) in list.iter().enumerate() {
            *score.entry(*id).or_default() += 1.0 / (60.0 + rank as f64 + 1.0);
        }
    }
    let mut v: Vec<(PhotoId, f64)> = score.into_iter().collect();
    v.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    v.into_iter().map(|(id, _)| id).collect()
}

/// Keep the meaning results worth showing: scores are not calibrated, so the cut is relative to
/// the best (within `margin`), and at most `max` (S1-12 tunes both on the eval set).
pub fn cut(scored: &[(PhotoId, f32)], margin: f32, max: usize) -> Vec<PhotoId> {
    let Some(best) = scored.first().map(|s| s.1) else {
        return vec![];
    };
    scored
        .iter()
        .take_while(|(_, s)| *s >= best - margin)
        .take(max)
        .map(|(id, _)| *id)
        .collect()
}
