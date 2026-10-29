//! M1-16: crash-safe persistence and record authenticity, on in-process replicas.

use shoebox_core::clock::ManualClock;
use shoebox_core::identity::{issue_admin_cert, issue_device_cert, KeyPair};
use shoebox_core::merge::Lww;
use shoebox_core::model::*;
use shoebox_core::store::*;
use shoebox_core::{Blake3Hash, EndpointId, Hlc, Timestamp};
use shoebox_store_loro::{LoroBackend, LoroNetwork};
use std::io::Write;

fn hash(n: u32) -> Blake3Hash {
    Blake3Hash::of_bytes(&n.to_le_bytes())
}

fn photo(n: u32, importer: EndpointId) -> PhotoRecord {
    let h = |k: u8| BlobRef {
        hash: Blake3Hash::of_bytes(&[&n.to_le_bytes()[..], &[k]].concat()),
        size: 10,
        mime: "image/jpeg".into(),
    };
    let hlc = Hlc {
        physical_ms: 1,
        counter: 0,
        device: importer,
    };
    PhotoRecord {
        id: hash(n),
        original: BlobRef {
            hash: hash(n),
            size: 100,
            mime: "image/jpeg".into(),
        },
        preview: h(1),
        thumb: h(2),
        taken_at: Timestamp(n as i64),
        exif: ExifSummary::default(),
        imported_by: importer,
        favorite: Lww::new(false, hlc),
        deleted_at: Lww::new(None, hlc),
    }
}

async fn count(l: &impl LibraryStore) -> usize {
    l.photos_by_time(TimeRange::ALL, Page::all())
        .await
        .unwrap()
        .len()
}

const ROOT_SEED: [u8; 32] = [42; 32];

#[tokio::test]
async fn a_library_survives_a_restart_a_torn_log_and_compaction() {
    let dir = tempfile::tempdir().unwrap();
    let net = LoroNetwork::new();
    let me = EndpointId([1; 32]);
    let root = KeyPair::from_seed(ROOT_SEED).root_public();
    let make =
        || LoroBackend::new(net_for(&net), me, ManualClock::new(100)).with_data_dir(dir.path());
    fn net_for(n: &LoroNetwork) -> LoroNetwork {
        n.clone()
    }

    let (lib, cap) = make().create_library(&root).await.unwrap();
    for n in 0..200 {
        lib.upsert_photo(photo(n, me)).await.unwrap();
    }
    lib.set_favorite(&hash(7), true).await.unwrap();
    drop(lib);

    // Same process, fresh backend and a fresh in-process network = a restart.
    let net2 = LoroNetwork::new();
    let reopened =
        LoroBackend::new(net2.clone(), me, ManualClock::new(200)).with_data_dir(dir.path());
    let lib = reopened.open_library(&cap).await.unwrap();
    assert_eq!(count(&lib).await, 200);
    assert!(lib.photo(&hash(7)).await.unwrap().unwrap().favorite.value);
    let (snapshot, log) = lib.disk_usage();
    assert_eq!(snapshot, 0, "nothing compacted yet");
    assert!(log > 0);
    drop(lib);

    // A crash mid-append leaves a torn frame: it is dropped, earlier records stay.
    let log_path = dir
        .path()
        .join(hex::encode(cap.library.0))
        .join("updates.log");
    let good_len = std::fs::metadata(&log_path).unwrap().len();
    std::fs::OpenOptions::new()
        .append(true)
        .open(&log_path)
        .unwrap()
        .write_all(&[200, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 9, 9])
        .unwrap();
    let lib = LoroBackend::new(LoroNetwork::new(), me, ManualClock::new(300))
        .with_data_dir(dir.path())
        .open_library(&cap)
        .await
        .unwrap();
    assert_eq!(count(&lib).await, 200);
    assert_eq!(
        std::fs::metadata(&log_path).unwrap().len(),
        good_len,
        "torn tail truncated"
    );
    lib.upsert_photo(photo(1000, me)).await.unwrap(); // appends after the valid frames

    lib.compact().unwrap();
    let (snapshot, log) = lib.disk_usage();
    assert!(snapshot > 0 && log == 0);
    lib.upsert_photo(photo(1001, me)).await.unwrap();
    drop(lib);
    let lib = LoroBackend::new(LoroNetwork::new(), me, ManualClock::new(400))
        .with_data_dir(dir.path())
        .open_library(&cap)
        .await
        .unwrap();
    assert_eq!(count(&lib).await, 202, "snapshot + later log");
}

struct Rig {
    root: KeyPair,
    net: LoroNetwork,
    lib: LibraryId,
    cap: LibraryCapability,
}

fn key(n: u8) -> KeyPair {
    KeyPair::from_seed([n; 32])
}

