//! M1-01: node skeleton. Two nodes in one process open, find each other, connect and shut down;
//! a restarted node keeps its identity.

use shoebox_node::{Node, NodeConfig, NodeEvent};
use std::time::Duration;
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(20);

async fn open(dir: &tempfile::TempDir, name: &str) -> Node {
    Node::open(NodeConfig::local_only(dir.path().join(name), name))
        .await
        .expect("node opens")
}

async fn next_matching(
    rx: &mut shoebox_node::NodeEvents,
    mut pred: impl FnMut(&NodeEvent) -> bool,
) -> NodeEvent {
    timeout(WAIT, async {
        loop {
            let ev = rx.recv().await.expect("event stream open");
            if pred(&ev) {
                return ev;
            }
        }
    })
    .await
    .expect("event arrives in time")
}

#[tokio::test]
async fn data_dir_layout_is_created() {
    let dir = tempfile::tempdir().unwrap();
    let node = open(&dir, "a").await;
    let root = dir.path().join("a");
    for sub in ["blobs", "store", "keys"] {
        assert!(root.join(sub).is_dir(), "{sub}/ exists");
    }
    assert!(root.join("config.toml").is_file());
    assert!(root.join("keys/endpoint.key").is_file());
    node.shutdown().await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn endpoint_key_file_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let node = open(&dir, "a").await;
    let mode = std::fs::metadata(dir.path().join("a/keys/endpoint.key"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
    node.shutdown().await.unwrap();
}

#[tokio::test]
async fn restart_keeps_the_endpoint_id() {
    let dir = tempfile::tempdir().unwrap();
    let first = open(&dir, "a").await;
    let id = first.id();
    first.shutdown().await.unwrap();
    let second = open(&dir, "a").await;
    assert_eq!(second.id(), id);
    second.shutdown().await.unwrap();
    // A different data dir is a different device.
    let other = open(&dir, "b").await;
    assert_ne!(other.id(), id);
    other.shutdown().await.unwrap();
}

#[tokio::test]
async fn new_subscribers_see_started_first() {
    let dir = tempfile::tempdir().unwrap();
    let node = open(&dir, "a").await;
    let mut rx = node.events();
    assert!(matches!(rx.recv().await.unwrap(), NodeEvent::Started { id } if id == node.id()));
    node.shutdown().await.unwrap();
}

#[tokio::test]
async fn two_nodes_connect_report_peers_and_shut_down_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let a = open(&dir, "a").await;
    let b = open(&dir, "b").await;
    let mut a_events = a.events();
    let mut b_events = b.events();

    // Nodes know each other's address (no discovery service is involved).
    let a_addr = a.addr();
    assert_eq!(a_addr.id, a.id());
    assert!(
        !a_addr.addrs.is_empty(),
        "bound to at least one direct address"
    );
    b.add_peer_addr(a_addr);

    let rtt = b.ping(a.id()).await.expect("b reaches a");
    assert!(rtt < WAIT);

    let ev = next_matching(&mut a_events, |e| {
        matches!(e, NodeEvent::PeerConnected { .. })
    })
    .await;
    assert_eq!(ev, NodeEvent::PeerConnected { id: b.id() });
    let ev = next_matching(&mut b_events, |e| {
        matches!(e, NodeEvent::PeerConnected { .. })
    })
    .await;
    assert_eq!(ev, NodeEvent::PeerConnected { id: a.id() });

    // b goes away: a reports the disconnect.
    b.shutdown().await.unwrap();
    let ev = next_matching(&mut a_events, |e| {
        matches!(e, NodeEvent::PeerDisconnected { .. })
    })
    .await;
    assert_eq!(ev, NodeEvent::PeerDisconnected { id: b.id() });
    timeout(WAIT, a.shutdown())
        .await
        .expect("shutdown completes")
        .unwrap();
}

#[tokio::test]
async fn pinging_an_unknown_peer_fails_instead_of_hanging() {
    let dir = tempfile::tempdir().unwrap();
    let a = open(&dir, "a").await;
    let b = open(&dir, "b").await;
    // a was never told how to reach b.
    let res = timeout(WAIT, a.ping(b.id())).await.expect("fails in time");
    assert!(res.is_err());
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
}

#[tokio::test]
async fn blocking_a_peer_cuts_traffic_until_unblocked() {
    let dir = tempfile::tempdir().unwrap();
    let a = open(&dir, "a").await;
    let b = open(&dir, "b").await;
    a.add_peer_addr(b.addr());
    b.add_peer_addr(a.addr());
    a.ping(b.id()).await.expect("reachable at first");
    let mut a_events = a.events();

    a.block_peer(b.id());
    assert!(a.is_blocked(b.id()));
    // Existing connections are dropped...
    next_matching(&mut a_events, |e| {
        matches!(e, NodeEvent::PeerDisconnected { .. })
    })
    .await;
    // ...and neither direction can reconnect.
    assert!(
        timeout(WAIT, a.ping(b.id())).await.unwrap().is_err(),
        "outgoing is refused"
    );
    assert!(
        timeout(WAIT, b.ping(a.id())).await.unwrap().is_err(),
        "incoming is refused"
    );

    a.unblock_peer(b.id());
    assert!(!a.is_blocked(b.id()));
    timeout(WAIT, a.ping(b.id()))
        .await
        .unwrap()
        .expect("reachable again");
    timeout(WAIT, b.ping(a.id()))
        .await
        .unwrap()
        .expect("both ways");
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
}
