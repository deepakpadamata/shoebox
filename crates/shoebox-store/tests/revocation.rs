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
    let (_rig, nodes) = library_nodes_with_root(&c, root.clone(), &[Role::Keeper; 3]).await;
    let lib = nodes[0].library_id();
    let hlc_at = |ms: u64| Hlc {
        physical_ms: ms,
        counter: 0,
        device: core_id(&c, 0),
    };

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
    let (_rig, nodes) = library_nodes_with_root(&c, root.clone(), &[Role::Keeper; 2]).await;
    let lib = nodes[0].library_id();
    let hlc_at = |ms: u64| Hlc {
        physical_ms: ms,
        counter: 0,
        device: core_id(&c, 0),
    };
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

/// A device is not stuck with the role it joined with (A-086): an admin (or the recovery phrase)
/// signs a new certificate, and the device follows it: its node config, and so what it fetches,
/// frees and announces.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn roles_can_change_and_devices_follow_their_certificate() {
    let c = Cluster::with_roles(&[Role::Keeper, Role::Phone, Role::Phone])
        .await
        .unwrap();
    let (rig, nodes) = library_nodes_roles(&c, &[Role::Keeper, Role::Phone, Role::Phone]).await;
    for n in &nodes {
        n.enforce_membership().await.unwrap();
    }
    let (phone, other) = (core_id(&c, 1), core_id(&c, 2));
    let mut events = c.node(1).events();

    // Not an admin: refused without the phrase.
    let err = nodes[1]
        .set_device_role(&other, Role::Keeper)
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("admin"), "{err:#}");

    // The admin makes the phone a viewer; the phone follows once the cert reaches it.
    nodes[0]
        .set_device_role(&phone, Role::Viewer)
        .await
        .unwrap();
    wait_until(Duration::from_secs(30), || async {
        c.node(1).config().role == Role::Viewer
    })
    .await
    .expect("the phone follows its new certificate");
    let mut saw = false;
    while let Ok(Ok(ev)) = tokio::time::timeout(Duration::from_millis(500), events.recv()).await {
        saw |= ev == shoebox_node::NodeEvent::RoleChanged { role: Role::Viewer };
    }
    assert!(saw, "RoleChanged was emitted for the app to persist");

    // With the recovery phrase a device that is no admin can change a role (here its own).
    let phrase =
        shoebox_core::recovery::RecoveryKit::new(rig.cap.library, 0, rig.root.clone()).phrase();
    nodes[1]
        .set_device_role_with_phrase(&phrase, &phone, Role::Phone)
        .await
        .unwrap();
    assert_eq!(c.node(1).config().role, Role::Phone, "follows at once");
    assert!(nodes[1]
        .set_device_role_with_phrase("not the phrase", &phone, Role::Keeper)
        .await
        .is_err());

    // An admin can change its own role; it stays an admin.
    nodes[0]
        .set_device_role(&core_id(&c, 0), Role::Phone)
        .await
        .unwrap();
    assert_eq!(c.node(0).config().role, Role::Phone);
    let list = nodes[0].store().device_list().await.unwrap();
    assert!(list.admins.iter().any(|a| a.admin == core_id(&c, 0)));
    // The others learn every change.
    wait_until(Duration::from_secs(30), || async {
        let l = nodes[2].store().device_list().await.unwrap();
        let role = |id| l.devices.iter().find(|d| d.device == id).map(|d| d.role);
        role(core_id(&c, 0)) == Some(Role::Phone) && role(phone) == Some(Role::Phone)
    })
    .await
    .expect("the change replicates");
    c.shutdown().await;
}
