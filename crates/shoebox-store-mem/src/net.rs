//! The simulated network and the backend entry point.

use crate::album::{AlbumShared, MemAlbum};
use crate::library::{LibShared, MemLibrary};
use async_trait::async_trait;
use shoebox_core::clock::Clock;
use shoebox_core::ids::{Blake3Hash, EndpointId};
use shoebox_core::model::AlbumId;
use shoebox_core::store::*;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub(crate) struct NetInner {
    pub libs: HashMap<(LibraryId, EndpointId), Arc<LibShared>>,
    pub albums: HashMap<(AlbumId, EndpointId), Arc<AlbumShared>>,
    pub counter: u64,
}

/// Shared registry of replicas; clone it to let backends reach each other.
#[derive(Clone, Default)]
pub struct MemNetwork(pub(crate) Arc<Mutex<NetInner>>);

impl MemNetwork {
    pub fn new() -> Self {
        Self::default()
    }

    /// A unique 32-byte nonce for ids and invite secrets (deterministic per network).
    pub(crate) fn nonce(&self, device: EndpointId, domain: &str) -> [u8; 32] {
        let mut inner = self.0.lock().unwrap();
        inner.counter += 1;
        let mut bytes = domain.as_bytes().to_vec();
        bytes.extend_from_slice(&device.0);
        bytes.extend_from_slice(&inner.counter.to_be_bytes());
        Blake3Hash::of_bytes(&bytes).0
    }
}

/// One device's view of the in-memory backend.
pub struct MemBackend {
    pub(crate) network: MemNetwork,
    pub(crate) device: EndpointId,
    pub(crate) clock: Arc<dyn Clock>,
}

impl MemBackend {
    pub fn new(network: MemNetwork, device: EndpointId, clock: impl Clock) -> Self {
        Self {
            network,
            device,
            clock: Arc::new(clock),
        }
    }

    fn library(&self, id: LibraryId, mode: Capability) -> MemLibrary {
        let shared = {
            let mut inner = self.network.0.lock().unwrap();
            inner
                .libs
                .entry((id, self.device))
                .or_insert_with(|| Arc::new(LibShared::new(self.device)))
                .clone()
        };
        MemLibrary::new(shared, self.network.clone(), id, self.clock.clone(), mode)
    }
}

#[async_trait]
impl StoreBackend for MemBackend {
    type Library = MemLibrary;
    type Album = MemAlbum;

    fn device(&self) -> EndpointId {
        self.device
    }

    async fn create_library(
        &self,
        root: &RootPublicKey,
    ) -> Result<(MemLibrary, LibraryCapability)> {
        let nonce = self.network.nonce(self.device, "library");
        let mut id = [0u8; 16];
        id.copy_from_slice(&nonce[..16]);
        let id = LibraryId(id);
        let cap = LibraryCapability {
            library: id,
            root: *root,
            mode: Capability::Write,
            backend: vec![],
        };
        Ok((self.library(id, Capability::Write), cap))
    }

    async fn open_library(&self, cap: &LibraryCapability) -> Result<MemLibrary> {
        Ok(self.library(cap.library, cap.mode))
    }

    async fn create_album(&self, meta: SharedAlbumMeta) -> Result<MemAlbum> {
        let shared = Arc::new(AlbumShared::new(meta.clone(), self.device));
        self.network
            .0
            .lock()
            .unwrap()
            .albums
            .insert((meta.id, self.device), shared.clone());
        let album = MemAlbum::new(
            shared,
            self.network.clone(),
            meta.id,
            self.device,
            self.clock.clone(),
        );
        album.install_owner(self.clock.now_ms());
        Ok(album)
    }

    async fn join_album(&self, ticket: &InviteTicket) -> Result<MemAlbum> {
        if let Expiry::At(t) = ticket.expires {
            if self.clock.now_ms() as i64 > t.0 {
                return Err(StoreError::Expired);
            }
        }
        let inviter = self
            .network
            .0
            .lock()
            .unwrap()
            .albums
            .get(&(ticket.album, ticket.inviter.id))
            .cloned()
            .ok_or(StoreError::Unreachable(ticket.inviter.id))?;
        let snapshot = inviter.redeem(ticket)?;
        let shared = Arc::new(AlbumShared::from_snapshot(snapshot, self.device));
        self.network
            .0
            .lock()
            .unwrap()
            .albums
            .insert((ticket.album, self.device), shared.clone());
        let album = MemAlbum::new(
            shared,
            self.network.clone(),
            ticket.album,
            self.device,
            self.clock.clone(),
        );
        album.install_member(ticket.role, self.clock.now_ms());
        Ok(album)
    }
}
