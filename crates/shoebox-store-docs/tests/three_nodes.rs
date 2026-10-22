//! M1-02 / M1-05 acceptance: three real nodes, a partition, offline edits, heal, converge.

mod common;

use common::*;
use shoebox_core::store::*;
use shoebox_store_docs::{from_endpoint_addr, DocsBackend, DocsLibrary};
use shoebox_testkit::{assert_converged, wait_until, Cluster};
use std::time::{Duration, Instant};

fn addr_of(c: &Cluster, i: usize) -> PeerAddr {
    from_endpoint_addr(&c.node(i).addr())
}

async fn sync_pair(a: &DocsLibrary, b: &DocsLibrary, b_addr: &PeerAddr, a_addr: &PeerAddr) {
    a.sync_with(b_addr).await.expect("a syncs with b");
    b.sync_with(a_addr).await.expect("b syncs with a");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn three_nodes_partition_edit_heal_converge() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
    let started = Instant::now();
    let c = Cluster::new(3).await.unwrap();
    let backends: Vec<DocsBackend> = (0..3)
        .map(|i| DocsBackend::for_node(c.node(i).clone(), c.clock().clone()))
        .collect();
    let (l0, cap) = backends[0]
        .create_library(&RootPublicKey([7; 32]))
        .await
        .unwrap();
    let l1 = backends[1].open_library(&cap).await.unwrap();
    let l2 = backends[2].open_library(&cap).await.unwrap();
    let addrs: Vec<PeerAddr> = (0..3).map(|i| addr_of(&c, i)).collect();

    // Start: node 0 imports three photos and everyone catches up.
    for n in [1u8, 2, 3] {
        l0.upsert_photo(photo(n, n as i64 * 10)).await.unwrap();
    }
    l1.sync_with(&addrs[0]).await.unwrap();
    l2.sync_with(&addrs[0]).await.unwrap();
    assert_converged(&[&l0, &l1, &l2]).await;

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

    // The partitioned node cannot reach anyone (and finds out quickly); the others still sync.
    let before = Instant::now();
    assert!(
        l2.sync_with(&addrs[0]).await.is_err(),
        "offline node must not reach node 0"
    );
    assert!(
        before.elapsed() < Duration::from_secs(10),
        "failure is reported promptly"
    );
    sync_pair(&l0, &l1, &addrs[1], &addrs[0]).await;
    assert!(l0.photo(&hash(4)).await.unwrap().is_some());
    assert!(l0.photo(&hash(2)).await.unwrap().unwrap().is_deleted());
    assert!(
        l0.photo(&hash(5)).await.unwrap().is_none(),
        "node 2's photo has not arrived yet"
    );

    // Heal: node 2 rejoins and everything converges, whoever syncs with whom.
    c.bring_online(2);
    c.heal();
    sync_pair(&l2, &l0, &addrs[0], &addrs[2]).await;
    sync_pair(&l2, &l1, &addrs[1], &addrs[2]).await;
    sync_pair(&l0, &l1, &addrs[1], &addrs[0]).await;
    assert_converged(&[&l0, &l1, &l2]).await;

    for l in [&l0, &l1, &l2] {
        let ids: Vec<_> = l
            .photos_by_time(TimeRange::ALL, Page::all())
            .await
            .unwrap()
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(
            ids,
            vec![hash(5), hash(4), hash(3), hash(1)],
            "live timeline, newest first"
        );
        assert!(
            l.photo(&hash(2)).await.unwrap().unwrap().is_deleted(),
            "delete reached everyone"
        );
        assert!(l.photo(&hash(3)).await.unwrap().unwrap().favorite.value);
        assert!(
            !l.photo(&hash(1)).await.unwrap().unwrap().favorite.value,
            "the later (offline) unfavorite beats the earlier favorite"
        );
    }
    c.shutdown().await;
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "scenario took {:?}",
        started.elapsed()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn gossip_converges_without_explicit_sync_calls() {
    // After one explicit sync joins the swarm, later edits flow on their own.
    let c = Cluster::new(2).await.unwrap();
    let b0 = DocsBackend::for_node(c.node(0).clone(), c.clock().clone());
    let b1 = DocsBackend::for_node(c.node(1).clone(), c.clock().clone());
    let (l0, cap) = b0.create_library(&RootPublicKey([7; 32])).await.unwrap();
    let l1 = b1.open_library(&cap).await.unwrap();
    l1.sync_with(&from_endpoint_addr(&c.node(0).addr()))
        .await
        .unwrap();

    l0.upsert_photo(photo(9, 90)).await.unwrap();
    wait_until(Duration::from_secs(20), || async {
        l1.photo(&hash(9)).await.unwrap().is_some()
    })
    .await
    .expect("the new photo reaches node 1 on its own");
    c.shutdown().await;
}
