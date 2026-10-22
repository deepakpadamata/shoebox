//! M6-03: failure scenarios. Each asserts that no original is lost and that the copy counts the
//! user sees are truthful. Described in `docs/notes/failure-scenarios.md`.

mod common;

use common::*;
use shoebox_core::identity::KeyPair;
use shoebox_core::ids::Blake3Hash;
use shoebox_core::model::Role;
use shoebox_core::store::*;
use shoebox_node::{FetcherConfig, ImportOptions, LibraryNode};
use shoebox_store::Library;
use shoebox_testkit::{wait_until, Cluster};
use std::path::{Path, PathBuf};
use std::time::Duration;

fn fast_fetch() -> FetcherConfig {
    FetcherConfig {
        concurrency: 4,
        tick: Duration::from_millis(200),
        retry: Duration::from_millis(300),
    }
}

fn samples() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/samples");
    ["photo-gps.jpg", "IMG_0001.jpg", "screenshot.png"]
        .iter()
        .map(|f| dir.join(f))
        .collect()
}

async fn originals_on(n: &LibraryNode<Library>, ids: &[Blake3Hash]) -> usize {
    let mut have = 0;
    for id in ids {
        if let Some(p) = n.store().photo(id).await.unwrap() {
            if n.node().blobs().has(&p.original.hash).await.unwrap() {
                have += 1;
            }
        }
    }
    have
}

async fn all_copy_counts(nodes: &[&LibraryNode<Library>], ids: &[Blake3Hash]) -> Vec<Vec<usize>> {
    let mut out = vec![];
    for n in nodes {
        let mut row = vec![];
        for id in ids {
            row.push(n.copy_count(id).await.unwrap());
        }
        out.push(row);
    }
    out
}

/// A keeper that joins after the library has history gets all of it, and the counts agree.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_keeper_that_joins_late_catches_up_and_counts_are_truthful() {
    let c = Cluster::with_roles(&[Role::Phone, Role::Phone, Role::Keeper])
        .await
        .unwrap();
    let nodes = library_nodes_auto(&c).await;
    c.take_offline(2);
    let report = nodes[0]
        .import_paths(&samples(), ImportOptions::default())
        .await
        .unwrap();
    let ids = report.imported.clone();
    assert_eq!(ids.len(), 3);
    wait_until(Duration::from_secs(30), || async {
        photos_of(nodes[1].store().as_ref()).await.len() == 3
    })
    .await
    .unwrap();
    nodes[1].store().set_favorite(&ids[0], true).await.unwrap();
    nodes[1].store().mark_deleted(&ids[2]).await.unwrap();
    // Before the keeper is back, only the importer holds the originals: two copies are missing.
    assert_eq!(nodes[0].copy_count(&ids[0]).await.unwrap(), 1);

    c.bring_online(2);
    nodes[2].start_fetcher(fast_fetch());
    wait_until(Duration::from_secs(60), || async {
        converged(&nodes).await
    })
    .await
    .expect("the late keeper converges with everyone");
    let live = [ids[0], ids[1]];
    wait_until(Duration::from_secs(60), || async {
        originals_on(&nodes[2], &live).await == 2
    })
    .await
    .expect("the keeper fetches the originals that are still wanted");
    assert_eq!(
        originals_on(&nodes[0], &ids).await,
        3,
        "the importer lost nothing"
    );
    // Counts: every live photo is on the importer and the keeper, and everyone agrees.
    wait_until(Duration::from_secs(60), || async {
        let rows = all_copy_counts(&[&nodes[0], &nodes[1], &nodes[2]], &live).await;
        rows.iter().all(|r| r == &vec![2, 2])
    })
    .await
    .expect("every device reports two copies");
    assert!(photos_of(nodes[2].store().as_ref())
        .await
        .iter()
        .any(|p| p.id == ids[0] && p.favorite.value));
    c.shutdown().await;
}

