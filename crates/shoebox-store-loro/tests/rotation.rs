//! M2-03b on the Loro store: a replica moves to a fresh epoch, carrying what is still valid;
//! devices that stay behind cannot reach it.

use shoebox_core::clock::ManualClock;
use shoebox_core::identity::{issue_admin_cert, issue_device_cert, KeyPair};
use shoebox_core::merge::Lww;
use shoebox_core::model::*;
use shoebox_core::store::*;
use shoebox_core::{Blake3Hash, EndpointId, Hlc, Timestamp};
use shoebox_store_loro::{LoroBackend, LoroNetwork};

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

fn key(n: u8) -> KeyPair {
    KeyPair::from_seed([n; 32])
}

fn peer(k: u8) -> PeerAddr {
    PeerAddr::id_only(key(k).endpoint_id())
}

struct World {
    net: LoroNetwork,
    cap: LibraryCapability,
}

impl World {
    fn backend(&self, k: u8) -> LoroBackend {
        LoroBackend::new(
            self.net.clone(),
            key(k).endpoint_id(),
            ManualClock::new(1_000),
        )
        .with_signer(key(k))
    }
}

/// Admin 1 plus keepers 2 and 3, all certified and synced.
async fn world() -> (World, [shoebox_store_loro::LoroLibrary; 3]) {
    let root = key(42);
    let net = LoroNetwork::new();
    let w0 = World {
        net: net.clone(),
        cap: LibraryCapability {
            library: LibraryId([0; 16]),
            root: root.root_public(),
            mode: Capability::Write,
            backend: vec![],
        },
        root: root.clone(),
    };
    let (a, cap) = w0
        .backend(1)
        .create_library(&root.root_public())
        .await
        .unwrap();
    let w = World {
        cap: cap.clone(),
        ..w0
    };
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
    for k in [1u8, 2, 3] {
        let cert = issue_device_cert(
            &root,
            cap.library,
            key(k).endpoint_id(),
            Role::Keeper,
            "d",
            h(2),
            false,
        );
        a.apply_device_change(SignedDeviceChange::Device(cert))
            .await
            .unwrap();
    }
    let b = w.backend(2).open_library(&cap).await.unwrap();
    let c = w.backend(3).open_library(&cap).await.unwrap();
    b.sync_with(&peer(1)).await.unwrap();
    c.sync_with(&peer(1)).await.unwrap();
    (w, [a, b, c])
}

#[tokio::test]
async fn an_admin_rotates_and_members_follow_while_a_stale_device_is_left_behind() {
    let (w, [a, b, c]) = world().await;
    for n in 0..50 {
        a.upsert_photo(photo(n, key(1).endpoint_id()))
            .await
            .unwrap();
    }
    b.upsert_photo(photo(1000, key(2).endpoint_id()))
        .await
        .unwrap();
    a.sync_with(&peer(2)).await.unwrap();
    c.sync_with(&peer(1)).await.unwrap();
    // b writes something that has not synced yet: it must survive its own move.
    b.upsert_photo(photo(1001, key(2).endpoint_id()))
        .await
        .unwrap();

    // Admin 1 moves to a new epoch (the revoked device would be 3 in a real flow).
    let new = a.begin_rotation().await.unwrap();
    assert_ne!(new.library, w.cap.library);
    assert_eq!(
        new.identity_id(),
        w.cap.identity_id(),
        "certificates keep their identity"
    );
    assert_eq!(
        a.photos_by_time(TimeRange::ALL, Page::all())
            .await
            .unwrap()
            .len(),
        51,
        "all records carried"
    );
    assert_eq!(
        a.device_list().await.unwrap().devices.len(),
        3,
        "and the device list"
    );

    // Member 2 follows; it carries its own unsynced photo and gets the rest by syncing.
    b.adopt_rotation(&new).await.unwrap();
    b.sync_with(&peer(1)).await.unwrap();
    a.sync_with(&peer(2)).await.unwrap();
    for l in [&a, &b] {
        assert_eq!(
            l.photos_by_time(TimeRange::ALL, Page::all())
                .await
                .unwrap()
                .len(),
            52
        );
    }
    assert!(
        a.photo(&hash(1001)).await.unwrap().is_some(),
        "member's unsynced record survived"
    );

    // Device 3 never moved: it can no longer reach anyone, and new records never reach it.
    assert!(
        c.sync_with(&peer(1)).await.is_err(),
        "the old epoch is gone"
    );
    assert!(c.sync_with(&peer(2)).await.is_err());
    a.upsert_photo(photo(2000, key(1).endpoint_id()))
        .await
        .unwrap();
    assert!(c.photo(&hash(2000)).await.unwrap().is_none());

    // New epoch keeps working both ways, with signatures still valid.
    b.set_favorite(&hash(2000), true).await.unwrap_err(); // b has not synced it yet
    b.sync_with(&peer(1)).await.unwrap();
    b.set_favorite(&hash(2000), true).await.unwrap();
    a.sync_with(&peer(2)).await.unwrap();
    assert!(a.photo(&hash(2000)).await.unwrap().unwrap().favorite.value);
}

