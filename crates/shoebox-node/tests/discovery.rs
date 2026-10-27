//! M1-09: LAN discovery, relay configuration and honest connection reporting.

use shoebox_node::{classify_addrs, Node, NodeConfig, PathKind, RelaySetting};
use std::net::SocketAddr;
use std::time::Duration;

fn sock(s: &str) -> SocketAddr {
    s.parse().unwrap()
}

#[test]
fn addresses_are_classified_as_lan_direct_or_relay() {
    use iroh::TransportAddr as T;
    let lan = |s: &str| classify_addrs(&[T::Ip(sock(s))]);
    for private in [
        "192.168.1.20:1",
        "10.0.0.5:1",
        "172.16.9.9:1",
        "172.31.255.1:1",
        "169.254.1.1:1",
        "127.0.0.1:1",
        "[::1]:1",
        "[fe80::1]:1",
        "[fd12:3456::1]:1",
    ] {
        assert_eq!(lan(private), PathKind::Lan, "{private}");
    }
    for public in ["8.8.8.8:1", "172.32.0.1:1", "1.2.3.4:9", "[2001:db8::1]:1"] {
        assert_eq!(lan(public), PathKind::Direct, "{public}");
    }
    let relay = T::Relay("https://euw1-1.relay.iroh.network./".parse().unwrap());
    assert!(matches!(
        classify_addrs(std::slice::from_ref(&relay)),
        PathKind::Relay { .. }
    ));
    // Mixed: an in-use direct path wins over a relay path; LAN wins over direct.
    assert_eq!(
        classify_addrs(&[relay.clone(), T::Ip(sock("8.8.8.8:1"))]),
        PathKind::Direct
    );
    assert_eq!(
        classify_addrs(&[T::Ip(sock("8.8.8.8:1")), T::Ip(sock("10.0.0.1:1")), relay]),
        PathKind::Lan
    );
    assert_eq!(classify_addrs(&[]), PathKind::Unknown);
}

#[test]
fn relay_settings_round_trip_through_config() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = NodeConfig::local_only(dir.path(), "x");
    cfg.relay = RelaySetting::Custom("https://relay.example.com".into());
    cfg.mdns = true;
    cfg.ensure_dirs().unwrap();
    cfg.save().unwrap();
    assert_eq!(NodeConfig::load(dir.path()).unwrap(), cfg);
    // Older config files without the new fields still load, with sensible defaults.
    std::fs::write(
        dir.path().join("config.toml"),
        "name = \"old\"\nrole = \"Keeper\"\nrelay = \"Disabled\"\n",
    )
    .unwrap();
    let old = NodeConfig::load(dir.path()).unwrap();
    assert!(old.mdns, "LAN discovery is on unless turned off");
}

#[tokio::test]
async fn a_bad_custom_relay_url_is_rejected_a_good_one_opens() {
    let dir = tempfile::tempdir().unwrap();
    let mut bad = NodeConfig::local_only(dir.path().join("bad"), "bad");
    bad.relay = RelaySetting::Custom("not a url".into());
    assert!(Node::open(bad).await.is_err());

    let mut good = NodeConfig::local_only(dir.path().join("good"), "good");
    good.relay = RelaySetting::Custom("https://relay.example.com".into());
    let node = Node::open(good)
        .await
        .expect("a custom relay is accepted without contacting it");
    node.shutdown().await.unwrap();
}

#[tokio::test]
async fn peer_info_reports_path_latency_and_last_seen() {
    let dir = tempfile::tempdir().unwrap();
    let a = Node::open(NodeConfig::local_only(dir.path().join("a"), "a"))
        .await
        .unwrap();
    let b = Node::open(NodeConfig::local_only(dir.path().join("b"), "b"))
        .await
        .unwrap();
    assert!(a.peer_info(b.id()).await.is_none(), "never heard of it");

    a.add_peer_addr(b.addr());
    a.ping(b.id()).await.unwrap();
    let info = a.peer_info(b.id()).await.expect("known after a ping");
    assert_eq!(info.id, b.id());
    assert_eq!(info.path, PathKind::Lan, "loopback counts as local");
    assert!(
        info.latency.is_some_and(|l| l < Duration::from_secs(5)),
        "{info:?}"
    );
    assert!(info
        .last_seen
        .is_some_and(|t| t.elapsed() < Duration::from_secs(5)));
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
}

/// Two nodes with *no* address exchange and no relay find each other through mDNS, which is what
/// makes sync work with the internet unplugged. Needs working multicast on this machine.
#[tokio::test]
async fn two_nodes_find_each_other_over_mdns_without_a_relay() {
    // GitHub's macOS runners are VMs without working multicast (the Linux runner and a real Mac
    // pass): skip there only, and say so.
    if cfg!(target_os = "macos") && std::env::var_os("GITHUB_ACTIONS").is_some() {
        eprintln!("skipped: no multicast on GitHub's macOS runners");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mut ca = NodeConfig::local_only(dir.path().join("a"), "a");
    let mut cb = NodeConfig::local_only(dir.path().join("b"), "b");
    ca.mdns = true;
    cb.mdns = true;
    let a = Node::open(ca).await.unwrap();
    let b = Node::open(cb).await.unwrap();

    let mut found = false;
    for _ in 0..40 {
        // B has never been told where A is.
        if tokio::time::timeout(Duration::from_secs(3), b.ping(a.id()))
            .await
            .is_ok_and(|r| r.is_ok())
        {
            found = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(
        found,
        "mDNS did not resolve the peer (is multicast blocked here?)"
    );
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
}
