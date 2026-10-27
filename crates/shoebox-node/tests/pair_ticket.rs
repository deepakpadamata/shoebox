//! M2-04: pairing tickets round-trip through their text (QR) form and reject damage.

use shoebox_core::ids::{EndpointId, Timestamp};
use shoebox_core::model::Role;
use shoebox_core::store::PeerAddr;
use shoebox_node::PairTicket;

fn ticket() -> PairTicket {
    PairTicket {
        addr: PeerAddr {
            id: EndpointId([3; 32]),
            relay_url: Some("https://relay.example/".into()),
            direct_addrs: vec!["192.168.1.5:4000".into()],
        },
        secret: [9; 32],
        expires: Timestamp(1_700_000_300_000),
        role: Role::Companion,
    }
}

#[test]
fn text_form_round_trips() {
    let t = ticket();
    let text = t.to_string();
    assert!(text.starts_with("shoebox-pair1"));
    assert_eq!(text.parse::<PairTicket>().unwrap(), t);
    assert_eq!(
        format!(" {text}\n").parse::<PairTicket>().unwrap(),
        t,
        "whitespace is forgiven"
    );
}

#[test]
fn damaged_tickets_are_rejected() {
    let text = ticket().to_string();
    assert!("shoebox-library1abcd".parse::<PairTicket>().is_err());
    assert!("shoebox-pair1zz".parse::<PairTicket>().is_err());
    let mut chars: Vec<char> = text.chars().collect();
    let i = chars.len() - 10;
    chars[i] = if chars[i] == '0' { '1' } else { '0' };
    assert!(chars
        .into_iter()
        .collect::<String>()
        .parse::<PairTicket>()
        .is_err());
    assert!(
        text[..text.len() - 4].parse::<PairTicket>().is_err(),
        "truncated"
    );
}
