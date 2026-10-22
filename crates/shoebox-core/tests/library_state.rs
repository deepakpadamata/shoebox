//! Direct tests of the shared `LibraryState` (backends are covered by the conformance suite).

mod fixtures;

use fixtures::{dev, hash, hlc};
use shoebox_core::library_state::{LibraryState, StateEntry, Touched};
use shoebox_core::merge::{Lww, OrSet};
use shoebox_core::model::*;
use shoebox_core::store::*;
use shoebox_core::{Blake3Hash, Timestamp};

fn photo(n: u8, taken: i64) -> PhotoRecord {
    let blob = |b: u8| BlobRef {
        hash: Blake3Hash([b; 32]),
        size: 1,
        mime: "image/jpeg".into(),
    };
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

fn busy_state() -> LibraryState {
    let mut s = LibraryState::new(dev(1));
    for (n, t) in [(1, 10), (2, 20), (3, 20), (4, 5)] {
        s.upsert_photo(&photo(n, t));
    }
    s.set_favorite(&hash(2), true, 100).unwrap();
    s.mark_deleted(&hash(3), 200).unwrap();
    s.upsert_album(&AlbumMeta {
        id: AlbumId([1; 16]),
        name: Lww::new("A".into(), hlc(1, 0, 1)),
        cover: Lww::new(None, hlc(1, 0, 1)),
        items: OrSet::new(),
    });
    s.album_add(&AlbumId([1; 16]), &[hash(1), hash(2)], 300)
        .unwrap();
    s.album_remove(&AlbumId([1; 16]), &[hash(1)]).unwrap();
    s.apply_device_change(&SignedDeviceChange::Device(DeviceCert {
        device: dev(2),
        role: Role::Keeper,
        name: "k".into(),
        added_by: dev(1),
        hlc: hlc(5, 0, 1),
        revoked: false,
        signature: vec![1],
    }))
    .unwrap();
    s.put_holdings(
        dev(1),
        &HoldingsDelta {
            hlc: hlc(9, 0, 1),
            added: vec![(hash(1), BlobKind::Original)],
            removed: vec![],
        },
    )
    .unwrap();
    s.mark_deleted(&hash(4), 400).unwrap();
    s.purge_expired(Timestamp(400 + RETENTION_MS), 400 + RETENTION_MS as u64);
    s
}

#[test]
fn every_entry_replays_into_an_empty_state_in_any_order() {
    let a = busy_state();
    let entries = a.all_entries();
    assert_eq!(
        entries.len(),
        7,
        "3 photos, album, device, holding, purge marker"
    );

    let mut forward = LibraryState::new(dev(9));
    entries.iter().for_each(|e| {
        forward.apply_entry(e);
    });
    let mut backward = LibraryState::new(dev(8));
    entries.iter().rev().for_each(|e| {
        backward.apply_entry(e);
    });
    assert_eq!(
        forward.all_entries(),
        entries,
        "replay reproduces the state"
    );
    assert_eq!(backward.all_entries(), entries, "order does not matter");
    // Applying again is a no-op: no changes, nothing to persist.
    for e in &entries {
        assert!(forward.apply_entry(e).is_empty(), "idempotent: {e:?}");
    }
}

#[test]
fn writes_report_exactly_the_records_to_persist() {
    let mut s = LibraryState::new(dev(1));
    s.upsert_photo(&photo(1, 10));
    let fx = s.set_favorite(&hash(1), true, 50).unwrap();
    assert_eq!(fx.touched, vec![Touched::Photo(hash(1))]);
    assert!(
        matches!(s.entry(&Touched::Photo(hash(1))), Some(StateEntry::Photo(p)) if p.favorite.value)
    );

    // A no-op write reports nothing to persist.
    assert!(s.upsert_photo(&photo(1, 10)).touched.is_empty());
    assert!(
        s.restore_photo(&hash(1), 60).unwrap().touched.is_empty(),
        "not deleted"
    );

    // Purge touches the record (now gone) and the marker.
    s.mark_deleted(&hash(1), 100).unwrap();
    let (purged, fx) = s.purge_expired(Timestamp(100 + RETENTION_MS), 100 + RETENTION_MS as u64);
    assert_eq!(purged, vec![hash(1)]);
    assert!(
        fx.touched.contains(&Touched::Photo(hash(1)))
            && fx.touched.contains(&Touched::Purged(hash(1)))
    );
    assert!(
        s.entry(&Touched::Photo(hash(1))).is_none(),
        "nothing left to store for a purged photo"
    );
    assert!(matches!(
        s.entry(&Touched::Purged(hash(1))),
        Some(StateEntry::Purged { .. })
    ));
}

#[test]
fn timeline_index_matches_a_brute_force_listing_through_every_transition() {
    let mut s = LibraryState::new(dev(1));
    let brute = |s: &LibraryState| -> Vec<PhotoId> {
        let mut live: Vec<PhotoRecord> = (0..=255u8)
            .filter_map(|n| s.photo(&hash(n)))
            .filter(|p| !p.is_deleted())
            .collect();
        live.sort_by_key(|p| (std::cmp::Reverse(p.taken_at), p.id));
        live.into_iter().map(|p| p.id).collect()
    };
    let listed = |s: &LibraryState| -> Vec<PhotoId> {
        s.photos_by_time(TimeRange::ALL, Page::all())
            .into_iter()
            .map(|p| p.id)
            .collect()
    };
    for n in 0..40u8 {
        s.upsert_photo(&photo(n, (n % 7) as i64));
        assert_eq!(listed(&s), brute(&s));
    }
    for n in (0..40u8).step_by(3) {
        s.mark_deleted(&hash(n), 1_000 + n as u64).unwrap();
        assert_eq!(listed(&s), brute(&s));
    }
    for n in (0..40u8).step_by(6) {
        s.restore_photo(&hash(n), 2_000 + n as u64).unwrap();
        assert_eq!(listed(&s), brute(&s));
    }
    let due = 2_000 + RETENTION_MS as u64;
    s.purge_expired(Timestamp(due as i64), due);
    assert_eq!(listed(&s), brute(&s));
    // A re-delivered stale record must not re-enter the index.
    s.upsert_photo(&photo(3, 3));
    assert_eq!(listed(&s), brute(&s));
}

#[test]
fn copy_count_counts_devices_holding_originals_only() {
    let mut a = LibraryState::new(dev(1));
    let put = |s: &mut LibraryState,
               d: u8,
               ms,
               added: Vec<(Blake3Hash, BlobKind)>,
               removed: Vec<Blake3Hash>| {
        s.put_holdings(
            dev(d),
            &HoldingsDelta {
                hlc: hlc(ms, 0, d),
                added,
                removed,
            },
        )
        .unwrap();
    };
    put(&mut a, 1, 1, vec![(hash(7), BlobKind::Original)], vec![]);
    assert_eq!(a.copy_count(&hash(7)), 1);
    let mut b = LibraryState::new(dev(2));
    put(&mut b, 2, 2, vec![(hash(7), BlobKind::Thumb)], vec![]);
    a.merge_from(&b);
    assert_eq!(a.holders(&hash(7)).len(), 2);
    assert_eq!(a.copy_count(&hash(7)), 1, "a thumb is not a copy");
    put(&mut b, 2, 3, vec![(hash(7), BlobKind::Original)], vec![]);
    a.merge_from(&b);
    assert_eq!(a.copy_count(&hash(7)), 2);
    put(&mut a, 1, 4, vec![], vec![hash(7)]);
    assert_eq!(a.copy_count(&hash(7)), 1);
    assert_eq!(
        a.holders(&hash(7))
            .iter()
            .map(|h| h.device)
            .collect::<Vec<_>>(),
        vec![dev(2)]
    );
}
