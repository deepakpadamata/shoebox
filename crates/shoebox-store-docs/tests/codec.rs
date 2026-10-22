//! The iroh-docs key layout and value encoding are an on-disk/on-wire format: lock them.

mod common;

use common::*;
use shoebox_core::library_state::StateEntry;
use shoebox_store_docs::codec::*;

fn entries() -> Vec<StateEntry> {
    vec![
        StateEntry::Photo(photo(1, 10)),
        StateEntry::Album(album()),
        StateEntry::Device(device_cert()),
        StateEntry::Admin(admin_cert()),
        StateEntry::Holding {
            device: dev(2),
            blob: hash(3),
            kind: Some(shoebox_core::model::BlobKind::Thumb),
            hlc: hlc(5, 1, 2),
        },
        StateEntry::Holding {
            device: dev(2),
            blob: hash(4),
            kind: None,
            hlc: hlc(6, 0, 2),
        },
        StateEntry::Purged {
            id: hash(1),
            hlc: hlc(9, 0, 1),
        },
    ]
}

#[test]
fn every_library_entry_round_trips_through_its_key_and_bytes() {
    for e in entries() {
        let key = library_key(&e);
        let bytes = encode_library_entry(&e);
        assert_eq!(bytes[0], 1, "version byte");
        assert_eq!(decode_library_entry(&key, &bytes).unwrap(), e);
    }
}

#[test]
fn library_keys_follow_the_documented_layout() {
    let p = hash(0xAB);
    let hex = "ab".repeat(32);
    let d = "02".repeat(32);
    let b = "03".repeat(32);
    let mut photo = photo(0xAB, 1);
    photo.id = p;
    assert_eq!(
        library_key(&StateEntry::Photo(photo)),
        format!("photos/{hex}")
    );
    assert_eq!(
        library_key(&StateEntry::Album(album())),
        format!("albums/{}", "09".repeat(16))
    );
    assert_eq!(
        library_key(&StateEntry::Device(device_cert())),
        format!("devices/{d}")
    );
    assert_eq!(
        library_key(&StateEntry::Admin(admin_cert())),
        format!("admins/{}", "01".repeat(32))
    );
    assert_eq!(
        library_key(&StateEntry::Holding {
            device: dev(2),
            blob: hash(3),
            kind: None,
            hlc: hlc(1, 0, 2)
        }),
        format!("holdings/{d}/{b}")
    );
    assert_eq!(
        library_key(&StateEntry::Purged {
            id: p,
            hlc: hlc(1, 0, 1)
        }),
        format!("purged/{hex}")
    );
}

#[test]
fn a_value_stored_under_the_wrong_key_is_rejected() {
    let e = StateEntry::Photo(photo(1, 10));
    let bytes = encode_library_entry(&e);
    let wrong = format!("photos/{}", "ff".repeat(32));
    assert!(
        decode_library_entry(&wrong, &bytes).is_err(),
        "key must name the record's own id"
    );
    assert!(
        decode_library_entry("albums/00", &bytes).is_err(),
        "prefix must match the kind"
    );
}

#[test]
fn unknown_versions_and_garbage_are_errors_not_panics() {
    let e = StateEntry::Device(device_cert());
    let key = library_key(&e);
    let mut bytes = encode_library_entry(&e);
    bytes[0] = 9;
    assert!(decode_library_entry(&key, &bytes).is_err());
    assert!(decode_library_entry(&key, &[]).is_err());
    assert!(decode_library_entry(&key, &[1, 0xFF, 0xFF]).is_err());
    assert!(decode_library_entry("nonsense", &encode_library_entry(&e)).is_err());
}

#[test]
fn golden_bytes_lock_the_v1_wire_format() {
    // If this fails you changed the replicated encoding: bump the version byte and migrate.
    let e = StateEntry::Holding {
        device: dev(2),
        blob: hash(3),
        kind: Some(shoebox_core::model::BlobKind::Original),
        hlc: hlc(60, 0, 2),
    };
    assert_eq!(
        hex(&encode_library_entry(&e)),
        format!(
            "01{}{}{}{}{}{}",
            "04",
            "02".repeat(32),
            "03".repeat(32),
            "0100",
            "3c00",
            "02".repeat(32)
        )
    );
}

#[test]
fn album_entries_round_trip_and_key_layout() {
    use shoebox_core::album_state::AlbumEntry;
    use shoebox_core::store::*;
    let item = shared_item(7);
    let member = shoebox_core::model::Member {
        key: dev(5),
        display_name: "Asha".into(),
        role: shoebox_core::model::MemberRole::Contributor,
    };
    let event = AlbumEvent {
        hlc: hlc(10, 2, 5),
        member: dev(5),
        kind: AlbumEventKind::ItemsAdded { count: 3 },
    };
    let meta = SharedAlbumMeta {
        id: shoebox_core::model::AlbumId([7; 16]),
        name: "Trip".into(),
        cover: None,
        created_by: dev(1),
    };
    for (e, key) in [
        (AlbumEntry::Meta(meta), "meta".to_string()),
        (AlbumEntry::Item(item), format!("items/{}", "07".repeat(32))),
        (
            AlbumEntry::Member {
                hlc: hlc(3, 0, 5),
                member,
            },
            format!("members/{}", "05".repeat(32)),
        ),
        (
            AlbumEntry::Event(event),
            format!("events/{:016x}{:08x}{}", 10, 2, "05".repeat(32)),
        ),
    ] {
        assert_eq!(album_key(&e), key);
        assert_eq!(
            decode_album_entry(&key, &encode_album_entry(&e)).unwrap(),
            e
        );
        assert!(
            decode_album_entry("items/00", &encode_album_entry(&e)).is_err() || key == "items/00"
        );
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
