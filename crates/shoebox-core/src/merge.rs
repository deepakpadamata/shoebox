//! Merge rules shared by every backend (M0-05).
//!
//! Every mergeable type forms a join-semilattice: `merge` is **commutative, associative and
//! idempotent**, so replicas converge regardless of delivery order or duplication. Property
//! tests in `tests/merge_props.rs` check this for each type.
//!
//! Semantics (see `docs/05-prototype-plan.md` §2 and `docs/notes/assumptions.md`):
//! - mutable scalar fields are [`Lww`] registers ordered by [`Hlc`] (device id breaks ties);
//! - album membership is an add-wins observed-remove set ([`OrSet`]);
//! - field edits never resurrect a deleted photo, and a restore is an explicit [`Lww`] write.

use crate::hlc::Hlc;
use crate::ids::Timestamp;
use crate::model::{AlbumMeta, PhotoRecord};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Merging two records that describe different entities.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("cannot merge records with different ids")]
pub struct IdMismatch;

/// Last-writer-wins register ordered by [`Hlc`] (device id breaks ties).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Lww<T> {
    pub value: T,
    pub hlc: Hlc,
}

impl<T: Ord + Clone> Lww<T> {
    pub fn new(value: T, hlc: Hlc) -> Self {
        Self { value, hlc }
    }

    /// The write with the greater HLC wins. Equal HLCs with different values cannot happen
    /// (an HLC is unique per device and write) but are resolved by value so merge stays total.
    pub fn merge(&self, other: &Self) -> Self {
        if (&self.hlc, &self.value) >= (&other.hlc, &other.value) {
            self.clone()
        } else {
            other.clone()
        }
    }

    /// Apply a local write; ignored if it is older than the current value.
    pub fn set(&mut self, value: T, hlc: Hlc) {
        *self = self.merge(&Lww { value, hlc });
    }
}

/// Add-wins observed-remove set. An element is present iff some add tag has not been observed
/// (and removed) by a remove, so a concurrent add and remove leaves the element in.
///
/// Tags are the [`Hlc`] of the add, which is unique per device and operation. Removed tags are
/// kept (a grow-only tombstone set); compaction is a later optimisation.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct OrSet<T: Ord> {
    pub adds: BTreeMap<T, BTreeSet<Hlc>>,
    pub removes: BTreeMap<T, BTreeSet<Hlc>>,
}

impl<T: Ord + Clone> OrSet<T> {
    pub fn new() -> Self {
        Self {
            adds: BTreeMap::new(),
            removes: BTreeMap::new(),
        }
    }

    /// Add `item` with a fresh unique `tag` (the HLC of this operation).
    pub fn add(&mut self, item: T, tag: Hlc) {
        self.adds.entry(item).or_default().insert(tag);
    }

    /// Remove `item`: tombstones exactly the add tags this replica has observed. Adds that
    /// arrive later or concurrently (unobserved tags) survive.
    pub fn remove(&mut self, item: &T) {
        if let Some(observed) = self.adds.get(item) {
            self.removes
                .entry(item.clone())
                .or_default()
                .extend(observed.iter().copied());
        }
    }

    pub fn contains(&self, item: &T) -> bool {
        match self.adds.get(item) {
            None => false,
            Some(tags) => {
                let removed = self.removes.get(item);
                tags.iter().any(|t| removed.is_none_or(|r| !r.contains(t)))
            }
        }
    }

    /// Present elements in sorted order.
    pub fn items(&self) -> Vec<T> {
        self.adds
            .keys()
            .filter(|k| self.contains(k))
            .cloned()
            .collect()
    }

    /// Pointwise union of add tags and remove tags.
    pub fn merge(&self, other: &Self) -> Self {
        fn union<K: Ord + Clone>(
            a: &BTreeMap<K, BTreeSet<Hlc>>,
            b: &BTreeMap<K, BTreeSet<Hlc>>,
        ) -> BTreeMap<K, BTreeSet<Hlc>> {
            let mut out = a.clone();
            for (k, tags) in b {
                out.entry(k.clone())
                    .or_default()
                    .extend(tags.iter().copied());
            }
            out
        }
        Self {
            adds: union(&self.adds, &other.adds),
            removes: union(&self.removes, &other.removes),
        }
    }
}

