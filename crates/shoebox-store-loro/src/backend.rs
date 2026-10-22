use crate::library::{LibraryShared, LoroLibrary};
use async_trait::async_trait;
use shoebox_core::clock::Clock;
use shoebox_core::hlc::Hlc;
use shoebox_core::ids::{Blake3Hash, EndpointId};
use shoebox_core::model::{Member, MemberRole};
use shoebox_core::store::*;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Registry of replicas; `sync_with(peer)` finds the peer's replica of the same library here.
#[derive(Clone, Default)]
pub struct LoroNetwork(pub(crate) Arc<Mutex<ReplicaMap>>);

pub(crate) type ReplicaMap = HashMap<(LibraryId, EndpointId), Arc<LibraryShared>>;

impl LoroNetwork {
    pub fn new() -> Self {
        Self::default()
    }
}

pub struct LoroBackend {
    network: LoroNetwork,
    device: EndpointId,
    clock: Arc<dyn Clock>,
}

impl LoroBackend {
    pub fn new(network: LoroNetwork, device: EndpointId, clock: impl Clock) -> Self {
        Self {
            network,
            device,
            clock: Arc::new(clock),
        }
    }

    fn library(&self, id: LibraryId, root: RootPublicKey, mode: Capability) -> LoroLibrary {
        let shared = self
            .network
            .0
            .lock()
            .unwrap()
            .entry((id, self.device))
            .or_insert_with(|| Arc::new(LibraryShared::new(self.device, root, id)))
            .clone();
        LoroLibrary::new(shared, self.network.clone(), id, self.clock.clone(), mode)
    }
}

/// Shared albums are outside the Loro spike: every method reports that.
pub struct UnsupportedAlbum;

fn unsupported<T>() -> Result<T> {
    Err(StoreError::Backend(
        "shared albums are not implemented in the Loro spike".into(),
    ))
}

#[async_trait]
impl AlbumStore for UnsupportedAlbum {
    async fn meta(&self) -> Result<SharedAlbumMeta> {
        unsupported()
    }
    async fn add_items(&self, _items: Vec<shoebox_core::model::SharedItem>) -> Result<()> {
        unsupported()
    }
    async fn items(&self, _page: Page) -> Result<Vec<shoebox_core::model::SharedItem>> {
        unsupported()
    }
    async fn members(&self) -> Result<Vec<Member>> {
        unsupported()
    }
    async fn events(&self, _since: Option<Hlc>) -> Result<Vec<AlbumEvent>> {
        unsupported()
    }
    async fn invite(&self, _role: MemberRole, _expiry: Expiry) -> Result<InviteTicket> {
        unsupported()
    }
    fn subscribe(&self) -> futures::stream::BoxStream<'static, Change> {
        Box::pin(futures::stream::empty())
    }
    async fn sync_with(&self, _peer: &PeerAddr) -> Result<()> {
        unsupported()
    }
}

#[async_trait]
impl StoreBackend for LoroBackend {
    type Library = LoroLibrary;
    type Album = UnsupportedAlbum;

    fn device(&self) -> EndpointId {
        self.device
    }

    async fn create_library(
        &self,
        root: &RootPublicKey,
    ) -> Result<(LoroLibrary, LibraryCapability)> {
        let mut bytes = self.device.0.to_vec();
        bytes.extend_from_slice(&root.0);
        bytes.extend_from_slice(&(self.network.0.lock().unwrap().len() as u64).to_le_bytes());
        let mut id = [0u8; 16];
        id.copy_from_slice(&Blake3Hash::of_bytes(&bytes).0[..16]);
        let id = LibraryId(id);
        let cap = LibraryCapability {
            library: id,
            root: *root,
            mode: Capability::Write,
            backend: vec![],
        };
        Ok((self.library(id, *root, Capability::Write), cap))
    }

    async fn open_library(&self, cap: &LibraryCapability) -> Result<LoroLibrary> {
        Ok(self.library(cap.library, cap.root, cap.mode))
    }

    async fn create_album(&self, _meta: SharedAlbumMeta) -> Result<UnsupportedAlbum> {
        unsupported()
    }

    async fn join_album(&self, _ticket: &InviteTicket) -> Result<UnsupportedAlbum> {
        unsupported()
    }
}
