//! M2-03b: revoke a device and rotate the library. Remaining members keep syncing on the new
//! epoch (also the one that was offline), the revoked device is left on the old one.

mod common;

use common::*;
use shoebox_core::clock::Clock;
use shoebox_core::model::Role;
use shoebox_core::store::*;
use shoebox_node::NodeEvent;
use shoebox_testkit::{wait_until, Cluster};
use std::sync::Arc;
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
    // Both photos: it follows the move carrying only its own records, and photo 1 (written by
    // node 0) comes back by syncing, a moment after the new one.
    wait_until(Duration::from_secs(30), || async {
        photos_of(nodes[2].store().as_ref()).await.len() == 2
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
    use shoebox_core::identity::{issue_admin_cert, issue_device_cert};
    use shoebox_core::Hlc;
    // 0: the real admin, 1: the stolen admin phone, 2: a member that is away, 3: a device the
    // stolen phone certified (its current cert is signed by 1).
    let c = Cluster::new(4).await.unwrap();
    let (rig, nodes) = library_nodes_roles(
        &c,
        &[Role::Keeper, Role::Companion, Role::Companion, Role::Keeper],
    )
    .await;
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
    c.partition(2, 3); // or device 3 passes the real notice on to the member before the thief's
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

/// Root rotation (the recovery phrase may have leaked): a device that is no admin, with the
/// current phrase, hands the library over to a new root. Everyone who is a member now follows,
/// also one that was away; the photos are still there; the old phrase opens nothing any more and
/// the new one does.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_root_key_can_be_replaced_and_the_old_phrase_stops_working() {
    let c = Cluster::new(3).await.unwrap();
    let (rig, nodes) =
        library_nodes_roles(&c, &[Role::Keeper, Role::Companion, Role::Companion]).await;
    for n in &nodes {
        n.enforce_membership().await.unwrap();
    }
    nodes[0].store().upsert_photo(photo(1, 10)).await.unwrap();
    wait_until(Duration::from_secs(20), || async {
        nodes[2].store().photo(&hash(1)).await.unwrap().is_some()
    })
    .await
    .expect("in sync");
    let old_phrase = rig_phrase(&rig);
    let old_root = rig.cap.root;

    c.take_offline(2);
    let replaced = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut events = c.node(0).events();
    let flag = replaced.clone();
    tokio::spawn(async move {
        while let Ok(ev) = events.recv().await {
            if ev == shoebox_node::NodeEvent::RecoveryPhraseReplaced {
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
    });
    let kit = nodes[1].rotate_root(&old_phrase).await.unwrap();
    assert_ne!(kit.phrase(), old_phrase);
    let new_root = kit.root().root_public();
    assert_ne!(new_root, old_root);
    assert!(
        !nodes[1].recovery_kit_saved().await.unwrap(),
        "the new kit still has to be saved"
    );

    // Node 0 follows at once; everything there is certified by the new root.
    wait_until(Duration::from_secs(20), || async {
        nodes[0].library_id() == nodes[1].library_id()
    })
    .await
    .expect("node 0 follows the handover");
    // The event follows the move by a moment (the store changes epoch first).
    wait_until(Duration::from_secs(5), || async {
        replaced.load(std::sync::atomic::Ordering::SeqCst)
    })
    .await
    .expect("a following device is told the phrase was replaced");
    wait_until(Duration::from_secs(20), || async {
        let l = nodes[0].store().device_list().await.unwrap();
        l.devices.len() == 3
            && l.devices
                .iter()
                .all(|d| d.added_by.0 == new_root.0 && !d.revoked)
    })
    .await
    .expect("every member re-certified by the new root");
    let admins = nodes[0].store().device_list().await.unwrap().admins;
    assert!(admins.iter().any(|a| a.admin == core_id(&c, 0)));
    assert!(
        admins.iter().any(|a| a.admin == core_id(&c, 1)),
        "the device that held the phrase became an admin"
    );
    assert!(nodes[0].store().photo(&hash(1)).await.unwrap().is_some());

    // The device that was away catches up when it is back.
    c.bring_online(2);
    wait_until(Duration::from_secs(30), || async {
        nodes[2].library_id() == nodes[1].library_id()
            && nodes[2].store().photo(&hash(1)).await.unwrap().is_some()
    })
    .await
    .expect("the away device follows and keeps the photos");

    // The old phrase opens nothing: no recovery, no removal. The new one works.
    let o = Cluster::new(1).await.unwrap();
    o.node(0).add_peer_addr(c.node(0).addr());
    c.node(0).add_peer_addr(o.node(0).addr());
    let link = nodes[0].peer_link();
    let err = o
        .node(0)
        .recover(&link, &old_phrase, Role::Keeper, "x", c.clock().now_ms())
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("rejected"), "{err:#}");
    assert!(nodes[0]
        .revoke_and_rotate_with_phrase(&old_phrase, &core_id(&c, 2))
        .await
        .is_err());
    o.node(0)
        .recover(&link, &kit.phrase(), Role::Keeper, "y", c.clock().now_ms())
        .await
        .unwrap();
    o.shutdown().await;
    c.shutdown().await;
}

/// Whoever else has the old phrase can hand over too; each member follows the first valid
/// handover it sees and refuses the other.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_first_valid_handover_wins() {
    let c = Cluster::new(3).await.unwrap();
    let (rig, nodes) =
        library_nodes_roles(&c, &[Role::Keeper, Role::Companion, Role::Companion]).await;
    for n in &nodes {
        n.enforce_membership().await.unwrap();
    }
    let phrase = rig_phrase(&rig);
    // Two holders of the phrase act at once, each unaware of the other; node 2 hears neither yet.
    c.partition(0, 1);
    c.partition(0, 2);
    c.partition(1, 2);
    nodes[0].rotate_root(&phrase).await.unwrap();
    nodes[1].rotate_root(&phrase).await.unwrap();
    let ours = nodes[0].epoch_notice().unwrap();
    let theirs = nodes[1].epoch_notice().unwrap();
    assert_ne!(ours.new_cap().library, theirs.new_cap().library);

    assert!(nodes[2].accept_epoch_notice_for_test(&ours).await.unwrap());
    let err = nodes[2]
        .accept_epoch_notice_for_test(&theirs)
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("another epoch"), "{err:#}");
    assert_eq!(nodes[2].library_id(), ours.new_cap().library);
    c.shutdown().await;
}

