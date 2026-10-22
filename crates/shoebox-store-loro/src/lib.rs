//! `LibraryStore` on a Loro CRDT document (M1-14 spike, promoted to production in M1-16).
//!
//! The merge rules, indexes and queries are the shared `core::library_state::LibraryState`.
//! Every replicated record is a binary value in one Loro map keyed `<author prefix>/<record key>`
//! (the same key layout as the iroh-docs backend), so each device only ever writes its own
//! keys and cross-device conflicts are resolved by `core::merge` on read.
//!
//! * **Transport:** `shoebox/loro/0` over iroh ([`wire`]): version-vector exchange both ways,
//!   repeated until both replicas hold the same operations; local changes are pushed by the
//!   node's sync manager (`LibraryStore::replicates_by_push`).
//! * **Persistence:** snapshot + framed update log, compaction, torn-tail recovery ([`persist`]).
//! * **Authenticity:** record values are signed with the author's device key; importers check the
//!   signature and the author's certificate and role ([`auth`]).
//! * **Incremental import:** a sync folds only the keys Loro reports as changed.
//!
//! Shared albums are not here; they stay on iroh-docs (see the `shoebox-store` crate).

/// First byte of the opaque `LibraryCapability::backend` payload.
pub const CAPABILITY_VERSION: u8 = 1;

#[cfg(feature = "loro")]
mod auth;
#[cfg(feature = "loro")]
mod backend;
#[cfg(all(feature = "loro", feature = "testing"))]
pub mod harness;
#[cfg(feature = "loro")]
mod library;
#[cfg(feature = "loro")]
mod net;
#[cfg(feature = "loro")]
mod persist;
#[cfg(feature = "loro")]
mod wire;

#[cfg(feature = "loro")]
pub use backend::{LoroBackend, LoroNetwork, UnsupportedAlbum};
#[cfg(feature = "loro")]
pub use library::LoroLibrary;
