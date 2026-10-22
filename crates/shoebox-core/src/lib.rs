//! Shoebox core: the pure domain layer shared by every crate.
//!
//! This crate deliberately has **no async runtime and no iroh dependency** (M0-04), which keeps
//! the domain logic unit-testable and the storage backend swappable.

pub mod album_state;
pub mod clock;
pub mod codec;
#[cfg(feature = "testing")]
pub mod conformance;
pub mod entry_codec;
pub mod hlc;
pub mod identity;
pub mod ids;
pub mod library_state;
pub mod merge;
pub mod model;
pub mod policy;
pub mod recovery;
pub mod store;

pub use hlc::{Hlc, HlcClock};
pub use ids::{Blake3Hash, EndpointId, Timestamp};
