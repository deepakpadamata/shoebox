//! SPIKE (M1-14): a `LibraryStore` backed by a Loro CRDT document.
//!
//! What it is: the shared [`shoebox_core::library_state::LibraryState`] (merge rules, indexes,
//! queries) with every replicated record stored as a binary value in one Loro map, keyed
//! `<author>/<record key>` exactly like the iroh-docs layout. Replicas converge by exchanging
//! Loro updates (`export updates since the peer's version vector` / `import`). Because each
//! device writes only under its own author prefix there are no concurrent writes to one Loro key;
//! cross-device conflicts are resolved by `core::merge` on read, as in the other backends.
//!
//! What it is *not* (spike scope): there is no network transport (sync between replicas is
//! simulated in-process through a [`LoroNetwork`], using real Loro export/import), no gossip
//! announcements, no persistence to disk beyond [`LoroLibrary::export_snapshot`], and shared
//! albums are not implemented. See `docs/notes/loro-spike.md` for the findings.

#[cfg(feature = "loro")]
mod backend;
#[cfg(all(feature = "loro", feature = "testing"))]
pub mod harness;
#[cfg(feature = "loro")]
mod library;

#[cfg(feature = "loro")]
pub use backend::{LoroBackend, LoroNetwork, UnsupportedAlbum};
#[cfg(feature = "loro")]
pub use library::LoroLibrary;
