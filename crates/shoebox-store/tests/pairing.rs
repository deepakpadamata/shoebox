//! M2-04: pairing over `shoebox/pair/0` between two real nodes (loopback, no relay).

mod common;

use common::*;
use shoebox_core::identity::KeyPair;
use shoebox_core::model::Role;
use shoebox_core::store::*;
use shoebox_node::{DecideFn, LibraryNode, Membership, NodeEvent, PairTicket};
use shoebox_store::{HybridBackend, Library};
use shoebox_testkit::{wait_until, Cluster};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

fn decider(ok: bool) -> (DecideFn, UnboundedReceiver<String>) {
    let (tx, rx) = unbounded_channel();
    let f: DecideFn = Arc::new(move |code| {
        let _ = tx.send(format!(
            "{}|{}|{:?}",
            code.sas, code.peer_name, code.peer_role
        ));
        Box::pin(async move { ok })
    });
    (f, rx)
}

/// Node 0 owns a library and is its admin; node 1 is a stranger.
async fn setup() -> (Cluster, LibraryNode<Library>) {
    let c = Cluster::new(2).await.unwrap();
    let root = KeyPair::generate();
    let backend = HybridBackend::for_node(c.node(0).clone(), c.clock().clone());
    let (lib, cap) = backend.create_library(&root.root_public()).await.unwrap();
    let ln =
        LibraryNode::with_sync_config(c.node(0).clone(), Arc::new(lib), c.clock().clone(), fast())
            .with_capability(cap);
    ln.create_identity(&root, Role::Keeper, "home")
        .await
        .unwrap();
    (c, ln)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pairing_adds_a_certified_device_and_hands_over_the_library() {
    let (c, a) = setup().await;
    let (da, mut seen_a) = decider(true);
    let (db, mut seen_b) = decider(true);
    let ticket = a
        .start_pairing(Role::Phone, Duration::from_secs(300), da)
        .await
        .unwrap();
    // The ticket survives its text form (what a QR code carries).
    let ticket: PairTicket = ticket.to_string().parse().unwrap();

    let mut events_a = c.node(0).events();
    let grant = c
        .node(1)
        .pair_join(&ticket, "Asha's phone", db)
        .await
        .unwrap();
    assert_eq!(grant.role, Role::Phone);
    assert_eq!(grant.cap.library, a.library_id());

    // Both people saw the same six digits and the right peer details.
    let (sa, sb) = (seen_a.recv().await.unwrap(), seen_b.recv().await.unwrap());
    let (sas_a, rest_a) = sa.split_once('|').unwrap();
    let (sas_b, _) = sb.split_once('|').unwrap();
    assert_eq!(sas_a, sas_b);
    assert_eq!(sas_a.len(), 6);
    assert!(sas_a.chars().all(|ch| ch.is_ascii_digit()));
    assert_eq!(rest_a, "Asha's phone|Phone");

    // The inviter recorded the certificate; the joiner can open the library and sync it.
    let list = a.store().device_list().await.unwrap();
    assert!(list
        .devices
        .iter()
        .any(|d| d.name == "Asha's phone" && d.role == Role::Phone && !d.revoked));
    let backend = HybridBackend::for_node(c.node(1).clone(), c.clock().clone());
    let lib = backend.open_library(&grant.cap).await.unwrap();
    let b =
        LibraryNode::with_sync_config(c.node(1).clone(), Arc::new(lib), c.clock().clone(), fast())
            .with_capability(grant.cap.clone());
    for peer in grant.peers {
        b.add_peer(peer).unwrap();
    }
    a.store().upsert_photo(photo(1, 10)).await.unwrap();
    wait_until(Duration::from_secs(20), || async {
        b.store().photo(&hash(1)).await.unwrap().is_some()
            && b.store().device_list().await.unwrap().devices.len() == 2
    })
    .await
    .expect("the new device syncs the library and its own certificate");
    b.enforce_membership().await.unwrap();
    assert_eq!(b.membership().await.unwrap(), Membership::Member);

    let mut done = false;
    while let Ok(Ok(ev)) = tokio::time::timeout(Duration::from_millis(500), events_a.recv()).await {
        done |= matches!(ev, NodeEvent::PairingDone { .. });
    }
    assert!(done, "PairingDone on the inviter");
    c.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_declined_code_aborts_without_any_state_change() {
    let (c, a) = setup().await;
    let before = a.store().device_list().await.unwrap();
    for (inviter_ok, joiner_ok) in [(true, false), (false, true)] {
        let (da, _) = decider(inviter_ok);
        let (db, _) = decider(joiner_ok);
        let ticket = a
            .start_pairing(Role::Phone, Duration::from_secs(300), da)
            .await
            .unwrap();
        let err = c.node(1).pair_join(&ticket, "x", db).await.unwrap_err();
        assert!(
            format!("{err:#}").to_lowercase().contains("declined"),
            "{err:#}"
        );
        assert_eq!(
            a.store().device_list().await.unwrap(),
            before,
            "no certificate was issued"
        );
    }
    c.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tickets_are_single_use_and_expire() {
    let (c, a) = setup().await;
    let (da, _) = decider(true);
    let (db, _) = decider(true);
    let ticket = a
        .start_pairing(Role::Phone, Duration::from_secs(300), da.clone())
        .await
        .unwrap();
    c.node(1)
        .pair_join(&ticket, "one", db.clone())
        .await
        .unwrap();
    let err = c
        .node(1)
        .pair_join(&ticket, "two", db.clone())
        .await
        .unwrap_err();
    assert!(
        format!("{err:#}").contains("expired or already used"),
        "{err:#}"
    );

    let ticket = a
        .start_pairing(Role::Phone, Duration::from_secs(300), da)
        .await
        .unwrap();
    c.clock().advance(301_000);
    let err = c.node(1).pair_join(&ticket, "late", db).await.unwrap_err();
    assert!(
        format!("{err:#}").contains("expired or already used"),
        "{err:#}"
    );
    c.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn failed_attempts_are_rate_limited() {
    let (c, a) = setup().await;
    let (da, _) = decider(true);
    let (db, _) = decider(true);
    let ticket = a
        .start_pairing(Role::Phone, Duration::from_secs(300), da)
        .await
        .unwrap();
    // Someone who can reach the device but does not know a valid secret gets nowhere...
    for i in 0..30u8 {
        let mut wrong = ticket.clone();
        wrong.secret[0] ^= i + 1;
        let err = c
            .node(1)
            .pair_join(&wrong, "x", db.clone())
            .await
            .unwrap_err();
        assert!(
            format!("{err:#}").contains("expired or already used"),
            "{err:#}"
        );
    }
    // ...and after too many failures even the real ticket is refused for a while.
    let err = c
        .node(1)
        .pair_join(&ticket, "x", db.clone())
        .await
        .unwrap_err();
    assert!(
        format!("{err:#}").contains("too many failed attempts"),
        "{err:#}"
    );
    // A minute later pairing works again.
    c.clock().advance(61_000);
    let ticket = a
        .start_pairing(Role::Phone, Duration::from_secs(300), decider(true).0)
        .await
        .unwrap();
    c.node(1).pair_join(&ticket, "x", db).await.unwrap();
    c.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn only_admins_can_start_pairing() {
    let c = Cluster::new(1).await.unwrap();
    let backend = HybridBackend::for_node(c.node(0).clone(), c.clock().clone());
    let (lib, cap) = backend
        .create_library(&KeyPair::generate().root_public())
        .await
        .unwrap();
    let ln =
        LibraryNode::with_sync_config(c.node(0).clone(), Arc::new(lib), c.clock().clone(), fast())
            .with_capability(cap);
    let (d, _) = decider(true);
    assert!(ln
        .start_pairing(Role::Phone, Duration::from_secs(60), d)
        .await
        .is_err());
    c.shutdown().await;
}