/// A delete and a favorite made on different devices at the same time both apply, on every
/// device, and the blob stays until maintenance reclaims it (the 30-day window).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_delete_racing_a_favorite_converges_and_loses_no_original() {
    let c = Cluster::with_roles(&[Role::Keeper, Role::Keeper])
        .await
        .unwrap();
    let nodes = library_nodes_auto(&c).await;
    let report = nodes[0]
        .import_paths(&samples()[..1], ImportOptions::default())
        .await
        .unwrap();
    let id = report.imported[0];
    wait_until(Duration::from_secs(30), || async {
        nodes[1].store().photo(&id).await.unwrap().is_some()
    })
    .await
    .unwrap();
    c.partition(0, 1);
    c.clock().advance(1_000);
    nodes[0].store().mark_deleted(&id).await.unwrap();
    nodes[1].store().set_favorite(&id, true).await.unwrap();
    c.heal();
    wait_until(Duration::from_secs(40), || async {
        let (a, b) = (
            nodes[0].store().photo(&id).await.unwrap().unwrap(),
            nodes[1].store().photo(&id).await.unwrap().unwrap(),
        );
        a == b && a.is_deleted() && a.favorite.value
    })
    .await
    .expect("both devices end with the photo deleted and favorited");
    let p = nodes[0].store().photo(&id).await.unwrap().unwrap();
    assert!(
        nodes[0].node().blobs().has(&p.original.hash).await.unwrap(),
        "the original is still on disk"
    );
    // Restoring brings it back everywhere.
    nodes[1].store().restore_photo(&id).await.unwrap();
    wait_until(Duration::from_secs(40), || async {
        !nodes[0]
            .store()
            .photo(&id)
            .await
            .unwrap()
            .unwrap()
            .is_deleted()
    })
    .await
    .unwrap();
    c.shutdown().await;
}

/// Revoking a keeper while it is still downloading: it stops, the originals are untouched on the
/// devices that had them, and the revoked keeper's copies stop counting.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn revoking_a_device_mid_transfer_keeps_every_original_and_stops_counting_its_copies() {
    let c = Cluster::new(3).await.unwrap();
    let root = KeyPair::generate();
    let (_rig, nodes) = library_nodes_with_root(&c, root, &[Role::Keeper; 3]).await;
    for n in &nodes {
        n.enforce_membership().await.unwrap();
    }
    let report = nodes[0]
        .import_paths(&samples(), ImportOptions::default())
        .await
        .unwrap();
    let ids = report.imported.clone();
    nodes[1].start_fetcher(fast_fetch());
    nodes[2].start_fetcher(fast_fetch());
    // Let the second keeper get going, then cut the first one off (as an admin).
    wait_until(Duration::from_secs(30), || async {
        photos_of(nodes[1].store().as_ref()).await.len() == 3
    })
    .await
    .unwrap();
    c.clock().advance(1_000);
    nodes[0]
        .revoke_device(&core_id(&c, 1))
        .await
        .expect("node 0 is an admin");
    // The other keeper still completes; the importer lost nothing.
    wait_until(Duration::from_secs(90), || async {
        originals_on(&nodes[2], &ids).await == 3
    })
    .await
    .expect("the remaining keeper gets every original");
    assert_eq!(originals_on(&nodes[0], &ids).await, 3);
    // Truth in the counts: importer + remaining keeper = 2, the revoked device's copies are not counted.
    wait_until(Duration::from_secs(60), || async {
        all_copy_counts(&[&nodes[0], &nodes[2]], &ids)
            .await
            .iter()
            .all(|r| r.iter().all(|n| *n == 2))
    })
    .await
    .expect("two copies, not three");
    c.shutdown().await;
}

