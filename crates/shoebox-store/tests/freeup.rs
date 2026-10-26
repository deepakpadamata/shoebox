//! M2-08: "free up space" never frees an original unless enough other devices really hold it.

mod common;

use common::*;
use shoebox_core::ids::Blake3Hash;
use shoebox_core::model::Role;
use shoebox_node::{FetcherConfig, ImportOptions, LibraryNode, Protection};
use shoebox_store::Library;
use shoebox_testkit::{wait_until, Cluster};
use std::path::PathBuf;
use std::time::Duration;

const DAY: u64 = 86_400_000;

fn fast() -> FetcherConfig {
    FetcherConfig {
        concurrency: 6,
        tick: Duration::from_millis(300),
        retry: Duration::from_millis(300),
    }
}

/// A phone that imported all sample photos; every other node is a keeper that fetches originals.
async fn rig(keepers: usize) -> (Cluster, Vec<LibraryNode<Library>>, Vec<Blake3Hash>, u64) {
    let mut roles = vec![Role::Phone];
    roles.extend(std::iter::repeat_n(Role::Keeper, keepers));
    let c = Cluster::with_roles(&roles).await.unwrap();
    let nodes = library_nodes_auto(&c).await;
    let samples = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/samples");
    let files = ["photo-gps.jpg", "IMG_0001.jpg", "screenshot.png"].map(|f| samples.join(f));
    let ids = nodes[0]
        .import_paths(&files, ImportOptions::default())
        .await
        .unwrap()
        .imported;
    let mut bytes = 0;
    for id in &ids {
        bytes += nodes[0].node().blobs().size(id).await.unwrap().unwrap();
    }
    for n in &nodes[1..] {
        n.start_fetcher(fast());
    }
    wait_until(Duration::from_secs(40), || async {
        for n in &nodes[1..] {
            for id in &ids {
                if !n.node().blobs().has(id).await.unwrap() {
                    return false;
                }
            }
        }
        // ...and everybody has heard that those keepers hold them.
        // Every photo, not only the first: the others' holdings can arrive a moment later (R1-02).
        for id in &ids {
            if nodes[0].copy_count(id).await.unwrap() != 1 + keepers {
                return false;
            }
        }
        true
    })
    .await
    .expect("keepers fetch the originals and say so");
    (c, nodes, ids, bytes)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_has_probe_tells_the_truth() {
    let (c, nodes, ids, _) = rig(1).await;
    let keeper = shoebox_node::from_iroh_id(&c.node(1).id());
    let phantom = Blake3Hash([7; 32]);
    let got = nodes[0]
        .probe_holder(&keeper, &[ids[0], phantom, ids[1]])
        .await
        .unwrap();
    assert_eq!(got, vec![true, false, true]);
    c.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dry_run_and_real_run_agree_and_pins_are_kept() {
    let (c, nodes, ids, bytes) = rig(2).await; // two other copies = the default target of 2
    let phone = &nodes[0];
    let dry = phone.free_up_space(true).await.unwrap();
    assert_eq!(dry.bytes, bytes);
    assert_eq!(dry.photos.len(), 3);
    // Nothing changed yet.
    assert!(phone
        .node()
        .blobs()
        .protections(&ids[0])
        .await
        .unwrap()
        .contains(&Protection::Library));

    phone.pin_original(&ids[1]).await.unwrap();
    let dry = phone.free_up_space(true).await.unwrap();
    assert_eq!(dry.photos.len(), 2, "a pinned original is never freed");
    assert!(!dry.photos.contains(&ids[1]));
    let pinned_size = phone.node().blobs().size(&ids[1]).await.unwrap().unwrap();
    assert_eq!(dry.bytes, bytes - pinned_size);

    let done = phone.free_up_space(false).await.unwrap();
    assert_eq!(
        (done.bytes, done.photos.clone()),
        (dry.bytes, dry.photos.clone()),
        "the run frees what the dry run promised"
    );
    for id in &done.photos {
        assert!(
            phone
                .node()
                .blobs()
                .protections(id)
                .await
                .unwrap()
                .is_empty(),
            "protection released"
        );
    }
    assert!(phone.node().blobs().has(&ids[1]).await.unwrap());
    // Once the blob store collects them the bytes are really gone.
    wait_until(Duration::from_secs(20), || async {
        let mut gone = true;
        for id in &done.photos {
            gone &= !phone.node().blobs().has(id).await.unwrap();
        }
        gone
    })
    .await
    .expect("freed originals are collected");
    // The others still hold them and know this device no longer does.
    wait_until(Duration::from_secs(20), || async {
        nodes[1].copy_count(&ids[0]).await.unwrap() == 2
    })
    .await
    .expect("holdings updated");
    assert!(
        phone.free_up_space(true).await.unwrap().photos.is_empty(),
        "nothing left to free"
    );
    // And the phone does not fetch them back.
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(!phone.node().blobs().has(&ids[0]).await.unwrap());
    c.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn too_few_other_copies_means_nothing_is_freed() {
    let (c, nodes, _, _) = rig(1).await; // one other copy, target 2
                                         // That holder is no keeper (a keeper's copy alone is enough, see below): make it a phone. It
                                         // keeps the originals it already fetched.
    let holder = core_id(&c, 1);
    nodes[0]
        .set_device_role(&holder, Role::Phone)
        .await
        .unwrap();
    assert!(nodes[0]
        .free_up_space(true)
        .await
        .unwrap()
        .photos
        .is_empty());
    nodes[0].set_redundancy_target(1).await.unwrap();
    assert_eq!(nodes[0].free_up_space(true).await.unwrap().photos.len(), 3);
    c.shutdown().await;
}

/// R1-03: one keeper that confirms it holds the originals is enough, whatever the target: the
/// same rule as "backed up". Without its confirmation nothing is freed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_confirmed_keeper_copy_is_enough() {
    let (c, nodes, _, bytes) = rig(1).await; // one keeper, target 2
    let dry = nodes[0].free_up_space(true).await.unwrap();
    assert_eq!((dry.photos.len(), dry.bytes), (3, bytes));
    // The keeper stays away past the confirmation's 7 days: nothing any more.
    c.take_offline(1);
    c.clock().advance(8 * DAY);
    assert!(nodes[0]
        .free_up_space(true)
        .await
        .unwrap()
        .photos
        .is_empty());
    assert!(nodes[0]
        .free_up_space(false)
        .await
        .unwrap()
        .photos
        .is_empty());
    c.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_unconfirmed_holder_stops_counting_after_seven_days() {
    let (c, nodes, _, _) = rig(1).await;
    nodes[0].set_redundancy_target(1).await.unwrap();
    // A dry run probes the keeper and records the confirmation.
    assert_eq!(nodes[0].free_up_space(true).await.unwrap().photos.len(), 3);

    // The keeper drops off the network. For six days the confirmation still counts...
    c.take_offline(1);
    c.clock().advance(6 * DAY);
    assert_eq!(nodes[0].free_up_space(true).await.unwrap().photos.len(), 3);
    // ...after eight it does not, and nothing is freed.
    c.clock().advance(2 * DAY);
    let report = nodes[0].free_up_space(true).await.unwrap();
    assert!(report.photos.is_empty() && report.bytes == 0, "{report:?}");
    assert!(nodes[0]
        .free_up_space(false)
        .await
        .unwrap()
        .photos
        .is_empty());

    // Back online it is confirmed again and counts.
    c.bring_online(1);
    wait_until(Duration::from_secs(20), || async {
        nodes[0]
            .free_up_space(true)
            .await
            .map(|r| r.photos.len() == 3)
            .unwrap_or(false)
    })
    .await
    .expect("a reachable holder confirms again");
    c.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn keepers_never_free_space() {
    let (c, nodes, _, _) = rig(1).await;
    assert!(nodes[1].free_up_space(true).await.is_err());
    c.shutdown().await;
}
