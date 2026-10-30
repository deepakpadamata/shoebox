//! S1-06: index shard and indexer records on signed replicas: who may write them, that they
//! follow a rotation, and that a replica from before they existed keeps them.

use shoebox_core::clock::ManualClock;
use shoebox_core::identity::{issue_admin_cert, issue_device_cert, KeyPair};
use shoebox_core::merge::Lww;
use shoebox_core::model::*;
use shoebox_core::store::*;
use shoebox_core::{Blake3Hash, EndpointId, Hlc, Timestamp};
use shoebox_store_loro::{LoroBackend, LoroLibrary, LoroNetwork};

const ROOT_SEED: [u8; 32] = [42; 32];

fn key(n: u8) -> KeyPair {
    KeyPair::from_seed([n; 32])
}

fn peer(k: u8) -> PeerAddr {
    PeerAddr::id_only(key(k).endpoint_id())
}

fn photo(n: u8, importer: EndpointId) -> PhotoRecord {
    let blob = |k: u8| BlobRef {
        hash: Blake3Hash([n.wrapping_add(k); 32]),
        size: 10,
        mime: "image/jpeg".into(),
    };
    let hlc = Hlc {
        physical_ms: 1,
        counter: 0,
        device: importer,
    };
    PhotoRecord {
        id: Blake3Hash([n; 32]),
        original: blob(0),
        preview: blob(1),
        thumb: blob(2),
        taken_at: Timestamp(i64::from(n)),
        exif: ExifSummary::default(),
        imported_by: importer,
        favorite: Lww::new(false, hlc),
        deleted_at: Lww::new(None, hlc),
    }
}

fn shard(n: u8) -> IndexShard {
    IndexShard {
        blob: BlobRef {
            hash: Blake3Hash([n; 32]),
            size: 1_000,
            mime: "application/x-shoebox-index".into(),
        },
        model: ModelId([9; 32]),
        kind: ShardKind::Vec8,
        count: 1,
        replaces: vec![],
        made_by: EndpointId([0; 32]),
        hlc: Hlc::GENESIS,
    }
}

fn status(k: u8) -> IndexerStatus {
    IndexerStatus {
        device: key(k).endpoint_id(),
        models: vec![ModelId([9; 32])],
        kinds: vec![ShardKind::Vec8],
        photos_per_min: 30,
        always_on: true,
        hlc: Hlc::GENESIS,
    }
}

fn backend(net: &LoroNetwork, k: u8) -> LoroBackend {
    LoroBackend::new(net.clone(), key(k).endpoint_id(), ManualClock::new(1_000)).with_signer(key(k))
}

/// Device 1 (admin keeper) creates the library and certifies 2 (keeper) and 3 (viewer).
async fn rig(net: &LoroNetwork) -> (LoroLibrary, LibraryCapability) {
    let root = KeyPair::from_seed(ROOT_SEED);
    let (a, cap) = backend(net, 1)
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
    for (k, role) in [(1u8, Role::Keeper), (2, Role::Keeper), (3, Role::Viewer)] {
        let cert = issue_device_cert(
            &root,
            cap.library,
            key(k).endpoint_id(),
            role,
            "d",
            h(2),
            false,
        );
        a.apply_device_change(SignedDeviceChange::Device(cert))
            .await
            .unwrap();
    }
    (a, cap)
}

#[tokio::test]
async fn members_publish_shards_and_a_viewer_cannot() {
    let net = LoroNetwork::new();
    let (a, cap) = rig(&net).await;
    let b = backend(&net, 2).open_library(&cap).await.unwrap();
    let v = backend(&net, 3).open_library(&cap).await.unwrap();
    b.sync_with(&peer(1)).await.unwrap();
    v.sync_with(&peer(1)).await.unwrap();

    b.put_index_shard(shard(1)).await.unwrap();
    b.put_indexer_status(status(2)).await.unwrap();
    v.put_index_shard(shard(2)).await.unwrap(); // the viewer's local write
    v.put_indexer_status(status(3)).await.unwrap();
    a.sync_with(&peer(2)).await.unwrap();
    a.sync_with(&peer(3)).await.unwrap();

    let shards = a.index_shards().await.unwrap();
    assert_eq!(shards.len(), 1, "only the keeper's shard");
    assert_eq!(shards[0].made_by, key(2).endpoint_id());
    let statuses = a.indexer_statuses().await.unwrap();
    assert_eq!(
        statuses.iter().map(|s| s.device).collect::<Vec<_>>(),
        vec![key(2).endpoint_id()]
    );

    // Records follow the library into a new epoch.
    a.put_index_shard(shard(3)).await.unwrap();
    a.put_indexer_status(status(1)).await.unwrap();
    a.begin_rotation().await.unwrap();
    let carried: Vec<Blake3Hash> = a
        .index_shards()
        .await
        .unwrap()
        .iter()
        .map(|s| s.blob.hash)
        .collect();
    assert_eq!(carried, vec![Blake3Hash([1; 32]), Blake3Hash([3; 32])]);
    // Its own status only: the others republish theirs.
    let statuses = a.indexer_statuses().await.unwrap();
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].device, key(1).endpoint_id());
}

/// Index records written by this build, after passing through a replica of the build before
/// them (0.1.0): see `fixtures/README.md` for how the fixture was made. The old build rejected
/// them as undecodable (logged at `debug`), kept them in its document and handed them on; this
/// build accepts them.
#[tokio::test]
async fn records_kept_by_an_older_replica_are_accepted() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");
    let cap: LibraryCapability =
        postcard::from_bytes(&std::fs::read(format!("{dir}/index-records.cap")).unwrap()).unwrap();
    let via_old = std::fs::read(format!("{dir}/index-records-via-0.1.0.loro")).unwrap();
    let net = LoroNetwork::new();
    let lib = backend(&net, 4).open_library(&cap).await.unwrap();
    lib.import(&via_old).unwrap();
    assert_eq!(
        lib.photos_by_time(TimeRange::ALL, Page::all())
            .await
            .unwrap()
            .len(),
        2,
        "the photo from the new build and the one the old build added"
    );
    let shards = lib.index_shards().await.unwrap();
    assert_eq!(shards.len(), 1);
    assert_eq!(shards[0].blob.hash, Blake3Hash([1; 32]));
    assert_eq!(lib.indexer_statuses().await.unwrap().len(), 1);
}

/// Writes `fixtures/index-records.{cap,loro}` (run by hand; see `fixtures/README.md`).
#[tokio::test]
#[ignore]
async fn write_index_records_fixture() {
    let net = LoroNetwork::new();
    let (a, cap) = rig(&net).await;
    a.upsert_photo(photo(1, key(1).endpoint_id()))
        .await
        .unwrap();
    a.put_index_shard(shard(1)).await.unwrap();
    a.put_indexer_status(status(1)).await.unwrap();
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");
    std::fs::write(
        format!("{dir}/index-records.cap"),
        postcard::to_stdvec(&cap).unwrap(),
    )
    .unwrap();
    std::fs::write(format!("{dir}/index-records.loro"), a.export_snapshot()).unwrap();
}
