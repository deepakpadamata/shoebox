use shoebox_testkit::{wait_until, Cluster};
use std::time::{Duration, Instant};

async fn reachable(c: &Cluster, from: usize, to: usize) -> bool {
    tokio::time::timeout(Duration::from_secs(10), c.node(from).ping(c.node(to).id()))
        .await
        .map(|r| r.is_ok())
        .unwrap_or(false)
}

#[tokio::test]
async fn nodes_start_knowing_each_other() {
    let c = Cluster::new(3).await.unwrap();
    assert_eq!(c.len(), 3);
    for from in 0..3 {
        for to in 0..3 {
            if from != to {
                assert!(reachable(&c, from, to).await, "{from} -> {to}");
            }
        }
    }
    c.shutdown().await;
}

#[tokio::test]
async fn partition_cuts_one_pair_and_heal_restores_it() {
    let c = Cluster::new(3).await.unwrap();
    c.partition(0, 1);
    assert!(!reachable(&c, 0, 1).await);
    assert!(!reachable(&c, 1, 0).await);
    assert!(reachable(&c, 0, 2).await, "other pairs are unaffected");
    assert!(reachable(&c, 2, 1).await);
    c.heal();
    assert!(reachable(&c, 0, 1).await);
    assert!(reachable(&c, 1, 0).await);
    c.shutdown().await;
}

#[tokio::test]
async fn offline_node_is_cut_from_everyone_until_brought_back() {
    let c = Cluster::new(3).await.unwrap();
    c.take_offline(2);
    assert!(!reachable(&c, 0, 2).await);
    assert!(!reachable(&c, 2, 1).await);
    assert!(reachable(&c, 0, 1).await);
    c.bring_online(2);
    assert!(reachable(&c, 0, 2).await);
    assert!(reachable(&c, 2, 1).await);
    c.shutdown().await;
}

#[tokio::test]
async fn wait_until_returns_when_true_and_errors_on_timeout() {
    let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let f2 = flag.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        f2.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    let start = Instant::now();
    wait_until(Duration::from_secs(5), || {
        let flag = flag.clone();
        async move { flag.load(std::sync::atomic::Ordering::SeqCst) }
    })
    .await
    .unwrap();
    assert!(start.elapsed() < Duration::from_secs(2));

    let err = wait_until(Duration::from_millis(200), || async { false })
        .await
        .unwrap_err();
    assert!(err.to_string().contains("timed out"), "{err}");
}

#[tokio::test]
async fn cluster_shares_one_fake_clock() {
    let c = Cluster::new(2).await.unwrap();
    use shoebox_core::clock::Clock;
    let t0 = c.clock().now_ms();
    c.clock().advance(5_000);
    assert_eq!(c.clock().now_ms(), t0 + 5_000);
    c.shutdown().await;
}
