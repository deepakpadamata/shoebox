//! Conversions between the backend-agnostic [`PeerAddr`] and iroh's `EndpointAddr`.

use iroh::{EndpointAddr, EndpointId as IrohId, TransportAddr};
use shoebox_core::ids::EndpointId;
use shoebox_core::store::{PeerAddr, StoreError};

pub fn to_iroh_id(id: &EndpointId) -> Result<IrohId, StoreError> {
    IrohId::from_bytes(&id.0)
        .map_err(|e| StoreError::Invalid(format!("not a valid endpoint id: {e}")))
}

pub fn from_iroh_id(id: &IrohId) -> EndpointId {
    EndpointId(*id.as_bytes())
}

/// `PeerAddr` → iroh address. Unparseable direct addresses are skipped (the id still works with
/// address lookup).
pub fn to_endpoint_addr(peer: &PeerAddr) -> Result<EndpointAddr, StoreError> {
    let mut addrs: Vec<TransportAddr> = peer
        .direct_addrs
        .iter()
        .filter_map(|a| a.parse().ok().map(TransportAddr::Ip))
        .collect();
    if let Some(relay) = peer.relay_url.as_deref().and_then(|u| u.parse().ok()) {
        addrs.push(TransportAddr::Relay(relay));
    }
    Ok(EndpointAddr::from_parts(to_iroh_id(&peer.id)?, addrs))
}

pub fn from_endpoint_addr(addr: &EndpointAddr) -> PeerAddr {
    PeerAddr {
        id: from_iroh_id(&addr.id),
        relay_url: addr.addrs.iter().find_map(|a| match a {
            TransportAddr::Relay(url) => Some(url.to_string()),
            _ => None,
        }),
        direct_addrs: addr
            .addrs
            .iter()
            .filter_map(|a| match a {
                TransportAddr::Ip(ip) => Some(ip.to_string()),
                _ => None,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_addr_round_trips_through_iroh() {
        let key = iroh::SecretKey::generate().public();
        let original = EndpointAddr::from_parts(
            key,
            [
                TransportAddr::Ip("192.168.1.5:4433".parse().unwrap()),
                TransportAddr::Ip("[::1]:9".parse().unwrap()),
                TransportAddr::Relay("https://relay.example.com".parse().unwrap()),
            ],
        );
        let peer = from_endpoint_addr(&original);
        assert_eq!(peer.id, EndpointId(*key.as_bytes()));
        assert_eq!(peer.direct_addrs.len(), 2);
        assert!(peer
            .relay_url
            .as_deref()
            .unwrap()
            .contains("relay.example.com"));
        assert_eq!(to_endpoint_addr(&peer).unwrap(), original);
    }

    #[test]
    fn invalid_ids_and_addresses_are_handled() {
        let bad = PeerAddr {
            id: EndpointId([0xFF; 32]),
            relay_url: None,
            direct_addrs: vec![],
        };
        // 0xFF.. is not a valid curve point: reported, not panicked on.
        let _ = to_endpoint_addr(&bad);
        let key = iroh::SecretKey::generate().public();
        let junk = PeerAddr {
            id: EndpointId(*key.as_bytes()),
            relay_url: Some("not a url".into()),
            direct_addrs: vec!["nonsense".into(), "10.0.0.1:1".into()],
        };
        let addr = to_endpoint_addr(&junk).unwrap();
        assert_eq!(addr.addrs.len(), 1, "only the parseable address survives");
    }
}
