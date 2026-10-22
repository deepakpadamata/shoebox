//! Plugs the in-memory backend into the shoebox-core conformance suite.

use crate::{MemBackend, MemNetwork};
use async_trait::async_trait;
use shoebox_core::clock::ManualClock;
use shoebox_core::conformance::Harness;
use shoebox_core::ids::EndpointId;
use shoebox_core::store::PeerAddr;

/// Each [`Harness::fresh`] world gets its own [`MemNetwork`].
#[derive(Default)]
pub struct MemHarness {
    network: MemNetwork,
}

#[async_trait]
impl Harness for MemHarness {
    type Backend = MemBackend;

    fn fresh(&self) -> Self {
        Self::default()
    }

    async fn backend(&self, device: EndpointId, clock: ManualClock) -> MemBackend {
        MemBackend::new(self.network.clone(), device, clock)
    }

    fn peer_addr(&self, device: EndpointId) -> PeerAddr {
        PeerAddr::id_only(device)
    }
}
