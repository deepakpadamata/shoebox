//! The photos of a [`crate::library_state::LibraryState`], packed (M4-11).
//!
//! A [`PhotoRecord`] is about 600 bytes in memory once its strings are counted: six heap strings
//! (three MIME types, camera make, model and lens) and three 32-byte device ids (the importer and
//! one in each register's clock). Across a library those strings and devices are a handful of
//! distinct values, so here each is stored once in a table and a photo keeps small indexes into
//! it: about 250 bytes a photo. Records go in and come out as plain `PhotoRecord`s.

use crate::hlc::Hlc;
use crate::ids::{Blake3Hash, EndpointId, Timestamp};
use crate::merge::Lww;
use crate::model::{BlobRef, ExifSummary, Gps, PhotoId, PhotoRecord};
use std::collections::{BTreeMap, HashMap};
use std::ops::RangeBounds;
use std::sync::Arc;

/// Values stored once, referred to by index. Never shrinks: a library has few distinct values.
#[derive(Clone)]
struct Interner<T: Clone + Eq + std::hash::Hash> {
    values: Vec<T>,
    index: HashMap<T, u32>,
}

impl<T: Clone + Eq + std::hash::Hash> Default for Interner<T> {
    fn default() -> Self {
        Self {
            values: vec![],
            index: HashMap::new(),
        }
    }
}

impl<T: Clone + Eq + std::hash::Hash> Interner<T> {
    fn put(&mut self, v: &T) -> u32 {
        if let Some(i) = self.index.get(v) {
            return *i;
        }
        let i = self.values.len() as u32;
        self.values.push(v.clone());
        self.index.insert(v.clone(), i);
        i
    }

    fn get(&self, i: u32) -> &T {
        &self.values[i as usize]
    }
}

#[derive(Clone, Copy)]
struct Blob {
    hash: Blake3Hash,
    size: u64,
    mime: u32,
}

#[derive(Clone, Copy)]
struct Clock {
    physical_ms: u64,
    counter: u32,
    device: u32,
}

#[derive(Clone)]
struct Packed {
    original: Blob,
    preview: Blob,
    thumb: Blob,
    taken_at: Timestamp,
    /// Index + 1 into the strings; 0 = none.
    make: u32,
    model: u32,
    lens: u32,
    width: u32,
    height: u32,
    gps: Option<Gps>,
    imported_by: u32,
    favorite: bool,
    favorite_hlc: Clock,
    deleted_at: Option<Timestamp>,
    deleted_hlc: Clock,
}

#[derive(Clone, Default)]
pub(crate) struct PhotoTable {
    photos: BTreeMap<PhotoId, Packed>,
    strings: Interner<Arc<str>>,
    devices: Interner<EndpointId>,
}

impl PhotoTable {
    fn string(&mut self, s: &str) -> u32 {
        self.strings.put(&Arc::from(s))
    }

    fn maybe(&mut self, s: &Option<String>) -> u32 {
        s.as_deref().map_or(0, |s| self.string(s) + 1)
    }

    fn blob(&mut self, b: &BlobRef) -> Blob {
        Blob {
            hash: b.hash,
            size: b.size,
            mime: self.string(&b.mime),
        }
    }

    fn clock(&mut self, h: &Hlc) -> Clock {
        Clock {
            physical_ms: h.physical_ms,
            counter: h.counter,
            device: self.devices.put(&h.device),
        }
    }

    fn pack(&mut self, r: &PhotoRecord) -> Packed {
        Packed {
            original: self.blob(&r.original),
            preview: self.blob(&r.preview),
            thumb: self.blob(&r.thumb),
            taken_at: r.taken_at,
            make: self.maybe(&r.exif.camera_make),
            model: self.maybe(&r.exif.camera_model),
            lens: self.maybe(&r.exif.lens),
            width: r.exif.width,
            height: r.exif.height,
            gps: r.exif.gps,
            imported_by: self.devices.put(&r.imported_by),
            favorite: r.favorite.value,
            favorite_hlc: self.clock(&r.favorite.hlc),
            deleted_at: r.deleted_at.value,
            deleted_hlc: self.clock(&r.deleted_at.hlc),
        }
    }

