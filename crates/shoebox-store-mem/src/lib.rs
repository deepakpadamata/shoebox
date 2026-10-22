//! In-memory [`StoreBackend`] (M0-06): a complete implementation of the store traits on top of
//! [`shoebox_core::merge`], with a simulated replica-to-replica `sync_with` for tests.
//!
//! Replicas live in a shared [`MemNetwork`]; `sync_with(peer)` looks the peer's replica of the
//! same library/album up by device id and exchanges state both ways.

mod album;
mod library;
mod net;

#[cfg(feature = "testing")]
pub mod harness;

pub use album::MemAlbum;
pub use library::MemLibrary;
pub use net::{MemBackend, MemNetwork};
