//! Plugs the Loro spike into the shoebox-core conformance suite (library scenarios only).

use crate::{LoroBackend, LoroNetwork};
use async_trait::async_trait;
use shoebox_core::clock::ManualClock;
use shoebox_core::conformance::Harness;
use shoebox_core::ids::EndpointId;
use shoebox_core::store::PeerAddr;

#[derive(Default)]
pub struct LoroHarness {
    network: LoroNetwork,
}

#[async_trait]
impl Harness for LoroHarness {
    type Backend = LoroBackend;

    fn fresh(&self) -> Self {
        Self::default()
    }

    async fn backend(&self, device: EndpointId, clock: ManualClock) -> LoroBackend {
        LoroBackend::new(self.network.clone(), device, clock)
    }

    fn peer_addr(&self, device: EndpointId) -> PeerAddr {
        PeerAddr::id_only(device)
    }
}
