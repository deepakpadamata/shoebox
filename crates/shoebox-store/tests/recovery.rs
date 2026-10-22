//! A-060: getting back into a library with only the recovery phrase and one surviving device.

mod common;

use common::*;
use shoebox_core::clock::Clock;
use shoebox_core::identity::KeyPair;
use shoebox_core::model::Role;
use shoebox_core::store::*;
use shoebox_node::{LibraryNode, Membership, PeerLink};
use shoebox_store::HybridBackend;
use shoebox_testkit::{wait_until, Cluster};
use std::sync::Arc;
use std::time::Duration;

/// Node 0 owns the library and is its admin; the test keeps the printed phrase.
async fn setup() -> (Cluster, LibraryNode<shoebox_store::Library>, String) {
    let c = Cluster::new(2).await.unwrap();
    let root = KeyPair::generate();
    let backend = HybridBackend::for_node(c.node(0).clone(), c.clock().clone());
    let (lib, cap) = backend.create_library(&root.root_public()).await.unwrap();
    let ln =
        LibraryNode::with_sync_config(c.node(0).clone(), Arc::new(lib), c.clock().clone(), fast())
            .with_capability(cap);
    let kit = ln
        .create_identity(&root, Role::Keeper, "home")
        .await
        .unwrap();
    for n in 1..=3u32 {
        ln.store()
            .upsert_photo(photo(n as u8, n as i64))
            .await
            .unwrap();
    }
    (c, ln, kit.phrase())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_fresh_device_recovers_the_library_with_the_phrase_alone() {
    let (c, a, phrase) = setup().await;
    let link: PeerLink = a.peer_link().to_string().parse().unwrap();
    let grant = c
        .node(1)
        .recover(
            &link,
            &phrase,
            Role::Keeper,
            "replacement",
            c.clock().now_ms(),
        )
        .await
        .unwrap();
    assert_eq!(grant.cap.library, a.library_id());
    // The surviving device lists the newcomer as an admin right away.
    let list = a.store().device_list().await.unwrap();
    assert!(list
        .devices
        .iter()
        .any(|d| d.name == "replacement" && !d.revoked));
    assert_eq!(list.admins.len(), 2);

    // The newcomer opens the library, syncs and sees every photo.
    let backend = HybridBackend::for_node(c.node(1).clone(), c.clock().clone());
    let lib = backend.open_library(&grant.cap).await.unwrap();
    let b =
        LibraryNode::with_sync_config(c.node(1).clone(), Arc::new(lib), c.clock().clone(), fast())
            .with_capability(grant.cap.clone());
    for p in grant.peers {
        b.add_peer(p).unwrap();
    }
    wait_until(Duration::from_secs(20), || async {
        photos_of(b.store().as_ref()).await.len() == 3
    })
    .await
    .expect("the recovered device syncs the whole library");
    b.enforce_membership().await.unwrap();
    assert_eq!(b.membership().await.unwrap(), Membership::Member);
    // And it can pair further devices: it is an admin.
    assert!(b
        .start_pairing(
            Role::Phone,
            Duration::from_secs(60),
            Arc::new(|_| Box::pin(async { true }))
        )
        .await
        .is_ok());
    c.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_wrong_phrase_changes_nothing_and_is_rate_limited() {
    let (c, a, _phrase) = setup().await;
    let link = a.peer_link();
    let before = a.store().device_list().await.unwrap();
    let stranger =
        shoebox_core::recovery::RecoveryKit::new(a.library_id(), 0, KeyPair::generate()).phrase();
    let err = c
        .node(1)
        .recover(&link, &stranger, Role::Keeper, "x", 1)
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("proof rejected"), "{err:#}");
    let err = c
        .node(1)
        .recover(&link, "not a phrase", Role::Keeper, "x", 1)
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("24-word"), "{err:#}");
    assert_eq!(
        a.store().device_list().await.unwrap(),
        before,
        "no state change"
    );

    for _ in 0..12 {
        let _ = c
            .node(1)
            .recover(&link, &stranger, Role::Keeper, "x", 1)
            .await;
    }
    // After too many failures even a correct attempt waits.
    let err = c
        .node(1)
        .recover(&link, &stranger, Role::Keeper, "x", 1)
        .await
        .unwrap_err();
    assert!(
        format!("{err:#}").contains("too many failed attempts"),
        "{err:#}"
    );
    c.shutdown().await;
}

#[test]
fn device_links_round_trip_and_reject_damage() {
    let link = PeerLink(PeerAddr {
        id: shoebox_core::EndpointId([4; 32]),
        relay_url: None,
        direct_addrs: vec!["10.0.0.2:1234".into()],
    });
    let s = link.to_string();
    assert_eq!(s.parse::<PeerLink>().unwrap(), link);
    let mut damaged = s.clone();
    damaged.replace_range(
        s.len() - 8..s.len() - 7,
        if &s[s.len() - 8..s.len() - 7] == "0" {
            "1"
        } else {
            "0"
        },
    );
    assert!(damaged.parse::<PeerLink>().is_err());
    assert!("shoebox-pair1abcd".parse::<PeerLink>().is_err());
}
