use shoebox_core::ids::EndpointId;
use shoebox_core::store::*;
use shoebox_node::LibraryTicket;

fn ticket() -> LibraryTicket {
    LibraryTicket {
        cap: LibraryCapability {
            library: LibraryId([4; 16]),
            root: RootPublicKey([5; 32]),
            mode: Capability::Write,
            backend: vec![1, 2, 3, 4],
        },
        peers: vec![PeerAddr {
            id: EndpointId([6; 32]),
            relay_url: Some("https://relay.example.com/".into()),
            direct_addrs: vec!["192.168.1.2:4433".into()],
        }],
    }
}

#[test]
fn ticket_round_trips_through_its_string_form() {
    let t = ticket();
    let text = t.to_string();
    assert!(text.starts_with("shoebox-library1"), "{text}");
    assert!(
        text.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
        "copy-paste friendly: {text}"
    );
    assert_eq!(text.parse::<LibraryTicket>().unwrap(), t);
}

#[test]
fn malformed_tickets_are_rejected() {
    let good = ticket().to_string();
    assert!("".parse::<LibraryTicket>().is_err());
    assert!("hello".parse::<LibraryTicket>().is_err());
    assert!(good
        .replacen("shoebox-library1", "other1", 1)
        .parse::<LibraryTicket>()
        .is_err());
    assert!(
        format!("{good}zz").parse::<LibraryTicket>().is_err(),
        "trailing garbage"
    );
    assert!(
        good[..good.len() - 7].parse::<LibraryTicket>().is_err(),
        "truncated"
    );
    let mut chars: Vec<char> = good.chars().collect();
    let last = chars.len() - 1;
    chars[last] = if chars[last] == 'a' { 'b' } else { 'a' };
    // A flipped final character is caught by the checksum.
    assert!(chars
        .into_iter()
        .collect::<String>()
        .parse::<LibraryTicket>()
        .is_err());
}

#[test]
fn ticket_without_peers_is_valid_for_offline_sharing() {
    let mut t = ticket();
    t.peers.clear();
    assert_eq!(t.to_string().parse::<LibraryTicket>().unwrap(), t);
}