    fn unpack(&self, id: &PhotoId, p: &Packed) -> PhotoRecord {
        let blob = |b: &Blob| BlobRef {
            hash: b.hash,
            size: b.size,
            mime: self.strings.get(b.mime).to_string(),
        };
        let maybe = |i: u32| (i > 0).then(|| self.strings.get(i - 1).to_string());
        let hlc = |c: &Clock| Hlc {
            physical_ms: c.physical_ms,
            counter: c.counter,
            device: *self.devices.get(c.device),
        };
        PhotoRecord {
            id: *id,
            original: blob(&p.original),
            preview: blob(&p.preview),
            thumb: blob(&p.thumb),
            taken_at: p.taken_at,
            exif: ExifSummary {
                camera_make: maybe(p.make),
                camera_model: maybe(p.model),
                lens: maybe(p.lens),
                width: p.width,
                height: p.height,
                gps: p.gps,
            },
            imported_by: *self.devices.get(p.imported_by),
            favorite: Lww::new(p.favorite, hlc(&p.favorite_hlc)),
            deleted_at: Lww::new(p.deleted_at, hlc(&p.deleted_hlc)),
        }
    }

    pub(crate) fn get(&self, id: &PhotoId) -> Option<PhotoRecord> {
        self.photos.get(id).map(|p| self.unpack(id, p))
    }

    pub(crate) fn contains_key(&self, id: &PhotoId) -> bool {
        self.photos.contains_key(id)
    }

    /// Whether the photo is in Recently Deleted, without unpacking it.
    pub(crate) fn is_deleted(&self, id: &PhotoId) -> Option<bool> {
        self.photos.get(id).map(|p| p.deleted_at.is_some())
    }

    pub(crate) fn insert(&mut self, rec: &PhotoRecord) -> Option<PhotoRecord> {
        let packed = self.pack(rec);
        let old = self.photos.insert(rec.id, packed)?;
        Some(self.unpack(&rec.id, &old))
    }

    pub(crate) fn remove(&mut self, id: &PhotoId) -> Option<PhotoRecord> {
        let old = self.photos.remove(id)?;
        Some(self.unpack(id, &old))
    }

    pub(crate) fn len(&self) -> usize {
        self.photos.len()
    }

    pub(crate) fn values(&self) -> impl Iterator<Item = PhotoRecord> + '_ {
        self.photos.iter().map(|(id, p)| self.unpack(id, p))
    }

    /// Ids with their original's hash and whether they are deleted, in id order (for `at_risk`).
    pub(crate) fn range(
        &self,
        r: impl RangeBounds<PhotoId>,
    ) -> impl Iterator<Item = (&PhotoId, Blake3Hash, bool)> + '_ {
        self.photos
            .range(r)
            .map(|(id, p)| (id, p.original.hash, p.deleted_at.is_some()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_comes_out_as_it_went_in() {
        let dev = |n: u8| EndpointId([n; 32]);
        let hlc = |n: u8| Hlc {
            physical_ms: 7,
            counter: 2,
            device: dev(n),
        };
        let blob = |n: u8, mime: &str| BlobRef {
            hash: Blake3Hash([n; 32]),
            size: n as u64,
            mime: mime.into(),
        };
        let full = PhotoRecord {
            id: Blake3Hash([1; 32]),
            original: blob(1, "image/heic"),
            preview: blob(2, "image/jpeg"),
            thumb: blob(3, "image/jpeg"),
            taken_at: Timestamp(-5),
            exif: ExifSummary {
                camera_make: Some("Apple".into()),
                camera_model: None,
                lens: Some("".into()),
                width: 4,
                height: 3,
                gps: Some(Gps {
                    lat_e7: 1,
                    lon_e7: -1,
                }),
            },
            imported_by: dev(9),
            favorite: Lww::new(true, hlc(8)),
            deleted_at: Lww::new(Some(Timestamp(99)), Hlc::GENESIS),
        };
        let bare = PhotoRecord {
            id: Blake3Hash([2; 32]),
            exif: ExifSummary::default(),
            deleted_at: Lww::new(None, hlc(9)),
            ..full.clone()
        };
        let mut t = PhotoTable::default();
        assert!(t.insert(&full).is_none());
        assert!(t.insert(&bare).is_none());
        assert_eq!(t.get(&full.id), Some(full.clone()));
        assert_eq!(t.get(&bare.id), Some(bare.clone()));
        assert_eq!(t.is_deleted(&full.id), Some(true));
        assert_eq!(t.insert(&bare), Some(bare.clone()));
        assert_eq!(t.values().collect::<Vec<_>>(), vec![full.clone(), bare]);
        assert_eq!(t.remove(&full.id), Some(full));
        assert_eq!(t.len(), 1);
    }
}