#[tokio::test]
async fn a_rotated_replica_reopens_from_disk_under_the_new_id() {
    let dir = tempfile::tempdir().unwrap();
    let root = key(42);
    let net = LoroNetwork::new();
    let backend = || {
        LoroBackend::new(net.clone(), key(1).endpoint_id(), ManualClock::new(1_000))
            .with_signer(key(1))
            .with_data_dir(dir.path())
    };
    let (a, cap) = backend().create_library(&root.root_public()).await.unwrap();
    let h = Hlc {
        physical_ms: 1,
        counter: 0,
        device: key(1).endpoint_id(),
    };
    a.apply_device_change(SignedDeviceChange::Admin(issue_admin_cert(
        &root,
        cap.library,
        key(1).endpoint_id(),
        h,
        None,
    )))
    .await
    .unwrap();
    a.apply_device_change(SignedDeviceChange::Device(issue_device_cert(
        &root,
        cap.library,
        key(1).endpoint_id(),
        Role::Keeper,
        "d",
        h,
        false,
    )))
    .await
    .unwrap();
    for n in 0..30 {
        a.upsert_photo(photo(n, key(1).endpoint_id()))
            .await
            .unwrap();
    }
    let new = a.begin_rotation().await.unwrap();
    a.upsert_photo(photo(99, key(1).endpoint_id()))
        .await
        .unwrap();
    drop(a);
    assert!(
        !dir.path().join(hex_id(&cap.library)).exists(),
        "the old epoch's files are removed"
    );
    let again = LoroBackend::new(
        LoroNetwork::new(),
        key(1).endpoint_id(),
        ManualClock::new(2_000),
    )
    .with_signer(key(1))
    .with_data_dir(dir.path())
    .open_library(&new)
    .await
    .unwrap();
    assert_eq!(
        again
            .photos_by_time(TimeRange::ALL, Page::all())
            .await
            .unwrap()
            .len(),
        31
    );
    assert_eq!(again.device_list().await.unwrap().devices.len(), 1);
}

fn hex_id(id: &LibraryId) -> String {
    id.0.iter().map(|b| format!("{b:02x}")).collect()
}

/// Acceptance: a rotation with 100k records completes without data loss.
#[tokio::test]
#[ignore = "slow: 100k records (about a minute in a debug build)"]
async fn rotating_a_100k_record_library_loses_nothing() {
    let (_w, [a, _b, _c]) = world().await;
    let n = 100_000u32;
    for i in 0..n {
        a.upsert_photo(photo(i, key(1).endpoint_id()))
            .await
            .unwrap();
    }
    let t = std::time::Instant::now();
    a.begin_rotation().await.unwrap();
    eprintln!("rotated {n} records in {:.1?}", t.elapsed());
    assert_eq!(
        a.photos_by_time(TimeRange::ALL, Page::all())
            .await
            .unwrap()
            .len(),
        n as usize
    );
}
