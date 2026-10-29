//! Signature checks remembered across restarts.
//!
//! Opening a replica judges every record again (role, revocation, whose holdings), against the
//! certificates as they are now; that part is cheap. Checking each record's signature is not: on a
//! phone it was most of the half second spent judging ~50,000 records. A record value whose
//! signature by a device already checked out is remembered here by a digest of its map key and
//! value, so the next open skips only the cryptography for it. Anything not remembered (a new
//! value, a damaged or missing file) is simply checked again.
//!
//! The file (`verdicts.bin` next to the snapshot) trusts the data folder, as the snapshot does:
//! whoever can write there can already change the library. Its layout: `SBV1`, the epoch id the
//! signatures were made for, the authors (32 bytes each), then `digest(16) ++ author index(u16)`
//! per record, and an 8-byte checksum of everything before it.

use shoebox_core::ids::EndpointId;
use shoebox_core::store::LibraryId;
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::Path;

const MAGIC: &[u8; 4] = b"SBV1";
pub(crate) const FILE: &str = "verdicts.bin";

pub(crate) type Digest = [u8; 16];

/// What a verdict is remembered by: the map key and the sealed value, signature included.
pub(crate) fn digest(key: &str, value: &[u8]) -> Digest {
    let mut h = blake3::Hasher::new();
    h.update(&(key.len() as u32).to_le_bytes());
    h.update(key.as_bytes());
    h.update(value);
    h.finalize().as_bytes()[..16].try_into().expect("16 bytes")
}

#[derive(Default)]
pub(crate) struct Verdicts {
    by_digest: HashMap<Digest, EndpointId>,
    /// Verdicts added since the file was last written.
    unsaved: usize,
}

impl Verdicts {
    /// The device whose signature over this record was already checked, if any.
    pub(crate) fn author(&self, d: &Digest) -> Option<EndpointId> {
        self.by_digest.get(d).copied()
    }

    pub(crate) fn insert(&mut self, d: Digest, author: EndpointId) {
        if self.by_digest.insert(d, author).is_none() {
            self.unsaved += 1;
        }
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.by_digest.len()
    }

    pub(crate) fn unsaved(&self) -> usize {
        self.unsaved
    }

    /// Forget verdicts for values no longer in the document.
    pub(crate) fn retain(&mut self, current: &HashSet<Digest>) {
        self.by_digest.retain(|d, _| current.contains(d));
    }

    /// The remembered verdicts for `library` in `dir`; none when the file is missing, damaged or
    /// for another epoch.
    pub(crate) fn load(dir: &Path, library: LibraryId) -> Self {
        let Ok(bytes) = std::fs::read(dir.join(FILE)) else {
            return Self::default();
        };
        match decode(&bytes, library) {
            Some(by_digest) => Self {
                by_digest,
                unsaved: 0,
            },
            None => {
                tracing::warn!("ignoring an unreadable {FILE}: signatures are checked again");
                Self::default()
            }
        }
    }

    pub(crate) fn encode(&self, library: LibraryId) -> Vec<u8> {
        let mut authors: Vec<EndpointId> = self.by_digest.values().copied().collect();
        authors.sort_by_key(|a| a.0);
        authors.dedup();
        let index: HashMap<EndpointId, u16> = authors
            .iter()
            .enumerate()
            .map(|(i, a)| (*a, i as u16))
            .collect();
        let mut out = Vec::with_capacity(32 + authors.len() * 32 + self.by_digest.len() * 18);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&library.0);
        out.extend_from_slice(&(authors.len() as u32).to_le_bytes());
        for a in &authors {
            out.extend_from_slice(&a.0);
        }
        out.extend_from_slice(&(self.by_digest.len() as u32).to_le_bytes());
        for (d, a) in &self.by_digest {
            out.extend_from_slice(d);
            out.extend_from_slice(&index[a].to_le_bytes());
        }
        let sum = blake3::hash(&out);
        out.extend_from_slice(&sum.as_bytes()[..8]);
        out
    }

    /// The file was written with everything known so far.
    pub(crate) fn mark_saved(&mut self) {
        self.unsaved = 0;
    }
}

fn decode(bytes: &[u8], library: LibraryId) -> Option<HashMap<Digest, EndpointId>> {
    let (body, sum) = bytes.split_at(bytes.len().checked_sub(8)?);
    if blake3::hash(body).as_bytes()[..8] != *sum {
        return None;
    }
    let mut at = 0usize;
    let mut take = |n: usize| -> Option<&[u8]> {
        let s = body.get(at..at + n)?;
        at += n;
        Some(s)
    };
    if take(4)? != MAGIC || take(16)? != library.0 {
        return None;
    }
    let n_authors = u32::from_le_bytes(take(4)?.try_into().ok()?) as usize;
    let mut authors = Vec::with_capacity(n_authors.min(1 << 16));
    for _ in 0..n_authors {
        authors.push(EndpointId(take(32)?.try_into().ok()?));
    }
    let n = u32::from_le_bytes(take(4)?.try_into().ok()?) as usize;
    let mut out = HashMap::with_capacity(n.min(1 << 20));
    for _ in 0..n {
        let d: Digest = take(16)?.try_into().ok()?;
        let i = u16::from_le_bytes(take(2)?.try_into().ok()?) as usize;
        out.insert(d, *authors.get(i)?);
    }
    (at == body.len()).then_some(out)
}

/// Write the verdicts file crash-safely (temp file, then rename).
pub(crate) fn write(dir: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = dir.join("verdicts.tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, dir.join(FILE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdicts_round_trip_and_refuse_damage_or_another_epoch() {
        let dir = tempfile::tempdir().unwrap();
        let lib = LibraryId([7; 16]);
        let mut v = Verdicts::default();
        v.insert(digest("k1", b"v1"), EndpointId([1; 32]));
        v.insert(digest("k2", b"v2"), EndpointId([2; 32]));
        v.insert(digest("k3", b"v3"), EndpointId([1; 32]));
        assert_eq!(v.unsaved(), 3);
        write(dir.path(), &v.encode(lib)).unwrap();

        let back = Verdicts::load(dir.path(), lib);
        assert_eq!(back.len(), 3);
        assert_eq!(back.author(&digest("k2", b"v2")), Some(EndpointId([2; 32])));
        assert_eq!(
            back.author(&digest("k2", b"other")),
            None,
            "a changed value is not known"
        );
        assert_eq!(back.unsaved(), 0);

        assert_eq!(
            Verdicts::load(dir.path(), LibraryId([8; 16])).len(),
            0,
            "another epoch"
        );
        let mut bytes = std::fs::read(dir.path().join(FILE)).unwrap();
        bytes[30] ^= 1;
        std::fs::write(dir.path().join(FILE), &bytes).unwrap();
        assert_eq!(Verdicts::load(dir.path(), lib).len(), 0, "damaged");
        std::fs::write(dir.path().join(FILE), b"x").unwrap();
        assert_eq!(Verdicts::load(dir.path(), lib).len(), 0, "too short");
    }

    #[test]
    fn retain_keeps_only_current_values() {
        let mut v = Verdicts::default();
        let (a, b) = (digest("k", b"old"), digest("k", b"new"));
        v.insert(a, EndpointId([1; 32]));
        v.insert(b, EndpointId([1; 32]));
        v.retain(&HashSet::from([b]));
        assert_eq!((v.author(&a), v.author(&b).is_some()), (None, true));
    }
}
