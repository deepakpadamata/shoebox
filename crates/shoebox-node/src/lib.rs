//! The Shoebox runtime (M1-01).

mod activity;
mod addr;
mod blobs;
mod config;
mod doc_digest;
mod fetcher;
mod holdings;
mod identity;
mod import;
mod keys;
mod library_node;
mod maintenance;
mod node;
mod presence;
mod sync;
mod ticket;

pub use activity::Activity;
pub use addr::{from_endpoint_addr, from_iroh_id, to_endpoint_addr, to_iroh_id};
pub use blobs::{BlobStore, ImportMode, Protection};
pub use config::{NodeConfig, RelaySetting};
pub use doc_digest::{doc_digest, DOC_DIGEST_ALPN};
pub use fetcher::{FetchSource, FetcherConfig};
pub use holdings::{DEFAULT_REDUNDANCY, REDUNDANCY_KEY};
pub use identity::{Membership, RECOVERY_KIT_SAVED_KEY};
pub use import::{ImportOptions, ImportReport};
pub use keys::load_or_create_secret;
pub use library_node::LibraryNode;
pub use maintenance::MaintenanceConfig;
pub use node::{
    classify_addrs, Node, NodeEvent, NodeEvents, PathKind, PeerAddr, PeerInfo, PING_ALPN,
};
pub use presence::{DeviceStatus, PresenceConfig};
pub use sync::{SyncConfig, SyncPhase};
pub use ticket::{LibraryTicket, TicketError};
