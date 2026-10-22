//! M1-16 acceptance on the production store (Loro library): real nodes, a partition, offline
//! edits, heal, converge; and edits that flow without explicit sync calls.

mod common;

use common::*;
use shoebox_core::model::Role;
use shoebox_core::store::*;
use shoebox_testkit::{wait_until, Cluster};
use std::time::{Duration, Instant};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn three_nodes_partition_edit_heal_converge() {
    let started = Instant::now();
    let c = Cluster::new(3).await.unwrap();
    let (_rig, libs) = rig(&c, &[Role::Keeper, Role::Keeper, Role::Keeper]).await;
    let (l0, l1, l2) = (&libs[0], &libs[1], &libs[2]);
    let a: Vec<PeerAddr> = (0..3).map(|i| peer_addr(&c, i)).collect();

    for n in [1u8, 2, 3] {
        l0.upsert_photo(photo(n, n as i64 * 10)).await.unwrap();
    }
    l1.sync_with(&a[0]).await.unwrap();
    l2.sync_with(&a[0]).await.unwrap();
    assert_eq!(photos_of(l2).await.len(), 3);

    // Node 2 goes offline. Everyone keeps editing.
    c.take_offline(2);
    c.clock().advance(1_000);
    l0.set_favorite(&hash(1), true).await.unwrap();
    c.clock().advance(1_000);
    l1.mark_deleted(&hash(2)).await.unwrap();
    l1.upsert_photo(photo(4, 40)).await.unwrap();
    c.clock().advance(1_000);
    l2.set_favorite(&hash(3), true).await.unwrap();
    l2.set_favorite(&hash(1), false).await.unwrap(); // later than node 0's favorite: must win
    l2.upsert_photo(photo(5, 50)).await.unwrap();

    let before = Instant::now();
    assert!(
        l2.sync_with(&a[0]).await.is_err(),
        "offline node must not reach node 0"
    );
    assert!(
        before.elapsed() < Duration::from_secs(10),
        "failure is reported promptly"
    );
    l0.sync_with(&a[1]).await.unwrap();
    l1.sync_with(&a[0]).await.unwrap();
    assert!(l0.photo(&hash(4)).await.unwrap().is_some());
    assert!(l0.photo(&hash(2)).await.unwrap().unwrap().is_deleted());
    assert!(
        l0.photo(&hash(5)).await.unwrap().is_none(),
        "node 2's photo has not arrived yet"
    );

    c.bring_online(2);
    c.heal();
    for (x, y) in [(2, 0), (2, 1), (0, 1)] {
        libs[x].sync_with(&a[y]).await.unwrap();
        libs[y].sync_with(&a[x]).await.unwrap();
    }
    for l in [l0, l1, l2] {
        let ids: Vec<_> = photos_of(l).await.into_iter().map(|p| p.id).collect();
        assert_eq!(
            ids,
            vec![hash(5), hash(4), hash(3), hash(1)],
            "live timeline, newest first"
        );
        assert!(l.photo(&hash(2)).await.unwrap().unwrap().is_deleted());
        assert!(l.photo(&hash(3)).await.unwrap().unwrap().favorite.value);
        assert!(
            !l.photo(&hash(1)).await.unwrap().unwrap().favorite.value,
            "the later offline unfavorite wins"
        );
    }
    c.shutdown().await;
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "scenario took {:?}",
        started.elapsed()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn edits_flow_between_library_nodes_without_explicit_sync_calls() {
    let c = Cluster::new(3).await.unwrap();
    let (_rig, nodes) = library_nodes(&c, &[Role::Keeper, Role::Keeper, Role::Keeper]).await;
    nodes[0].store().upsert_photo(photo(9, 90)).await.unwrap();
    wait_until(Duration::from_secs(20), || async {
        nodes[1].store().photo(&hash(9)).await.unwrap().is_some()
            && nodes[2].store().photo(&hash(9)).await.unwrap().is_some()
    })
    .await
    .expect("the new photo reaches everyone on its own");

    // And from the middle of the line, both directions, repeatedly.
    nodes[2].store().set_favorite(&hash(9), true).await.unwrap();
    nodes[1].store().upsert_photo(photo(10, 100)).await.unwrap();
    wait_until(Duration::from_secs(20), || async {
        nodes[0].store().photo(&hash(10)).await.unwrap().is_some()
            && nodes[0]
                .store()
                .photo(&hash(9))
                .await
                .unwrap()
                .is_some_and(|p| p.favorite.value)
    })
    .await
    .expect("edits flow back");
    assert!(converged(&nodes).await);
    c.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn offline_edits_converge_after_reconnect() {
    let c = Cluster::new(3).await.unwrap();
    let (_rig, nodes) = library_nodes(&c, &[Role::Keeper, Role::Keeper, Role::Keeper]).await;
    nodes[0].store().upsert_photo(photo(1, 10)).await.unwrap();
    wait_until(Duration::from_secs(20), || async {
        nodes[2].store().photo(&hash(1)).await.unwrap().is_some()
    })
    .await
    .unwrap();
    c.take_offline(2);
    c.clock().advance(1_000);
    nodes[0].store().set_favorite(&hash(1), true).await.unwrap();
    nodes[1].store().upsert_photo(photo(3, 30)).await.unwrap();
    c.clock().advance(1_000);
    nodes[2].store().mark_deleted(&hash(1)).await.unwrap();
    nodes[2].store().upsert_photo(photo(4, 40)).await.unwrap();
    wait_until(Duration::from_secs(20), || async {
        nodes[1]
            .store()
            .photo(&hash(1))
            .await
            .unwrap()
            .is_some_and(|p| p.favorite.value)
    })
    .await
    .unwrap();
    assert!(nodes[0].store().photo(&hash(4)).await.unwrap().is_none());
    c.bring_online(2);
    wait_until(Duration::from_secs(30), || async {
        converged(&nodes).await && nodes[0].store().photo(&hash(4)).await.unwrap().is_some()
    })
    .await
    .expect("converges after reconnect without manual sync");
    for n in &nodes {
        assert!(n
            .store()
            .photo(&hash(1))
            .await
            .unwrap()
            .unwrap()
            .is_deleted());
        assert!(n.store().photo(&hash(3)).await.unwrap().is_some());
    }
    c.shutdown().await;
}
