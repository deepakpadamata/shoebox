//! Authenticity of replicated records (M1-16).
//!
//! Every record value in the Loro map is `0x00 ++ payload` (unsigned: no key available) or
//! `0x01 ++ signature(64) ++ payload`, where the signature covers the library id, the full map key
//! and the payload with the author's device key. The map key starts with the author's 8-byte id
//! prefix, so an importer resolves the author among the certified devices and checks both the
//! signature and that the author's role may write that kind of record.

use shoebox_core::identity::{verify_signature, KeyPair};
use shoebox_core::store::LibraryId;

const DOMAIN: &[u8] = b"shoebox/loro-entry/v1\0";

fn message(library: LibraryId, key: &str, payload: &[u8]) -> Vec<u8> {
    let mut m = DOMAIN.to_vec();
    m.extend_from_slice(&library.0);
    m.extend_from_slice(&(key.len() as u32).to_le_bytes());
    m.extend_from_slice(key.as_bytes());
    m.extend_from_slice(payload);
    m
}

/// Wrap `payload` for storage under `key`.
pub(crate) fn seal(
    signer: Option<&KeyPair>,
    library: LibraryId,
    key: &str,
    payload: &[u8],
) -> Vec<u8> {
    match signer {
        Some(k) => {
            let sig = k.sign(&message(library, key, payload));
            let mut out = Vec::with_capacity(1 + sig.len() + payload.len());
            out.push(1);
            out.extend_from_slice(&sig);
            out.extend_from_slice(payload);
            out
        }
        None => {
            let mut out = Vec::with_capacity(1 + payload.len());
            out.push(0);
            out.extend_from_slice(payload);
            out
        }
    }
}

pub(crate) struct Opened<'a> {
    pub signature: Option<&'a [u8]>,
    pub payload: &'a [u8],
}

pub(crate) fn open(value: &[u8]) -> Option<Opened<'_>> {
    match value.first()? {
        0 => Some(Opened {
            signature: None,
            payload: &value[1..],
        }),
        1 if value.len() >= 65 => Some(Opened {
            signature: Some(&value[1..65]),
            payload: &value[65..],
        }),
        _ => None,
    }
}

pub(crate) fn verify(
    author: &[u8; 32],
    library: LibraryId,
    key: &str,
    opened: &Opened<'_>,
) -> bool {
    match opened.signature {
        Some(sig) => verify_signature(author, &message(library, key, opened.payload), sig),
        None => false,
    }
}
