//! `StoreBackend` on iroh-docs (M1-05): one docs namespace per library and per shared album.

mod album;
pub mod codec;
mod library;
mod replica;

#[cfg(feature = "testing")]
pub mod harness;

pub use album::DocsAlbum;
pub use library::DocsLibrary;
pub use shoebox_node::{from_endpoint_addr, from_iroh_id, to_endpoint_addr, to_iroh_id};

use async_trait::async_trait;
use iroh_docs::api::protocol::{AddrInfoOptions, ShareMode};
use iroh_docs::{Capability as DocsCapability, DocTicket};
use replica::{wait_synced, Replica};
use shoebox_core::clock::Clock;
use shoebox_core::ids::EndpointId;
use shoebox_core::store::*;
use shoebox_node::Node;
use std::sync::Arc;

fn backend_err(e: impl std::fmt::Display) -> StoreError {
    StoreError::Backend(e.to_string())
}

/// The iroh-docs backend for one device. `device` is the identity written into records; in
/// production it is the node's own endpoint id ([`DocsBackend::for_node`]).
pub struct DocsBackend {
    node: Node,
    device: EndpointId,
    clock: Arc<dyn Clock>,
}

impl DocsBackend {
    pub fn new(node: Node, device: EndpointId, clock: impl Clock) -> Self {
        Self {
            node,
            device,
            clock: Arc::new(clock),
        }
    }

    /// Use the node's own endpoint id as the device identity.
    pub fn for_node(node: Node, clock: impl Clock) -> Self {
        let device = from_iroh_id(&node.id());
        Self::new(node, device, clock)
    }

    pub fn node(&self) -> &Node {
        &self.node
    }

    async fn replica(&self, doc: iroh_docs::api::Doc) -> Result<Replica> {
        let author = self
            .node
            .docs()
            .author_default()
            .await
            .map_err(backend_err)?;
        // A namespace only answers incoming syncs (and joins the gossip swarm) once live sync is
        // enabled; with no peers this just opens it for others to dial.
        doc.start_sync(vec![]).await.map_err(backend_err)?;
        Ok(Replica::new(doc, author, self.node.clone()))
    }
}

fn decode_capability(cap: &LibraryCapability) -> Result<DocsCapability> {
    postcard::from_bytes(&cap.backend)
        .map_err(|e| StoreError::Invalid(format!("not an iroh-docs library capability: {e}")))
}

#[async_trait]
impl StoreBackend for DocsBackend {
    type Library = DocsLibrary;
    type Album = DocsAlbum;

    fn device(&self) -> EndpointId {
        self.device
    }

    async fn create_library(
        &self,
        root: &RootPublicKey,
    ) -> Result<(DocsLibrary, LibraryCapability)> {
        let doc = self.node.docs().create().await.map_err(backend_err)?;
        let ticket = doc
            .share(ShareMode::Write, AddrInfoOptions::Id)
            .await
            .map_err(backend_err)?;
        let mut id = [0u8; 16];
        id.copy_from_slice(&doc.id().to_bytes()[..16]);
        let cap = LibraryCapability {
            library: LibraryId(id),
            root: *root,
            mode: Capability::Write,
            backend: postcard::to_stdvec(&ticket.capability).map_err(backend_err)?,
        };
        let library = DocsLibrary::open(
            self.replica(doc).await?,
            self.device,
            self.clock.clone(),
            Capability::Write,
        )
        .await?;
        Ok((library, cap))
    }

    fn downgrade(&self, cap: &LibraryCapability) -> LibraryCapability {
        let read = match decode_capability(cap) {
            Ok(DocsCapability::Write(secret)) => DocsCapability::Read(secret.id()),
            Ok(read @ DocsCapability::Read(_)) => read,
            Err(_) => return cap.read_only(),
        };
        LibraryCapability {
            mode: Capability::Read,
            backend: postcard::to_stdvec(&read).expect("capability encodes"),
            ..cap.clone()
        }
    }

    async fn open_library(&self, cap: &LibraryCapability) -> Result<DocsLibrary> {
        let capability = match (decode_capability(cap)?, cap.mode) {
            (DocsCapability::Write(secret), Capability::Read) => DocsCapability::Read(secret.id()),
            (c, _) => c,
        };
        let mode = match capability {
            DocsCapability::Write(_) => Capability::Write,
            DocsCapability::Read(_) => Capability::Read,
        };
        let doc = self
            .node
            .docs()
            .import_namespace(capability)
            .await
            .map_err(backend_err)?;
        DocsLibrary::open(
            self.replica(doc).await?,
            self.device,
            self.clock.clone(),
            mode,
        )
        .await
    }

    async fn create_album(&self, meta: SharedAlbumMeta) -> Result<DocsAlbum> {
        let doc = self.node.docs().create().await.map_err(backend_err)?;
        DocsAlbum::create(
            self.replica(doc).await?,
            meta,
            self.device,
            self.clock.clone(),
            self.node.clone(),
        )
        .await
    }

    async fn join_album(&self, ticket: &InviteTicket) -> Result<DocsAlbum> {
        if let Expiry::At(t) = ticket.expires {
            if self.clock.now_ms() as i64 > t.0 {
                return Err(StoreError::Expired);
            }
        }
        let doc_ticket: DocTicket = String::from_utf8(ticket.backend.clone())
            .map_err(|_| StoreError::Invalid("invite carries no iroh-docs ticket".into()))?
            .parse()
            .map_err(|e| StoreError::Invalid(format!("bad iroh-docs ticket: {e}")))?;
        let inviter = to_iroh_id(&ticket.inviter.id)?;
        let (doc, mut events) = self
            .node
            .docs()
            .import_and_subscribe(doc_ticket)
            .await
            .map_err(|_| StoreError::Unreachable(ticket.inviter.id))?;
        wait_synced(&mut events, inviter)
            .await
            .map_err(|_| StoreError::Unreachable(ticket.inviter.id))?;
        DocsAlbum::join(
            self.replica(doc).await?,
            ticket.role,
            self.device,
            self.clock.clone(),
            self.node.clone(),
            ticket.inviter.id,
        )
        .await
    }
}
