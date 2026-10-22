//! The production `StoreBackend` (decision #7, M1-16): the personal library lives in a Loro
//! document ([`shoebox_store_loro`]) and shared albums stay on iroh-docs ([`shoebox_store_docs`]),
//! whose namespace capabilities give albums their read/write roles.

mod shared;

pub use shared::{
    decode_invite, encode_invite, item_blob, AlbumNotice, MemberInfo, SharedAlbums, SharedSummary,
};

use async_trait::async_trait;
use shoebox_core::clock::Clock;
use shoebox_core::ids::EndpointId;
use shoebox_core::store::*;
use shoebox_node::Node;
use shoebox_store_docs::{DocsAlbum, DocsBackend};
use shoebox_store_loro::{LoroBackend, LoroLibrary};

pub use shoebox_store_docs::DocsLibrary;
pub use shoebox_store_loro::LoroLibrary as Library;

/// Library on Loro, albums on iroh-docs, for one device.
pub struct HybridBackend {
    library: LoroBackend,
    albums: DocsBackend,
}

impl HybridBackend {
    /// Both halves use `node`'s endpoint, key and data directory. `clock` stamps record HLCs.
    pub fn for_node(node: Node, clock: impl Clock + Clone) -> Self {
        Self {
            library: LoroBackend::for_node(node.clone(), clock.clone()),
            albums: DocsBackend::for_node(node, clock),
        }
    }
}

#[async_trait]
impl StoreBackend for HybridBackend {
    type Library = LoroLibrary;
    type Album = DocsAlbum;

    fn device(&self) -> EndpointId {
        self.library.device()
    }

    async fn create_library(
        &self,
        root: &RootPublicKey,
    ) -> Result<(LoroLibrary, LibraryCapability)> {
        self.library.create_library(root).await
    }

    fn downgrade(&self, cap: &LibraryCapability) -> LibraryCapability {
        self.library.downgrade(cap)
    }

    async fn open_library(&self, cap: &LibraryCapability) -> Result<LoroLibrary> {
        self.library.open_library(cap).await
    }

    async fn create_album(&self, meta: SharedAlbumMeta) -> Result<DocsAlbum> {
        self.albums.create_album(meta).await
    }

    async fn join_album(&self, ticket: &InviteTicket) -> Result<DocsAlbum> {
        self.albums.join_album(ticket).await
    }
}
