#![allow(dead_code)]
use shoebox_core::merge::{Lww, OrSet};
use shoebox_core::model::*;
use shoebox_core::{Blake3Hash, EndpointId, Hlc, Timestamp};

pub fn dev(n: u8) -> EndpointId {
    EndpointId([n; 32])
}
pub fn hash(n: u8) -> Blake3Hash {
    Blake3Hash([n; 32])
}
pub fn hlc(ms: u64, counter: u32, d: u8) -> Hlc {
    Hlc {
        physical_ms: ms,
        counter,
        device: dev(d),
    }
}
pub fn blob(n: u8) -> BlobRef {
    BlobRef {
        hash: hash(n),
        size: 100 + n as u64,
        mime: "image/jpeg".into(),
    }
}
pub fn photo(n: u8, taken: i64) -> PhotoRecord {
    PhotoRecord {
        id: hash(n),
        original: blob(n),
        preview: blob(n.wrapping_add(100)),
        thumb: blob(n.wrapping_add(200)),
        taken_at: Timestamp(taken),
        exif: ExifSummary::default(),
        imported_by: dev(1),
        favorite: Lww::new(false, hlc(1, 0, 1)),
        deleted_at: Lww::new(None, hlc(1, 0, 1)),
    }
}
pub fn album() -> AlbumMeta {
    let mut items = OrSet::new();
    items.add(hash(1), hlc(2, 0, 1));
    AlbumMeta {
        id: AlbumId([9; 16]),
        name: Lww::new("Summer".into(), hlc(1, 0, 1)),
        cover: Lww::new(None, hlc(1, 0, 1)),
        items,
    }
}
pub fn device_cert() -> DeviceCert {
    DeviceCert {
        device: dev(2),
        role: Role::Keeper,
        name: "Pi".into(),
        added_by: dev(1),
        hlc: hlc(5, 0, 1),
        revoked: false,
        signature: vec![1, 2],
    }
}
pub fn admin_cert() -> AdminCert {
    AdminCert {
        admin: dev(1),
        hlc: hlc(1, 0, 1),
        signature: vec![9],
    }
}
pub fn shared_item(n: u8) -> SharedItem {
    SharedItem {
        id: hash(n),
        original: blob(n),
        shared_hash: None,
        preview: blob(n.wrapping_add(100)),
        thumb: blob(n.wrapping_add(200)),
        added_by: dev(1),
        added_at: Timestamp(10),
        taken_at: Timestamp(5),
    }
}
