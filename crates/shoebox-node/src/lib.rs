//! The Shoebox runtime (M1-01).

mod activity;
mod addr;
mod blobs;
mod config;
mod doc_digest;
mod ext;
mod fetcher;
pub mod format;
mod freeup;
mod holdings;
mod identity;
mod import;
mod in_place;
mod keys;
mod library_node;
mod limit;
mod maintenance;
mod node;
mod pairing;
mod presence;
mod recover;
mod rotation;
mod sync;
mod ticket;
pub mod update;

pub use activity::Activity;
pub use addr::{from_endpoint_addr, from_iroh_id, to_endpoint_addr, to_iroh_id};
pub use blobs::{BlobStore, ImportMode, Protection};
pub use config::{NodeConfig, RelaySetting};
pub use doc_digest::{doc_digest, DOC_DIGEST_ALPN};
pub use ext::{ConnHandler, ExtSlot};
pub use fetcher::{FetchSource, FetcherConfig};
pub use freeup::{FreeReport, CONFIRMATION_TTL_MS};
pub use holdings::{DeviceStorage, DEFAULT_REDUNDANCY, REDUNDANCY_KEY};
pub use identity::{Membership, RECOVERY_KIT_SAVED_KEY};
pub use import::{ImportOptions, ImportReport};
pub use keys::load_or_create_secret;
pub use library_node::LibraryNode;
pub use maintenance::MaintenanceConfig;
pub use node::{
    classify_addrs, FetchReport, Node, NodeEvent, NodeEvents, PathKind, PeerAddr, PeerInfo,
    ALBUM_ALPN, HAS_ALPN, LORO_ALPN, PAIR_ALPN, PING_ALPN, RECOVER_ALPN, ROTATE_ALPN,
};
pub use pairing::{DecideFn, PairGrant, PairTicket, PairingCode};
pub use presence::{DeviceStatus, PresenceConfig};
pub use recover::{PeerLink, RecoverGrant, RecoveryDecideFn, RecoveryRequest};
pub use sync::{SyncConfig, SyncPhase};
pub use ticket::{LibraryTicket, TicketError};
