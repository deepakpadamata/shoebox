//! Dev-only library ticket: a string that carries a library capability and where to reach a
//! device that already has it. M2 replaces this with the signed pairing flow.
//!
//! Format: `shoebox-library1` + lowercase hex of `postcard(body) ++ checksum` where the checksum
//! is the first 4 bytes of `blake3("shoebox-library1" ++ postcard(body))`.

use serde::{Deserialize, Serialize};
use shoebox_core::store::{LibraryCapability, PeerAddr};
use std::fmt;
use std::str::FromStr;

const PREFIX: &str = "shoebox-library1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryTicket {
    pub cap: LibraryCapability,
    /// Devices to sync with first (may be empty if the ticket is shared out of band).
    pub peers: Vec<PeerAddr>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TicketError {
    #[error("not a Shoebox library ticket (expected the `{PREFIX}` prefix)")]
    BadPrefix,
    #[error("ticket is not valid hex")]
    BadEncoding,
    #[error("ticket is truncated or was altered (checksum mismatch)")]
    BadChecksum,
    #[error("ticket contents are malformed: {0}")]
    Malformed(String),
}

fn checksum(body: &[u8]) -> [u8; 4] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(PREFIX.as_bytes());
    hasher.update(body);
    let hash = hasher.finalize();
    hash.as_bytes()[..4].try_into().expect("4 bytes")
}

impl fmt::Display for LibraryTicket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut body = postcard::to_stdvec(self).expect("ticket encodes");
        let sum = checksum(&body);
        body.extend(sum);
        write!(f, "{PREFIX}{}", hex::encode(body))
    }
}

impl FromStr for LibraryTicket {
    type Err = TicketError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let rest = s
            .trim()
            .strip_prefix(PREFIX)
            .ok_or(TicketError::BadPrefix)?;
        let mut bytes = hex::decode(rest).map_err(|_| TicketError::BadEncoding)?;
        if bytes.len() < 4 {
            return Err(TicketError::BadChecksum);
        }
        let sum: [u8; 4] = bytes
            .split_off(bytes.len() - 4)
            .try_into()
            .expect("4 bytes");
        if checksum(&bytes) != sum {
            return Err(TicketError::BadChecksum);
        }
        postcard::from_bytes(&bytes).map_err(|e| TicketError::Malformed(e.to_string()))
    }
}
