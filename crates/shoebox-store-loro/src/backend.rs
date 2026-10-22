use crate::library::{LibraryShared, LoroLibrary};
use crate::net::IrohNet;
use async_trait::async_trait;
use shoebox_core::clock::Clock;
use shoebox_core::hlc::Hlc;
use shoebox_core::identity::KeyPair;
use shoebox_core::ids::{Blake3Hash, EndpointId};
use shoebox_core::model::{Member, MemberRole};
use shoebox_core::store::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Registry of replicas for the in-process transport (tests): `sync_with(peer)` finds the peer's
/// replica of the same library here.
#[derive(Clone, Default)]
pub struct LoroNetwork(pub(crate) Arc<Mutex<ReplicaMap>>);

pub(crate) type ReplicaMap = HashMap<(LibraryId, EndpointId), Arc<LibraryShared>>;

impl LoroNetwork {
    pub fn new() -> Self {
        Self::default()
    }
}

/// How replicas reach each other.
#[derive(Clone)]
pub(crate) enum Transport {
    /// Same process, through a [`LoroNetwork`] registry (conformance tests, benchmarks).
    InProcess(LoroNetwork),
    /// iroh connections to other devices (`shoebox/loro/0`).
    Iroh(Arc<IrohNet>),
}

impl Transport {
    pub(crate) async fn sync(
        &self,
        shared: &Arc<LibraryShared>,
        library: LibraryId,
        peer: &PeerAddr,
    ) -> Result<()> {
        match self {
            Transport::Iroh(net) => net.sync(shared, library, peer).await,
            Transport::InProcess(network) => {
                let theirs = network
                    .0
                    .lock()
                    .unwrap()
                    .get(&(library, peer.id))
                    .cloned()
                    .ok_or(StoreError::Unreachable(peer.id))?;
                if Arc::ptr_eq(&theirs, shared) {
                    return Ok(());
                }
                // Real Loro exchange, version-vector based, in both directions.
                let for_me = theirs.updates_since(&shared.oplog_vv())?;
                shared.import_bytes(&for_me)?;
                let for_them = shared.updates_since(&theirs.oplog_vv())?;
                theirs.import_bytes(&for_them)?;
                Ok(())
            }
        }
    }
}

pub struct LoroBackend {
    transport: Transport,
    device: EndpointId,
    clock: Arc<dyn Clock>,
    signer: Option<KeyPair>,
    /// Where replicas are persisted (`<dir>/<library id hex>/`); `None` = memory only.
    data_dir: Option<PathBuf>,
}

impl LoroBackend {
    /// In-process replicas (no network, no disk, unsigned records): conformance tests and benchmarks.
    pub fn new(network: LoroNetwork, device: EndpointId, clock: impl Clock) -> Self {
        Self {
            transport: Transport::InProcess(network),
            device,
            clock: Arc::new(clock),
            signer: None,
            data_dir: None,
        }
    }

    /// The production backend for one node: iroh transport, records signed with the node's
    /// device key, replicas persisted under the node's data directory.
    pub fn for_node(node: shoebox_node::Node, clock: impl Clock) -> Self {
        let device = shoebox_node::from_iroh_id(&node.id());
        let dir = node.config().data_dir.join("loro");
        let signer = node.device_key();
        Self {
            transport: Transport::Iroh(IrohNet::new(node)),
            device,
            clock: Arc::new(clock),
            signer: Some(signer),
            data_dir: Some(dir),
        }
    }

    /// Sign this device's records with `key` (its id must be `key`'s public key).
    pub fn with_signer(mut self, key: KeyPair) -> Self {
        self.signer = Some(key);
        self
    }

    /// Persist replicas under `dir/<library id hex>/`.
    pub fn with_data_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.data_dir = Some(dir.into());
        self
    }

    fn library(&self, id: LibraryId, root: RootPublicKey, mode: Capability) -> Result<LoroLibrary> {
        let make = || -> Result<Arc<LibraryShared>> {
            let dir = self.data_dir.as_ref().map(|d| d.join(hex::encode(id.0)));
            Ok(Arc::new(LibraryShared::new(
                self.device,
                root,
                id,
                self.signer.clone(),
                dir.as_deref(),
            )?))
        };
        let shared = match &self.transport {
            Transport::InProcess(network) => {
                let mut map = network.0.lock().unwrap();
                match map.get(&(id, self.device)) {
                    Some(s) => s.clone(),
                    None => {
                        let s = make()?;
                        map.insert((id, self.device), s.clone());
                        s
                    }
                }
            }
            Transport::Iroh(net) => {
                let s = make()?;
                net.register(id, &s);
                s
            }
        };
        Ok(LoroLibrary::new(
            shared,
            self.transport.clone(),
            id,
            self.clock.clone(),
            mode,
        ))
    }
}

/// Shared albums are not part of this backend: they stay on iroh-docs (see `shoebox-store`).
pub struct UnsupportedAlbum;

fn unsupported<T>() -> Result<T> {
    Err(StoreError::Backend(
        "shared albums are not provided by the Loro backend (use shoebox-store's HybridBackend)"
            .into(),
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
        let id = match &self.transport {
            Transport::InProcess(network) => {
                let mut bytes = self.device.0.to_vec();
                bytes.extend_from_slice(&root.0);
                bytes.extend_from_slice(&(network.0.lock().unwrap().len() as u64).to_le_bytes());
                let mut id = [0u8; 16];
                id.copy_from_slice(&Blake3Hash::of_bytes(&bytes).0[..16]);
                LibraryId(id)
            }
            Transport::Iroh(_) => {
                let mut id = [0u8; 16];
                getrandom::fill(&mut id).expect("operating system randomness");
                LibraryId(id)
            }
        };
        let cap = LibraryCapability {
            library: id,
            root: *root,
            mode: Capability::Write,
            // Loro has no bearer secret: access is the device list (see `auth`).
            backend: vec![crate::CAPABILITY_VERSION],
        };
        Ok((self.library(id, *root, Capability::Write)?, cap))
    }

    fn downgrade(&self, cap: &LibraryCapability) -> LibraryCapability {
        cap.read_only()
    }

    async fn open_library(&self, cap: &LibraryCapability) -> Result<LoroLibrary> {
        self.library(cap.library, cap.root, cap.mode)
    }

    async fn create_album(&self, _meta: SharedAlbumMeta) -> Result<UnsupportedAlbum> {
        unsupported()
    }

    async fn join_album(&self, _ticket: &InviteTicket) -> Result<UnsupportedAlbum> {
        unsupported()
    }
}