/// A keeper whose disk refuses writes does not claim to hold what it could not store, keeps
/// trying, and finishes once there is room again.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_keeper_with_a_full_disk_reports_only_what_it_holds_and_recovers() {
    use std::os::unix::fs::PermissionsExt;
    let c = Cluster::with_roles(&[Role::Phone, Role::Keeper])
        .await
        .unwrap();
    let nodes = library_nodes_auto(&c).await;
    let report = nodes[0]
        .import_paths(&samples(), ImportOptions::default())
        .await
        .unwrap();
    let ids = report.imported.clone();
    wait_until(Duration::from_secs(30), || async {
        photos_of(nodes[1].store().as_ref()).await.len() == 3
    })
    .await
    .unwrap();

    // "Disk full": the blob store directories stop accepting new files.
    fn walk(dir: &Path, mode: u32) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                walk(&e.path(), mode);
            }
        }
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(mode));
    }
    let blobs_dir = c.node(1).config().store_dir().join("blobs");
    walk(&blobs_dir, 0o555);
    let before = originals_on(&nodes[1], &ids).await;
    nodes[1].start_fetcher(fast_fetch());
    tokio::time::sleep(Duration::from_secs(4)).await;
    let during = originals_on(&nodes[1], &ids).await;
    // Whatever it could not write is not announced as held: the importer's counts show it.
    let counts = all_copy_counts(&[&nodes[0]], &ids).await.remove(0);
    assert!(
        counts.iter().filter(|n| **n >= 2).count() <= during,
        "copies announced ({counts:?}) must not exceed originals really held ({during})"
    );
    assert_eq!(
        originals_on(&nodes[0], &ids).await,
        3,
        "the importer lost nothing"
    );

    // Space comes back: the keeper catches up on its own.
    walk(&blobs_dir, 0o755);
    wait_until(Duration::from_secs(90), || async {
        originals_on(&nodes[1], &ids).await == 3
    })
    .await
    .unwrap_or_else(|_| panic!("keeper never recovered (had {before}, then {during})"));
    wait_until(Duration::from_secs(60), || async {
        all_copy_counts(&[&nodes[0]], &ids).await[0]
            .iter()
            .all(|n| *n == 2)
    })
    .await
    .expect("counts catch up once the originals are really there");
    c.shutdown().await;
}

/// Devices whose clocks are a day apart still converge. A clock that is ahead wins conflicts
/// while it is ahead (last-writer-wins is by hybrid time); nothing is lost either way.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_clock_a_day_off_changes_who_wins_but_never_loses_anything() {
    use shoebox_core::clock::{Clock, ManualClock};
    let c = Cluster::new(2).await.unwrap();
    let day = 86_400_000u64;
    let base = c.clock().now_ms() + 5 * day;
    let (ahead, behind) = (ManualClock::new(base + day), ManualClock::new(base - day));
    let (rig, libs) = rig_with_clocks(
        &c,
        KeyPair::generate(),
        &[Role::Keeper; 2],
        &[ahead.clone(), behind.clone()],
    )
    .await;
    let mut libs = libs.into_iter();
    let n0 = LibraryNode::with_sync_config(
        c.node(0).clone(),
        std::sync::Arc::new(libs.next().unwrap()),
        ahead.clone(),
        fast(),
    )
    .with_capability(rig.cap.clone());
    let n1 = LibraryNode::with_sync_config(
        c.node(1).clone(),
        std::sync::Arc::new(libs.next().unwrap()),
        behind.clone(),
        fast(),
    )
    .with_capability(rig.cap.clone());
    n0.add_peer(peer_addr(&c, 1)).unwrap();
    n1.add_peer(peer_addr(&c, 0)).unwrap();

    n1.store().upsert_photo(photo(1, 5)).await.unwrap();
    n0.store().upsert_photo(photo(2, 6)).await.unwrap();
    wait_until(Duration::from_secs(30), || async {
        photos_of(n0.store().as_ref()).await.len() == 2
            && photos_of(n1.store().as_ref()).await.len() == 2
    })
    .await
    .expect("photos from both clocks arrive on both devices");
    // The device that is behind favorites later in real time, but its clock says a day earlier.
    n0.store().set_favorite(&hash(1), true).await.unwrap();
    behind.advance(60_000);
    n1.store().set_favorite(&hash(1), false).await.unwrap();
    wait_until(Duration::from_secs(30), || async {
        let (a, b) = (
            n0.store().photo(&hash(1)).await.unwrap().unwrap(),
            n1.store().photo(&hash(1)).await.unwrap().unwrap(),
        );
        a.favorite.value == b.favorite.value
    })
    .await
    .expect("both agree");
    assert!(
        n1.store()
            .photo(&hash(1))
            .await
            .unwrap()
            .unwrap()
            .favorite
            .value,
        "the clock that is ahead wins until the other catches up (documented trade-off)"
    );
    // Later, once the slow clock has passed, its edits win again.
    behind.advance(2 * day + 1_000);
    n1.store().set_favorite(&hash(1), false).await.unwrap();
    wait_until(Duration::from_secs(30), || async {
        !n0.store()
            .photo(&hash(1))
            .await
            .unwrap()
            .unwrap()
            .favorite
            .value
    })
    .await
    .expect("edits made after the clocks agree again win");
    c.shutdown().await;
}
