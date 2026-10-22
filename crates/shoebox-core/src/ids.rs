//! Identifier and scalar newtypes.

use serde::{Deserialize, Serialize};
use std::fmt;

/// A BLAKE3 hash (32 bytes). Used for blob hashes; a [`PhotoId`](crate::model::PhotoId) is the
/// hash of the original bytes.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Blake3Hash(pub [u8; 32]);

/// A device identity: the 32-byte public key of its iroh endpoint.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EndpointId(pub [u8; 32]);

/// Unix time in milliseconds. Signed so that pre-1970 EXIF dates are representable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Timestamp(pub i64);

macro_rules! hex_debug {
    ($t:ty, $name:literal) => {
        impl fmt::Display for $t {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&hex::encode(self.0))
            }
        }
        impl fmt::Debug for $t {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", $name, &hex::encode(self.0)[..8])
            }
        }
    };
}
hex_debug!(Blake3Hash, "Hash");
hex_debug!(EndpointId, "Endpoint");

impl Blake3Hash {
    /// Hash arbitrary bytes. Test/dev convenience; the media pipeline hashes streams itself.
    pub fn of_bytes(bytes: &[u8]) -> Self {
        Self(*blake3::hash(bytes).as_bytes())
    }
}
