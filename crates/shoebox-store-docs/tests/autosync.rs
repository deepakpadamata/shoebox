//! M1-08: libraries converge on their own (connect-triggered and periodic sync, with backoff),
//! including edits made while a node was offline.

mod common;

use common::*;
use shoebox_core::store::*;
use shoebox_node::{LibraryNode, NodeEvent, SyncPhase};
use shoebox_store_docs::{from_endpoint_addr, DocsLibrary};
use shoebox_testkit::{assert_converged, wait_until, Cluster};
use std::time::Duration;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn joined_nodes_converge_without_any_manual_sync() {
    let c = Cluster::new(3).await.unwrap();
    let nodes = library_nodes(&c).await;
    for n in [1u8, 2, 3] {
        nodes[0]
            .store()
            .upsert_photo(photo(n, n as i64))
            .await
            .unwrap();
    }
    // Entries arrive in any order, so wait for *all* of them, not just the last one written.
    wait_until(Duration::from_secs(20), || async {
        let all = |n: &LibraryNode<DocsLibrary>| {
            let store = n.store().clone();
            async move {
                store
                    .photos_by_time(TimeRange::ALL, Page::all())
                    .await
                    .unwrap()
                    .len()
                    == 3
            }
        };
        all(&nodes[1]).await && all(&nodes[2]).await
    })
    .await
    .expect("joiners receive the library on their own");
    assert!(converged(&nodes).await);
    c.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn offline_edits_converge_after_reconnect_and_progress_is_reported() {
    let c = Cluster::new(3).await.unwrap();
    let nodes = library_nodes(&c).await;
    nodes[0].store().upsert_photo(photo(1, 10)).await.unwrap();
    nodes[0].store().upsert_photo(photo(2, 20)).await.unwrap();
    wait_until(Duration::from_secs(20), || async {
        converged(&nodes).await && nodes[2].store().photo(&hash(2)).await.unwrap().is_some()
    })
    .await
    .unwrap();

    // Node 2 loses the network and everyone keeps editing.
    let mut events = c.node(2).events();
    c.take_offline(2);
    c.clock().advance(1_000);
    nodes[0].store().set_favorite(&hash(1), true).await.unwrap();
    nodes[1].store().upsert_photo(photo(3, 30)).await.unwrap();
    c.clock().advance(1_000);
    nodes[2].store().mark_deleted(&hash(2)).await.unwrap();
    nodes[2].store().upsert_photo(photo(4, 40)).await.unwrap();

    // The connected pair stays in sync; the offline node falls behind and reports why.
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
    let mut saw_failure = false;
    while let Ok(Ok(ev)) = tokio::time::timeout(Duration::from_secs(10), events.recv()).await {
        if matches!(
            ev,
            NodeEvent::SyncProgress {
                phase: SyncPhase::Failed(_),
                ..
            }
        ) {
            saw_failure = true;
            break;
        }
    }
    assert!(saw_failure, "the offline node reports failed sync attempts");

    // Reconnect: no manual sync, the manager retries and everything converges.
    c.bring_online(2);
    wait_until(Duration::from_secs(30), || async {
        converged(&nodes).await && nodes[0].store().photo(&hash(4)).await.unwrap().is_some()
    })
    .await
    .expect("converges after reconnect");
    let stores: Vec<&DocsLibrary> = nodes.iter().map(|n| n.store().as_ref()).collect();
    assert_converged(&stores).await;
    for s in &stores {
        assert!(s.photo(&hash(2)).await.unwrap().unwrap().is_deleted());
        assert!(s.photo(&hash(1)).await.unwrap().unwrap().favorite.value);
        assert!(s.photo(&hash(3)).await.unwrap().is_some());
    }
    // Success is reported too.
    let mut saw_finished = false;
    let mut events = c.node(2).events();
    let _ = events.recv().await;
    nodes[2]
        .sync_now(from_endpoint_addr(&c.node(0).addr()).id)
        .await
        .unwrap();
    while let Ok(Ok(ev)) = tokio::time::timeout(Duration::from_millis(500), events.recv()).await {
        saw_finished |= matches!(
            ev,
            NodeEvent::SyncProgress {
                phase: SyncPhase::Finished,
                ..
            }
        );
    }
    assert!(saw_finished);
    c.shutdown().await;
}

/// Acceptance check (opt-in, slow): 10k records converge between two nodes in under a minute.
/// Local loopback only — it says nothing about relay or WAN behaviour.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "slow: 10k records"]
async fn ten_thousand_records_converge_in_under_a_minute() {
    let c = Cluster::new(2).await.unwrap();
    let nodes = library_nodes(&c).await;
    let started = std::time::Instant::now();
    let n: u32 = std::env::var("RECORDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10_000);
    for i in 0..n {
        let mut p = photo((i % 250) as u8, i as i64);
        p.id = shoebox_core::Blake3Hash::of_bytes(&i.to_le_bytes());
        p.original.hash = p.id;
        nodes[0].store().upsert_photo(p).await.unwrap();
    }
    eprintln!("wrote {n} records in {:?}", started.elapsed());
    let t = std::time::Instant::now();
    let mut last = 0;
    loop {
        let have = nodes[1]
            .store()
            .at_risk(255, Page::all())
            .await
            .unwrap()
            .len();
        if have != last {
            eprintln!("  {:>6.1?}  node 1 has {have}/{n}", t.elapsed());
            last = have;
        }
        if have == n as usize {
            break;
        }
        assert!(
            t.elapsed() < Duration::from_secs(60),
            "only {have}/{n} records after 60 s"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    eprintln!("{n} records converged in {:?}", t.elapsed());
    c.shutdown().await;
}

/// Regression: a burst of remote entries once froze the whole docs engine, because the event
/// subscriber stopped draining while a rescan (which needs the engine) ran. ~1.5k records is
/// enough to overflow the engine's bounded event channels.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_burst_of_records_does_not_stall_the_engine() {
    let c = Cluster::new(2).await.unwrap();
    let nodes = library_nodes(&c).await;
    for i in 0..1_500u32 {
        let mut p = photo((i % 250) as u8, i as i64);
        p.id = shoebox_core::Blake3Hash::of_bytes(&i.to_le_bytes());
        p.original.hash = p.id;
        nodes[0].store().upsert_photo(p).await.unwrap();
    }
    wait_until(Duration::from_secs(30), || async {
        tokio::time::timeout(
            Duration::from_secs(5),
            nodes[1].store().at_risk(255, Page::all()),
        )
        .await
        .map(|r| r.unwrap().len() == 1_500)
        .unwrap_or(false)
    })
    .await
    .expect("all 1500 records arrive and the store stays responsive");
    c.shutdown().await;
}

/// `sync_now` must not return while a sync with that peer is still running (the background
/// manager starts one as soon as a peer is added): callers rely on "returned" meaning "synced".
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sync_now_waits_for_a_sync_already_in_flight() {
    use shoebox_core::store::{RootPublicKey, StoreBackend};
    let c = Cluster::new(2).await.unwrap();
    let b0 = shoebox_store_docs::DocsBackend::for_node(c.node(0).clone(), c.clock().clone());
    let (l0, cap) = b0.create_library(&RootPublicKey([7; 32])).await.unwrap();
    for i in 0..300u32 {
        let mut p = photo((i % 250) as u8, i as i64);
        p.id = shoebox_core::Blake3Hash::of_bytes(&i.to_le_bytes());
        p.original.hash = p.id;
        l0.upsert_photo(p).await.unwrap();
    }
    let b1 = shoebox_store_docs::DocsBackend::for_node(c.node(1).clone(), c.clock().clone());
    let l1 = b1.open_library(&cap).await.unwrap();
    let ln1 = shoebox_node::LibraryNode::with_sync_config(
        c.node(1).clone(),
        std::sync::Arc::new(l1),
        c.clock().clone(),
        fast(),
    );
    let peer = from_endpoint_addr(&c.node(0).addr());
    ln1.add_peer(peer.clone()).unwrap(); // the manager starts syncing right away...
    ln1.sync_now(peer.id).await.unwrap(); // ...and this must not return until it is done
    let have = ln1
        .store()
        .photos_by_time(TimeRange::ALL, Page::all())
        .await
        .unwrap()
        .len();
    assert_eq!(
        have, 300,
        "sync_now returned before the library had arrived"
    );
    c.shutdown().await;
}