/// A library created by device 1 (admin keeper), with devices 2 (keeper) and 3 (viewer) certified.
async fn rig() -> (
    Rig,
    shoebox_store_loro::LoroLibrary,
    shoebox_store_loro::LoroLibrary,
    shoebox_store_loro::LoroLibrary,
) {
    let root = KeyPair::from_seed(ROOT_SEED);
    let net = LoroNetwork::new();
    let backend = |k: u8| {
        LoroBackend::new(net.clone(), key(k).endpoint_id(), ManualClock::new(1_000))
            .with_signer(key(k))
    };
    let (a, cap) = backend(1)
        .create_library(&root.root_public())
        .await
        .unwrap();
    let lib = cap.library;
    let h = |ms| Hlc {
        physical_ms: ms,
        counter: 0,
        device: key(1).endpoint_id(),
    };
    a.apply_device_change(SignedDeviceChange::Admin(issue_admin_cert(
        &root,
        lib,
        key(1).endpoint_id(),
        h(1),
        None,
    )))
    .await
    .unwrap();
    for (k, role) in [(1u8, Role::Keeper), (2, Role::Keeper), (3, Role::Viewer)] {
        let cert = issue_device_cert(&root, lib, key(k).endpoint_id(), role, "d", h(2), false);
        a.apply_device_change(SignedDeviceChange::Device(cert))
            .await
            .unwrap();
    }
    let b = backend(2).open_library(&cap).await.unwrap();
    let v = backend(3).open_library(&cap).await.unwrap();
    let peer = |k: u8| PeerAddr::id_only(key(k).endpoint_id());
    b.sync_with(&peer(1)).await.unwrap();
    v.sync_with(&peer(1)).await.unwrap();
    (
        Rig {
            root,
            net,
            lib,
            cap,
        },
        a,
        b,
        v,
    )
}

#[tokio::test]
async fn certified_writers_are_accepted_and_a_viewer_cannot_publish_photos() {
    let (r, a, b, v) = rig().await;
    let peer = |k: u8| PeerAddr::id_only(key(k).endpoint_id());
    b.upsert_photo(photo(1, key(2).endpoint_id()))
        .await
        .unwrap();
    v.upsert_photo(photo(2, key(3).endpoint_id()))
        .await
        .unwrap(); // the viewer's local write
    a.sync_with(&peer(2)).await.unwrap();
    a.sync_with(&peer(3)).await.unwrap();
    assert!(
        a.photo(&hash(1)).await.unwrap().is_some(),
        "a keeper's photo is accepted"
    );
    assert!(
        a.photo(&hash(2)).await.unwrap().is_none(),
        "a viewer's photo is not"
    );
    b.sync_with(&peer(1)).await.unwrap();
    assert!(
        b.photo(&hash(2)).await.unwrap().is_none(),
        "and it does not spread"
    );

    // A viewer may still announce what it holds.
    v.put_holdings(
        key(3).endpoint_id(),
        HoldingsDelta {
            hlc: Hlc {
                physical_ms: 5,
                counter: 0,
                device: key(3).endpoint_id(),
            },
            added: vec![(hash(1), BlobKind::Thumb)],
            removed: vec![],
        },
    )
    .await
    .unwrap();
    a.sync_with(&peer(3)).await.unwrap();
    assert_eq!(a.holders(&hash(1)).await.unwrap().len(), 1);
    let _ = (&r.net, r.lib, &r.cap, &r.root);
}

#[tokio::test]
async fn forged_authors_and_unsigned_records_are_rejected() {
    let (r, a, b, _v) = rig().await;
    let peer = |k: u8| PeerAddr::id_only(key(k).endpoint_id());

    // An uncertified device with its own valid key: waits, never accepted.
    let evil = LoroBackend::new(r.net.clone(), key(9).endpoint_id(), ManualClock::new(5))
        .with_signer(key(9));
    let e = evil.open_library(&r.cap).await.unwrap();
    e.upsert_photo(photo(10, key(9).endpoint_id()))
        .await
        .unwrap();
    a.sync_with(&peer(9)).await.unwrap();
    assert!(
        a.photo(&hash(10)).await.unwrap().is_none(),
        "unknown author"
    );

    // Forgeries are built in a private network and delivered as raw Loro updates, the way a
    // malicious peer would hand them over (the registry would otherwise return b's own replica).
    // A device claiming keeper 2's id but without its key: unsigned records are refused.
    let fake = LoroBackend::new(
        LoroNetwork::new(),
        key(2).endpoint_id(),
        ManualClock::new(5),
    );
    let f = fake.open_library(&r.cap).await.unwrap();
    f.upsert_photo(photo(11, key(2).endpoint_id()))
        .await
        .unwrap();
    a.import(&f.export_snapshot()).unwrap();
    assert!(
        a.photo(&hash(11)).await.unwrap().is_none(),
        "unsigned record in a certified library"
    );

    // A device with the wrong key for the id it claims.
    let liar = LoroBackend::new(
        LoroNetwork::new(),
        key(2).endpoint_id(),
        ManualClock::new(6),
    )
    .with_signer(key(8));
    let l = liar.open_library(&r.cap).await.unwrap();
    l.upsert_photo(photo(12, key(2).endpoint_id()))
        .await
        .unwrap();
    a.import(&l.export_snapshot()).unwrap();
    assert!(
        a.photo(&hash(12)).await.unwrap().is_none(),
        "signature by another key"
    );
    let _ = (&b, peer(2));
}