/// The same notice often reaches a member from two devices at once (one pushes it, another hands
/// on the one it keeps). Exactly one of them moves the member; the store is moved once, its data
/// intact (R1-06 found two concurrent moves deleting the new epoch's own directory).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_same_notice_twice_at_once_moves_a_member_once() {
    let c = Cluster::new(3).await.unwrap();
    let (_rig, nodes) = library_nodes_roles(&c, &[Role::Keeper; 3]).await;
    for n in &nodes {
        n.enforce_membership().await.unwrap();
    }
    nodes[0].store().upsert_photo(photo(1, 10)).await.unwrap();
    wait_until(Duration::from_secs(20), || async {
        nodes[1].store().photo(&hash(1)).await.unwrap().is_some()
    })
    .await
    .expect("in sync");
    c.partition(0, 1); // node 1 hears of the move only from the test below
    c.partition(1, 2);
    let notice = nodes[0].revoke_and_rotate(&core_id(&c, 2)).await.unwrap();

    // Two tasks, so the two deliveries really run in parallel.
    let deliver = || {
        let (n, notice) = (nodes[1].clone(), notice.clone());
        tokio::spawn(async move { n.accept_notice_for_test(&notice).await })
    };
    let (a, b) = (deliver(), deliver());
    let moved = [a.await.unwrap().unwrap(), b.await.unwrap().unwrap()];
    assert_eq!(moved.iter().filter(|m| **m).count(), 1, "{moved:?}");
    assert_eq!(nodes[1].library_id(), notice.new_cap.library);
    c.heal();
    nodes[0].store().upsert_photo(photo(2, 20)).await.unwrap();
    wait_until(Duration::from_secs(30), || async {
        photos_of(nodes[1].store().as_ref()).await.len() == 2
    })
    .await
    .expect("the member keeps syncing on the new epoch, with every photo");
    c.shutdown().await;
}
