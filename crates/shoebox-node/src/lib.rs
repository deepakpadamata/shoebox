//! The Shoebox runtime (M1-01).

mod activity;
mod addr;
mod blobs;
mod clip;
mod compact;
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
mod indexer;
mod keys;
mod library_node;
mod limit;
mod maintenance;
mod model_download;
mod models;
mod node;
mod pairing;
mod people;
mod places;
mod presence;
mod previews;
mod recover;
mod rotation;
mod search;
mod search_status;
mod shards;
mod sync;
mod ticket;
mod transfers;
pub mod update;
pub mod web_live;
pub mod web_share;

pub use activity::Activity;
pub use addr::{from_endpoint_addr, from_iroh_id, to_endpoint_addr, to_iroh_id};
pub use blobs::{BlobHandle, BlobStore, ImportMode, Protection};
pub use compact::CompactReport;
pub use config::{NodeConfig, RelaySetting};
pub use doc_digest::{doc_digest, DOC_DIGEST_ALPN};
pub use ext::{ConnHandler, ExtSlot};
pub use fetcher::{FetchSource, FetcherConfig};
pub use freeup::{FreeReport, CONFIRMATION_TTL_MS};
pub use holdings::{DeviceStorage, DEFAULT_REDUNDANCY, REDUNDANCY_KEY};
pub use identity::{Membership, RECOVERY_KIT_SAVED_KEY};
pub use import::{ImportOptions, ImportReport};
pub use indexer::{
    owners_of, Always, FaceModels, IndexGate, IndexProgress, IndexerConfig, PEOPLE_SETTING,
};
pub use keys::load_or_create_secret;
pub use library_node::LibraryNode;
pub use maintenance::MaintenanceConfig;
pub use model_download::{download_model, download_model_from};
pub use models::ModelSources;
pub use node::{
    classify_addrs, classify_addrs_from, FetchReport, Node, NodeEvent, NodeEvents, PathKind,
    PeerAddr, PeerInfo, ALBUM_ALPN, HAS_ALPN, LORO_ALPN, PAIR_ALPN, PING_ALPN, RECOVER_ALPN,
    ROTATE_ALPN, WEBVIEW_ALPN,
};
pub use pairing::{DecideFn, PairGrant, PairTicket, PairingCode};
pub use people::{PeopleView, ME_SETTING};
pub use presence::{DeviceStatus, PresenceConfig};
pub use recover::{PeerLink, RecoverGrant, RecoveryDecideFn, RecoveryRequest};
pub use search::{SearchOptions, SearchResult};
pub use search_status::{IndexerInfo, SearchStatus};
pub use shards::{winning_rows, ShardView, PEOPLE_SNAPSHOT_KEY, SHARD_MIME};
pub use sync::{SyncConfig, SyncPhase};
pub use ticket::{LibraryTicket, TicketError};
pub use transfers::{Transfer, TransferSnapshot};
pub use web_live::LiveReport;
pub use web_share::{
    CloudBackupReport, CloudPolicy, CloudRestoreReport, RefreshReport, WebShare, WebShareOptions,
    CLOUD_SECRET_KEY, CLOUD_URL_KEY,
};