#[tokio::test]
async fn records_wait_for_their_authors_certificate_and_then_apply() {
    let (r, a, _b, _v) = rig().await;
    let peer = |k: u8| PeerAddr::id_only(key(k).endpoint_id());
    // Device 5 is certified later; its early records are held back, then accepted.
    let late = LoroBackend::new(r.net.clone(), key(5).endpoint_id(), ManualClock::new(5))
        .with_signer(key(5));
    let l = late.open_library(&r.cap).await.unwrap();
    l.upsert_photo(photo(20, key(5).endpoint_id()))
        .await
        .unwrap();
    a.sync_with(&peer(5)).await.unwrap();
    assert!(a.photo(&hash(20)).await.unwrap().is_none());

    let cert = issue_device_cert(
        &r.root,
        r.lib,
        key(5).endpoint_id(),
        Role::Companion,
        "late",
        Hlc {
            physical_ms: 9,
            counter: 0,
            device: key(1).endpoint_id(),
        },
        false,
    );
    a.apply_device_change(SignedDeviceChange::Device(cert))
        .await
        .unwrap();
    // The certificate arrives; the waiting record is retried and accepted.
    assert!(a.photo(&hash(20)).await.unwrap().is_some());
}

/// A replica left alone compacts and saves the signatures it checked; reopened, it keeps exactly
/// what a full check would keep: a demoted author's records go, a damaged verdicts file changes
/// nothing.
#[tokio::test]
async fn a_quiet_replica_compacts_and_its_remembered_signatures_change_nothing_a_restart_keeps() {
    let dir = tempfile::tempdir().unwrap();
    let root = KeyPair::from_seed(ROOT_SEED);
    let net = LoroNetwork::new();
    let backend = |k: u8, net: &LoroNetwork| {
        LoroBackend::new(net.clone(), key(k).endpoint_id(), ManualClock::new(1_000))
            .with_signer(key(k))
    };
    let (a, cap) = backend(1, &net)
        .with_data_dir(dir.path())
        .create_library(&root.root_public())
        .await
        .unwrap();
    let h = |ms| Hlc {
        physical_ms: ms,
        counter: 0,
        device: key(1).endpoint_id(),
    };
    a.apply_device_change(SignedDeviceChange::Admin(issue_admin_cert(
        &root,
        cap.library,
        key(1).endpoint_id(),
        h(1),
        None,
    )))
    .await
    .unwrap();
    let certify = |k: u8, role, ms| {
        SignedDeviceChange::Device(issue_device_cert(
            &root,
            cap.library,
            key(k).endpoint_id(),
            role,
            "d",
            h(ms),
            false,
        ))
    };
    for k in [1u8, 2] {
        a.apply_device_change(certify(k, Role::Keeper, 2))
            .await
            .unwrap();
    }
    let b = backend(2, &net).open_library(&cap).await.unwrap();
    b.sync_with(&PeerAddr::id_only(key(1).endpoint_id()))
        .await
        .unwrap();
    for n in 0..1_500 {
        b.upsert_photo(photo(n, key(2).endpoint_id()))
            .await
            .unwrap();
    }
    a.sync_with(&PeerAddr::id_only(key(2).endpoint_id()))
        .await
        .unwrap();
    assert_eq!(count(&a).await, 1_500);

    let replica = dir.path().join(hex::encode(cap.library.0));
    let (_, log) = a.disk_usage();
    a.compact_if_idle(std::time::Duration::from_secs(3_600));
    assert_eq!(a.disk_usage().1, log, "not quiet long enough: left alone");
    a.compact_if_idle(std::time::Duration::ZERO);
    let (snapshot, log) = a.disk_usage();
    assert!(snapshot > 0 && log == 0, "compacted");
    assert!(
        replica.join("verdicts.bin").exists(),
        "checked signatures saved"
    );
    drop(a);

    let reopen = || async {
        LoroBackend::new(
            LoroNetwork::new(),
            key(1).endpoint_id(),
            ManualClock::new(2_000),
        )
        .with_signer(key(1))
        .with_data_dir(dir.path())
        .open_library(&cap)
        .await
    };
    let a = reopen().await.unwrap();
    assert_eq!(count(&a).await, 1_500, "reopened from the snapshot");
    drop(a);

    // A damaged file only means checking again.
    std::fs::write(replica.join("verdicts.bin"), b"damaged").unwrap();
    let a = reopen().await.unwrap();
    assert_eq!(count(&a).await, 1_500);

    // Device 2 becomes a viewer. A restart judges every record against the certificates as they
    // are now, so its photos go, even though their signatures are remembered.
    a.compact().unwrap();
    a.apply_device_change(certify(2, Role::Viewer, 3))
        .await
        .unwrap();
    drop(a);
    let a = reopen().await.unwrap();
    assert_eq!(
        count(&a).await,
        0,
        "a viewer's records are refused on reopening"
    );
}
