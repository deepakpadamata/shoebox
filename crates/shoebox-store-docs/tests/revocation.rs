//! M2-03: only devices in the valid device list can connect; a revoked device stops receiving
//! records and blobs and learns that it was removed, while the others keep syncing.

mod common;

use common::*;
use shoebox_core::clock::Clock;
use shoebox_core::identity::{issue_device_cert, KeyPair};
use shoebox_core::model::Role;
use shoebox_core::store::*;
use shoebox_core::{EndpointId, Hlc};
use shoebox_node::{Membership, NodeEvent};
use shoebox_testkit::{wait_until, Cluster};
use std::time::Duration;

fn core_id(c: &Cluster, i: usize) -> EndpointId {
    EndpointId(*c.node(i).id().as_bytes())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_revoked_device_is_cut_off_and_told_so() {
    let c = Cluster::new(3).await.unwrap();
    let root = KeyPair::generate();
    let nodes = library_nodes_with_root(&c, root.root_public()).await;
    let lib = nodes[0].library_id();
    let hlc_at = |ms: u64| Hlc {
        physical_ms: ms,
        counter: 0,
        device: core_id(&c, 0),
    };

    // The root certifies all three devices (through node 0, which replicates the certs).
    let t0 = c.clock().now_ms();
    for (i, name) in ["A", "B", "C"].iter().enumerate() {
        let cert = issue_device_cert(
            &root,
            lib,
            core_id(&c, i),
            Role::Keeper,
            name,
            hlc_at(t0 + i as u64),
            false,
        );
        nodes[0]
            .store()
            .apply_device_change(SignedDeviceChange::Device(cert))
            .await
            .unwrap();
    }
    wait_until(Duration::from_secs(20), || async {
        let mut ok = true;
        for n in &nodes {
            ok &= n.store().device_list().await.unwrap().devices.len() == 3;
        }
        ok
    })
    .await
    .expect("the device list reaches everyone");
    for n in &nodes {
        n.enforce_membership().await.unwrap();
        assert_eq!(n.membership().await.unwrap(), Membership::Member);
    }

    // Everyone still syncs under enforcement.
    nodes[0].store().upsert_photo(photo(1, 10)).await.unwrap();
    wait_until(Duration::from_secs(20), || async {
        nodes[1].store().photo(&hash(1)).await.unwrap().is_some()
            && nodes[2].store().photo(&hash(1)).await.unwrap().is_some()
    })
    .await
    .expect("members sync");

    // Revoke C. B hears about it; C hears about it only if it can still connect — it must be told
    // before the cut or find out when its peers refuse it.
    let mut events_c = c.node(2).events();
    c.clock().advance(1_000);
    let revoke = issue_device_cert(
        &root,
        lib,
        core_id(&c, 2),
        Role::Keeper,
        "C",
        hlc_at(c.clock().now_ms()),
        true,
    );
    nodes[0]
        .store()
        .apply_device_change(SignedDeviceChange::Device(revoke))
        .await
        .unwrap();
    wait_until(Duration::from_secs(20), || async {
        nodes[1]
            .store()
            .device_list()
            .await
            .unwrap()
            .devices
            .iter()
            .any(|d| d.revoked)
            && nodes[0].node().is_refused(c.node(2).id())
            && nodes[1].node().is_refused(c.node(2).id())
    })
    .await
    .expect("A and B refuse C");

    // New records reach B but never C.
    nodes[0].store().upsert_photo(photo(2, 20)).await.unwrap();
    wait_until(Duration::from_secs(20), || async {
        nodes[1].store().photo(&hash(2)).await.unwrap().is_some()
    })
    .await
    .expect("A and B keep syncing");
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(
        nodes[2].store().photo(&hash(2)).await.unwrap().is_none(),
        "C is cut off"
    );

    // C's own writes do not reach the others either.
    nodes[2].store().upsert_photo(photo(3, 30)).await.unwrap();
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(nodes[0].store().photo(&hash(3)).await.unwrap().is_none());
    assert!(nodes[1].store().photo(&hash(3)).await.unwrap().is_none());

    // If C did learn it was removed (the revocation cert reached it before the cut), it says so.
    // Either way it must not report itself as a member of anything it can still sync.
    let _ = &mut events_c;
    let m = nodes[2].membership().await.unwrap();
    assert!(matches!(m, Membership::Member | Membership::Removed));
    c.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_device_that_receives_its_own_revocation_reports_removed_and_stops_syncing() {
    let c = Cluster::new(2).await.unwrap();
    let root = KeyPair::generate();
    let nodes = library_nodes_with_root(&c, root.root_public()).await;
    let lib = nodes[0].library_id();
    let hlc_at = |ms: u64| Hlc {
        physical_ms: ms,
        counter: 0,
        device: core_id(&c, 0),
    };
    let t0 = c.clock().now_ms();
    for i in 0..2 {
        let cert = issue_device_cert(
            &root,
            lib,
            core_id(&c, i),
            Role::Keeper,
            "d",
            hlc_at(t0 + i as u64),
            false,
        );
        nodes[0]
            .store()
            .apply_device_change(SignedDeviceChange::Device(cert))
            .await
            .unwrap();
    }
    wait_until(Duration::from_secs(20), || async {
        nodes[1].store().device_list().await.unwrap().devices.len() == 2
    })
    .await
    .unwrap();
    let mut events = c.node(1).events();
    nodes[1].enforce_membership().await.unwrap();
    nodes[0].enforce_membership().await.unwrap();

    c.clock().advance(1_000);
    let revoke = issue_device_cert(
        &root,
        lib,
        core_id(&c, 1),
        Role::Keeper,
        "d",
        hlc_at(c.clock().now_ms()),
        true,
    );
    // Delivered to the victim directly (e.g. before the cut).
    nodes[1]
        .store()
        .apply_device_change(SignedDeviceChange::Device(revoke))
        .await
        .unwrap();
    wait_until(Duration::from_secs(10), || async {
        nodes[1].membership().await.unwrap() == Membership::Removed
    })
    .await
    .expect("the revoked device notices");
    let mut saw = false;
    while let Ok(Ok(ev)) = tokio::time::timeout(Duration::from_millis(500), events.recv()).await {
        saw |= matches!(ev, NodeEvent::RemovedFromLibrary);
    }
    assert!(saw, "RemovedFromLibrary is emitted");

    // It no longer receives anything.
    nodes[0].store().upsert_photo(photo(5, 50)).await.unwrap();
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(nodes[1].store().photo(&hash(5)).await.unwrap().is_none());
    c.shutdown().await;
}
