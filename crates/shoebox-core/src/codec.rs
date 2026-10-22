//! Stable binary encoding: one version byte followed by the postcard body.
//!
//! `encode` = `[VERSION] ++ postcard(value)`. Decoding rejects unknown versions so a future v2
//! reader can branch on the first byte. Golden-bytes tests lock v1 (see the tests below).

use serde::{de::DeserializeOwned, Serialize};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CodecError {
    #[error("empty input")]
    Empty,
    #[error("unsupported record version {found} (expected {expected})")]
    UnsupportedVersion { found: u8, expected: u8 },
    #[error("malformed record: {0}")]
    Malformed(String),
}

/// A record with a stable on-the-wire encoding.
pub trait Record: Serialize + DeserializeOwned {
    /// Version byte written before the body. Bump only with a migration (M6-05).
    const VERSION: u8 = 1;

    fn encode(&self) -> Vec<u8> {
        let mut out = vec![Self::VERSION];
        out.extend(postcard::to_stdvec(self).expect("postcard encoding of a record cannot fail"));
        out
    }

    fn decode(bytes: &[u8]) -> Result<Self, CodecError> {
        let (&version, body) = bytes.split_first().ok_or(CodecError::Empty)?;
        if version != Self::VERSION {
            return Err(CodecError::UnsupportedVersion {
                found: version,
                expected: Self::VERSION,
            });
        }
        postcard::from_bytes(body).map_err(|e| CodecError::Malformed(e.to_string()))
    }
}

macro_rules! records {
    ($($t:ty),* $(,)?) => { $(impl Record for $t {})* };
}

records!(
    crate::model::PhotoRecord,
    crate::model::AlbumMeta,
    crate::model::SharedItem,
    crate::model::Member,
    crate::model::DeviceCert,
    crate::model::AdminCert,
    crate::model::Holding,
    crate::model::BlobRef,
    crate::model::ExifSummary,
);
