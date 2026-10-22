//! Deterministic sample records shared by the codec and merge tests.
#![allow(dead_code)]

use shoebox_core::merge::{Lww, OrSet};
use shoebox_core::model::*;
use shoebox_core::{Blake3Hash, EndpointId, Hlc, Timestamp};
use std::collections::{BTreeMap, BTreeSet};

pub fn dev(b: u8) -> EndpointId {
    EndpointId([b; 32])
}
pub fn hash(b: u8) -> Blake3Hash {
    Blake3Hash([b; 32])
}
pub fn hlc(physical_ms: u64, counter: u32, d: u8) -> Hlc {
    Hlc {
        physical_ms,
        counter,
        device: dev(d),
    }
}
pub fn blob(b: u8, kind: &str) -> BlobRef {
    BlobRef {
        hash: hash(b),
        size: 1000 + b as u64,
        mime: format!("image/{kind}"),
    }
}
pub fn exif() -> ExifSummary {
    ExifSummary {
        camera_make: Some("Fuji".into()),
        camera_model: Some("X100V".into()),
        lens: None,
        width: 6240,
        height: 4160,
        gps: Some(Gps {
            lat_e7: 515_074_000,
            lon_e7: -1_278_000,
        }),
    }
}
pub fn photo() -> PhotoRecord {
    PhotoRecord {
        id: hash(1),
        original: blob(1, "jpeg"),
        preview: blob(2, "webp"),
        thumb: blob(3, "webp"),
        taken_at: Timestamp(1_700_000_000_000),
        exif: exif(),
        imported_by: dev(7),
        favorite: Lww {
            value: true,
            hlc: hlc(10, 0, 7),
        },
        deleted_at: Lww {
            value: Some(Timestamp(1_700_000_100_000)),
            hlc: hlc(20, 1, 8),
        },
    }
}
pub fn album() -> AlbumMeta {
    let mut adds = BTreeMap::new();
    adds.insert(hash(1), BTreeSet::from([hlc(5, 0, 1), hlc(6, 0, 2)]));
    let mut removes = BTreeMap::new();
    removes.insert(hash(1), BTreeSet::from([hlc(5, 0, 1)]));
    AlbumMeta {
        id: AlbumId([9; 16]),
        name: Lww {
            value: "Summer".into(),
            hlc: hlc(3, 0, 1),
        },
        cover: Lww {
            value: Some(hash(1)),
            hlc: hlc(4, 0, 1),
        },
        items: OrSet { adds, removes },
    }
}
pub fn shared_item() -> SharedItem {
    SharedItem {
        id: hash(1),
        original: blob(1, "jpeg"),
        shared_hash: Some(hash(4)),
        preview: blob(2, "webp"),
        thumb: blob(3, "webp"),
        added_by: dev(7),
        added_at: Timestamp(1_700_000_200_000),
        taken_at: Timestamp(1_700_000_000_000),
    }
}
pub fn member() -> Member {
    Member {
        key: dev(5),
        display_name: "Asha".into(),
        role: MemberRole::Contributor,
    }
}
pub fn device_cert() -> DeviceCert {
    DeviceCert {
        device: dev(2),
        role: Role::Keeper,
        name: "Pi in the closet".into(),
        added_by: dev(1),
        hlc: hlc(100, 2, 1),
        revoked: false,
        signature: vec![0xAB; 4],
    }
}
pub fn admin_cert() -> AdminCert {
    AdminCert {
        admin: dev(1),
        hlc: hlc(50, 0, 0),
        expires_at: None,
        signature: vec![0xCD; 4],
    }
}
pub fn holding() -> Holding {
    Holding {
        device: dev(2),
        blob: hash(1),
        kind: BlobKind::Original,
        since: hlc(60, 0, 2),
    }
}
