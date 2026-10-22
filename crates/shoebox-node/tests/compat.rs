//! M6-05: the rule that keeps neighbouring versions talking. Every protocol is named by an ALPN
//! with a version number (`shoebox/loro/0`). A release that changes a protocol keeps serving the
//! old name next to the new one for a release (N and N-1 interoperate); a peer that only speaks a
//! newer name than we serve is refused cleanly, and nothing else about the node is disturbed.

use shoebox_node::{
    Node, NodeConfig, ALBUM_ALPN, DOC_DIGEST_ALPN, HAS_ALPN, LORO_ALPN, PAIR_ALPN, PING_ALPN,
    RECOVER_ALPN, ROTATE_ALPN,
};

#[test]
fn every_protocol_name_carries_a_version() {
    for alpn in [
        LORO_ALPN, PAIR_ALPN, PING_ALPN, RECOVER_ALPN, ROTATE_ALPN, HAS_ALPN, ALBUM_ALPN,
        DOC_DIGEST_ALPN,
    ] {
        let s = std::str::from_utf8(alpn).unwrap();
        let parts: Vec<&str> = s.split('/').collect();
        assert_eq!(parts.len(), 3, "{s}");
        assert_eq!(parts[0], "shoebox", "{s}");
        assert!(parts[2].parse::<u32>().is_ok(), "{s} has no version number");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_newer_protocol_version_is_refused_without_harm() {
    let dir = tempfile::tempdir().unwrap();
    let a = Node::open(NodeConfig::local_only(dir.path().join("a"), "a")).await.unwrap();
    let b = Node::open(NodeConfig::local_only(dir.path().join("b"), "b")).await.unwrap();
    b.add_peer_addr(a.addr());
    let to = a.addr();
    let newer = b.endpoint().connect(to.clone(), b"shoebox/loro/99").await;
    assert!(newer.is_err(), "a protocol version nobody serves cannot be opened");
    // The version we do serve still works afterwards.
    let ok = b.endpoint().connect(to, PING_ALPN).await;
    assert!(ok.is_ok(), "{:?}", ok.err());
}
