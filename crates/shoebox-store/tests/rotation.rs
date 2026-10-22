//! M2-03b: revoke a device and rotate the library. Remaining members keep syncing on the new
//! epoch (also the one that was offline), the revoked device is left on the old one.

mod common;

use common::*;
use shoebox_core::model::Role;
use shoebox_core::store::*;
use shoebox_node::NodeEvent;
use shoebox_testkit::{wait_until, Cluster};
use std::time::Duration;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn revoking_a_device_moves_the_others_to_a_new_epoch() {
    let c = Cluster::new(4).await.unwrap();
    let (rig, nodes) = library_nodes_roles(&c, &[Role::Keeper; 4]).await;
    let _ = &rig;
    let old = nodes[0].library_id();
    for n in &nodes {
        n.enforce_membership().await.unwrap();
    }
    nodes[0].store().upsert_photo(photo(1, 10)).await.unwrap();
    wait_until(Duration::from_secs(20), || async {
        for n in &nodes {
            if n.store().photo(&hash(1)).await.unwrap().is_none() {
                return false;
            }
        }
        true
    })
    .await
    .expect("everyone starts in sync");

    // Node 2 is away while the admin revokes node 3.
    c.take_offline(2);
    let mut events = c.node(1).events();
    let victim = core_id(&c, 3);
    let notice = nodes[0].revoke_and_rotate(&victim).await.unwrap();
    assert_ne!(notice.new_cap.library, old);
    assert_eq!(nodes[0].library_id(), notice.new_cap.library);

    // Node 1 follows by itself and keeps syncing.
    wait_until(Duration::from_secs(20), || async {
        nodes[1].library_id() == notice.new_cap.library
    })
    .await
    .expect("a connected member follows the rotation");
    let mut rotated = false;
    while let Ok(Ok(ev)) = tokio::time::timeout(Duration::from_millis(300), events.recv()).await {
        rotated |= matches!(ev, NodeEvent::LibraryRotated { .. });
    }
    assert!(rotated, "LibraryRotated is announced");
    nodes[0].store().upsert_photo(photo(2, 20)).await.unwrap();
    wait_until(Duration::from_secs(20), || async {
        nodes[1].store().photo(&hash(2)).await.unwrap().is_some()
    })
    .await
    .expect("members sync on the new epoch");
    assert_eq!(photos_of(nodes[1].store().as_ref()).await.len(), 2);

    // The revoked device stays on the old epoch and receives nothing, not even through a sync it starts.
    assert_eq!(nodes[3].library_id(), old);
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(nodes[3].store().photo(&hash(2)).await.unwrap().is_none());
    assert!(nodes[3].sync_now(core_id(&c, 0)).await.is_err(), "refused");
    assert!(
        nodes[3].rotation_notice().is_none(),
        "the revoked device is never told where the library went"
    );

    // Node 2 comes back, hears about the move from whoever it meets first, and catches up.
    c.bring_online(2);
    wait_until(Duration::from_secs(30), || async {
        nodes[2].library_id() == notice.new_cap.library
    })
    .await
    .expect("a returning member learns of the rotation");
    wait_until(Duration::from_secs(30), || async {
        nodes[2].store().photo(&hash(2)).await.unwrap().is_some()
    })
    .await
    .expect("and syncs the new epoch");
    for n in &nodes[..3] {
        assert_eq!(photos_of(n.store().as_ref()).await.len(), 2);
        let list = n.store().device_list().await.unwrap();
        assert!(list.devices.iter().any(|d| d.device == victim && d.revoked));
    }
    c.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn only_admins_can_rotate_and_forged_notices_are_ignored() {
    let c = Cluster::new(3).await.unwrap();
    let (_rig, nodes) = library_nodes_roles(&c, &[Role::Keeper; 3]).await;
    let err = nodes[1]
        .revoke_and_rotate(&core_id(&c, 2))
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("admin"), "{err:#}");

    // A notice signed by a non-admin is refused by members.
    let real = nodes[0].revoke_and_rotate(&core_id(&c, 2)).await.unwrap();
    let mut forged = real.clone();
    forged.new_cap.library = shoebox_core::store::LibraryId([99; 16]);
    assert!(nodes[1].accept_notice_for_test(&forged).await.is_err());
    c.shutdown().await;
}