impl PhotoRecord {
    /// Merge two replicas of the same photo.
    ///
    /// - immutable fields (blobs, EXIF, importer) come from the content-addressed original; if
    ///   two importers produced different previews/thumbs, the lowest `(original, preview, thumb,
    ///   taken_at, exif, imported_by)` wins on every replica;
    /// - `favorite` and `deleted_at` are independent [`Lww`] registers.
    pub fn merge(&self, other: &Self) -> Result<Self, IdMismatch> {
        if self.id != other.id {
            return Err(IdMismatch);
        }
        let key = |p: &PhotoRecord| {
            (
                p.original.clone(),
                p.preview.clone(),
                p.thumb.clone(),
                p.taken_at,
                p.exif.clone(),
                p.imported_by,
            )
        };
        let winner = if key(self) <= key(other) { self } else { other };
        Ok(PhotoRecord {
            favorite: self.favorite.merge(&other.favorite),
            deleted_at: self.deleted_at.merge(&other.deleted_at),
            ..winner.clone()
        })
    }

    pub fn is_deleted(&self) -> bool {
        self.deleted_at.value.is_some()
    }

    /// Soft-delete. Beats any concurrent or earlier field edit (those never undelete) and is
    /// only undone by [`PhotoRecord::restore`] with a later HLC.
    pub fn mark_deleted(&mut self, at: Timestamp, hlc: Hlc) {
        self.deleted_at.set(Some(at), hlc);
    }

    /// Explicit restore from Recently Deleted: an LWW write of `None`, so it only wins if its
    /// HLC is after the delete's.
    pub fn restore(&mut self, hlc: Hlc) {
        self.deleted_at.set(None, hlc);
    }

    pub fn set_favorite(&mut self, favorite: bool, hlc: Hlc) {
        self.favorite.set(favorite, hlc);
    }
}

