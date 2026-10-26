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

/// A-085: a stolen admin device. Another admin cannot remove it on its own (that would let a thief
/// remove the real admin too); the recovery phrase can. Devices the thief had certified stay, and a
/// member that already followed the thief's own rotation switches to the real one.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stolen_admin_is_removed_with_the_recovery_phrase() {
    use shoebox_core::clock::Clock;
    use shoebox_core::identity::{issue_admin_cert, issue_device_cert};
    use shoebox_core::Hlc;
    // 0: the real admin, 1: the stolen admin phone, 2: a member that is away, 3: a device the
    // stolen phone certified (its current cert is signed by 1).
    let c = Cluster::new(4).await.unwrap();
    let (rig, nodes) =
        library_nodes_roles(&c, &[Role::Keeper, Role::Phone, Role::Phone, Role::Keeper]).await;
    let (real, thief, away, added) = (
        core_id(&c, 0),
        core_id(&c, 1),
        core_id(&c, 2),
        core_id(&c, 3),
    );
    // After the rig's certs (now - 99), before anything the nodes sign from now on.
    let at = |n: u32, d| Hlc {
        physical_ms: c.clock().now_ms() - 50,
        counter: n,
        device: d,
    };
    nodes[0]
        .store()
        .apply_device_change(SignedDeviceChange::Admin(issue_admin_cert(
            &rig.root,
            rig.cap.library,
            thief,
            at(1, real),
            None,
        )))
        .await
        .unwrap();
    wait_until(Duration::from_secs(30), || async {
        let l = nodes[1].store().device_list().await.unwrap();
        l.admins.iter().any(|a| a.admin == thief)
    })
    .await
    .expect("the phone learns it is an admin");
    nodes[1]
        .store()
        .apply_device_change(SignedDeviceChange::Device(issue_device_cert(
            &c.node(1).device_key(),
            rig.cap.library,
            added,
            Role::Keeper,
            "added by the phone",
            at(2, thief),
            false,
        )))
        .await
        .unwrap();
    for n in &nodes {
        n.enforce_membership().await.unwrap();
    }
    wait_until(Duration::from_secs(30), || async {
        for n in &nodes {
            let l = n.store().device_list().await.unwrap();
            if !l.admins.iter().any(|a| a.admin == thief)
                || !l
                    .devices
                    .iter()
                    .any(|d| d.device == added && d.added_by == thief)
            {
                return false;
            }
        }
        true
    })
    .await
    .expect("everyone knows both admins and who certified device 3");

    // Without the phrase an admin cannot remove another admin.
    let err = nodes[0].revoke_and_rotate(&thief).await.unwrap_err();
    assert!(format!("{err:#}").contains("recovery phrase"), "{err:#}");

    // The member is away, and the phone hears nothing, while the real admin removes the phone
    // with the phrase.
    c.partition(0, 2);
    c.partition(1, 2);
    c.partition(0, 1);
    c.partition(1, 3);
    let real_notice = nodes[0]
        .revoke_and_rotate_with_phrase(&rig_phrase(&rig), &thief)
        .await
        .unwrap();
    let list = nodes[0].store().device_list().await.unwrap();
    assert!(list.devices.iter().any(|d| d.device == thief && d.revoked));
    assert!(
        list.devices
            .iter()
            .any(|d| d.device == added && !d.revoked && d.added_by.0 == rig.cap.root.0),
        "the device the phone certified stays, re-signed by the root"
    );
    assert!(list.admins.iter().all(|a| a.admin != thief));

    // The thief, which never heard, removes device 3 and moves the member to its own epoch.
    let thief_notice = nodes[1].revoke_and_rotate(&added).await.unwrap();
    assert!(nodes[2]
        .accept_notice_for_test(&thief_notice)
        .await
        .unwrap());
    assert_eq!(nodes[2].library_id(), thief_notice.new_cap.library);

    // Back in touch, the real notice wins: it removes, with the root key, the admin it followed.
    c.heal();
    assert!(nodes[2].accept_notice_for_test(&real_notice).await.unwrap());
    assert_eq!(nodes[2].library_id(), real_notice.new_cap.library);
    let list = nodes[2].store().device_list().await.unwrap();
    assert!(list.devices.iter().any(|d| d.device == thief && d.revoked));
    assert!(list.devices.iter().any(|d| d.device == away && !d.revoked));
    // The thief's epoch only knew the thief's (now void) removal of device 3, so device 3 is back
    // once the member has the root's re-signed cert from the real admin.
    wait_until(Duration::from_secs(30), || async {
        let l = nodes[2].store().device_list().await.unwrap();
        l.devices.iter().any(|d| d.device == added && !d.revoked)
    })
    .await
    .expect("the thief's removal of device 3 does not count");
    // And the thief's notice is refused from now on.
    assert!(nodes[2]
        .accept_notice_for_test(&thief_notice)
        .await
        .is_err());
    c.shutdown().await;
}

fn rig_phrase(rig: &Rig) -> String {
    shoebox_core::recovery::RecoveryKit::new(rig.cap.library, 0, rig.root.clone()).phrase()
}