impl AlbumMeta {
    /// Merge two replicas of the same personal album (LWW name/cover, add-wins membership).
    pub fn merge(&self, other: &Self) -> Result<Self, IdMismatch> {
        if self.id != other.id {
            return Err(IdMismatch);
        }
        Ok(AlbumMeta {
            id: self.id,
            name: self.name.merge(&other.name),
            cover: self.cover.merge(&other.cover),
            items: self.items.merge(&other.items),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::EndpointId;

    fn h(physical_ms: u64, counter: u32, d: u8) -> Hlc {
        Hlc {
            physical_ms,
            counter,
            device: EndpointId([d; 32]),
        }
    }

    #[test]
    fn lww_later_hlc_wins_regardless_of_argument_order() {
        let a = Lww::new(1, h(10, 0, 1));
        let b = Lww::new(2, h(11, 0, 1));
        assert_eq!(a.merge(&b), b);
        assert_eq!(b.merge(&a), b);
    }

    #[test]
    fn lww_counter_then_device_break_ties() {
        let a = Lww::new("a", h(10, 1, 1));
        let b = Lww::new("b", h(10, 1, 2));
        assert_eq!(a.merge(&b).value, "b");
        assert_eq!(b.merge(&a).value, "b");
        let c = Lww::new("c", h(10, 2, 0));
        assert_eq!(a.merge(&c).value, "c");
    }

    #[test]
    fn lww_set_ignores_stale_writes() {
        let mut r = Lww::new(1, h(10, 0, 1));
        r.set(2, h(5, 0, 1));
        assert_eq!(r.value, 1);
        r.set(3, h(20, 0, 1));
        assert_eq!(r.value, 3);
    }

    #[test]
    fn orset_add_remove_basic() {
        let mut s = OrSet::new();
        s.add("x", h(1, 0, 1));
        assert!(s.contains(&"x"));
        s.remove(&"x");
        assert!(!s.contains(&"x"));
        s.add("x", h(2, 0, 1)); // re-add after remove works (fresh tag)
        assert!(s.contains(&"x"));
        assert_eq!(s.items(), vec!["x"]);
    }

    #[test]
    fn orset_remove_of_unknown_item_is_a_noop() {
        let mut s: OrSet<&str> = OrSet::new();
        s.remove(&"ghost");
        assert_eq!(s, OrSet::new());
    }

    #[test]
    fn orset_concurrent_add_and_remove_keeps_the_item() {
        // Both replicas start with x added by device 1.
        let mut base = OrSet::new();
        base.add("x", h(1, 0, 1));
        let mut replica_a = base.clone();
        let mut replica_b = base.clone();
        replica_a.remove(&"x"); // A removes the tag it observed
        replica_b.add("x", h(2, 0, 2)); // B concurrently adds again (new tag)
        let ab = replica_a.merge(&replica_b);
        let ba = replica_b.merge(&replica_a);
        assert!(ab.contains(&"x"), "add wins");
        assert_eq!(ab, ba);
    }

    #[test]
    fn orset_remove_after_seeing_all_adds_removes() {
        let mut a = OrSet::new();
        a.add("x", h(1, 0, 1));
        let mut b = OrSet::new();
        b.add("x", h(2, 0, 2));
        let mut merged = a.merge(&b);
        merged.remove(&"x"); // observed both tags
        assert!(!merged.contains(&"x"));
        assert!(
            !merged.merge(&a).contains(&"x"),
            "stale replica cannot resurrect"
        );
    }

    fn photo() -> PhotoRecord {
        use crate::ids::Blake3Hash;
        use crate::model::{BlobRef, ExifSummary};
        let blob = |b: u8| BlobRef {
            hash: Blake3Hash([b; 32]),
            size: 1,
            mime: "image/jpeg".into(),
        };
        PhotoRecord {
            id: Blake3Hash([1; 32]),
            original: blob(1),
            preview: blob(2),
            thumb: blob(3),
            taken_at: Timestamp(0),
            exif: ExifSummary::default(),
            imported_by: EndpointId([1; 32]),
            favorite: Lww::new(false, h(1, 0, 1)),
            deleted_at: Lww::new(None, h(1, 0, 1)),
        }
    }

    #[test]
    fn delete_versus_favorite_race_stays_deleted_and_keeps_the_edit() {
        let base = photo();
        let mut phone = base.clone();
        let mut laptop = base.clone();
        phone.mark_deleted(Timestamp(5_000), h(100, 0, 2));
        laptop.set_favorite(true, h(100, 0, 3)); // concurrent, even HLC-later than the delete
        let a = phone.merge(&laptop).unwrap();
        let b = laptop.merge(&phone).unwrap();
        assert_eq!(a, b);
        assert!(a.is_deleted(), "an edit never undeletes");
        assert!(a.favorite.value, "the edit is not lost");
    }

    #[test]
    fn restore_is_an_explicit_later_write() {
        let mut p = photo();
        p.mark_deleted(Timestamp(5_000), h(100, 0, 1));
        let mut stale_restore = p.clone();
        stale_restore.restore(h(50, 0, 2)); // older than the delete: loses
        assert!(p.merge(&stale_restore).unwrap().is_deleted());
        let mut restored = p.clone();
        restored.restore(h(200, 0, 2));
        let merged = p.merge(&restored).unwrap();
        assert!(!merged.is_deleted());
        assert_eq!(merged, restored.merge(&p).unwrap());
    }

    #[test]
    fn merge_rejects_different_ids() {
        let a = photo();
        let mut b = photo();
        b.id.0[0] = 9;
        assert_eq!(a.merge(&b), Err(IdMismatch));
    }

    #[test]
    fn conflicting_immutable_fields_resolve_identically_on_both_sides() {
        let a = photo();
        let mut b = photo();
        b.preview.hash.0[0] = 0xFF; // another importer rendered a different preview
        let ab = a.merge(&b).unwrap();
        assert_eq!(ab, b.merge(&a).unwrap());
        assert_eq!(ab.preview, a.preview.clone().min(b.preview.clone()));
    }

    #[test]
    fn album_merge_combines_lww_fields_and_add_wins_items() {
        use crate::model::AlbumId;
        let mut a = AlbumMeta {
            id: AlbumId([1; 16]),
            name: Lww::new("Trip".to_string(), h(1, 0, 1)),
            cover: Lww::new(None, h(1, 0, 1)),
            items: OrSet::new(),
        };
        a.items.add(crate::ids::Blake3Hash([7; 32]), h(2, 0, 1));
        let mut b = a.clone();
        a.items.remove(&crate::ids::Blake3Hash([7; 32]));
        b.name.set("Trip 2026".to_string(), h(3, 0, 2));
        b.items.add(crate::ids::Blake3Hash([7; 32]), h(4, 0, 2));
        let m = a.merge(&b).unwrap();
        assert_eq!(m.name.value, "Trip 2026");
        assert!(m.items.contains(&crate::ids::Blake3Hash([7; 32])));
        assert_eq!(m, b.merge(&a).unwrap());
    }
}
