//! The share manifest for web links (W1-03): what a link's viewer page trusts. It names the album's photos
//! and carries each blob's key, so it is encrypted with the share key (which lives only in the link's
//! `#fragment`) and signed by a device key of the sharer (whose public key is in the link too).
//!
//! Wire format of a sealed manifest, all integers little endian:
//!
//! ```text
//! "SBXS" | format u8 | share_id [16] | manifest_version u32 | nonce [24] | ct_len u32 | ciphertext | signature [64]
//! ```
//!
//! The ciphertext is XChaCha20-Poly1305 over the postcard-encoded [`Manifest`], with everything before
//! `ct_len` as associated data. The signature covers every byte before it. The server sees the header
//! (it needs `share_id` and the version) and nothing else.

use crate::cloud::BlobKey;
use crate::identity::{verify_signature, KeyPair};
use crate::ids::{Blake3Hash, Timestamp};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use std::fmt;

const MAGIC: &[u8; 4] = b"SBXS";
/// The only format this build writes and reads.
pub const FORMAT: u8 = 1;
const HEADER: usize = 4 + 1 + 16 + 4 + 24;
const SIG: usize = 64;
const KEY_CONTEXT: &str = "shoebox/share-manifest-key/v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ShareError {
    #[error("that is not a Shoebox share")]
    NotAShare,
    #[error("the share is cut short")]
    Truncated,
    #[error("this share was made by a newer Shoebox (format {found}); update this page")]
    NewerFormat { found: u8 },
    #[error("the share was not signed by the person who sent the link")]
    BadSignature,
    #[error("the link's key does not open this share")]
    WrongKey,
    #[error("the share is damaged")]
    Malformed,
}

/// The key that opens a share's manifest. Random, per share; it goes in the link, never to a server.
#[derive(Clone, PartialEq, Eq)]
pub struct ShareKey(pub [u8; 32]);

impl ShareKey {
    pub fn generate() -> Self {
        let mut k = [0u8; 32];
        getrandom::fill(&mut k).expect("operating system randomness");
        Self(k)
    }
    fn cipher(&self) -> XChaCha20Poly1305 {
        XChaCha20Poly1305::new((&blake3::derive_key(KEY_CONTEXT, &self.0)).into())
    }
}

/// The key a share's manifest is signed with, derived from the share key, so that any member device can sign a
/// refreshed manifest and anyone with the link can check it. (A device key could not do both: the link would name
/// one device, and a keeper's refresh would fail the check.) Since the same link opens the manifest, this signature
/// is an integrity check, not proof of who made the share.
impl ShareKey {
    pub fn signer(&self) -> KeyPair {
        KeyPair::from_seed(blake3::derive_key("shoebox/share-signer/v1", &self.0))
    }
}

impl fmt::Debug for ShareKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ShareKey(..)")
    }
}

/// One stored blob and the key that opens it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct CloudBlob {
    pub cid: Blake3Hash,
    pub key: [u8; 32],
    /// Ciphertext length, so the page can show sizes and plan downloads.
    pub size: u64,
}

impl CloudBlob {
    pub fn key(&self) -> BlobKey {
        BlobKey(self.key)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum ItemKind {
    Photo,
    Video,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Item {
    pub kind: ItemKind,
    pub taken: Timestamp,
    pub width: u32,
    pub height: u32,
    /// EXIF orientation, 1 to 8 (1 = upright).
    pub orientation: u8,
    pub thumb: CloudBlob,
    pub preview: CloudBlob,
    /// Present only when the sharer allowed full-size downloads.
    pub original: Option<CloudBlob>,
    /// The short video of a Live or motion photo.
    pub motion: Option<CloudBlob>,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub album_name: String,
    pub items: Vec<Item>,
    /// After this the server stops serving the share; shown by the page too.
    pub expires: Option<Timestamp>,
    /// Reserved for guests adding photos (W1-11): the key a browser seals uploads to.
    pub inbox_key: Option<[u8; 32]>,
}

/// What the server and the viewer both read without a key.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Header {
    pub share_id: [u8; 16],
    pub manifest_version: u32,
}

fn parse_header(bytes: &[u8]) -> Result<Header, ShareError> {
    if bytes.len() < 5 || &bytes[..4] != MAGIC {
        return Err(ShareError::NotAShare);
    }
    if bytes[4] > FORMAT {
        return Err(ShareError::NewerFormat { found: bytes[4] });
    }
    if bytes[4] != FORMAT || bytes.len() < HEADER + 4 + SIG {
        return Err(ShareError::Truncated);
    }
    Ok(Header {
        share_id: bytes[5..21].try_into().unwrap(),
        manifest_version: u32::from_le_bytes(bytes[21..25].try_into().unwrap()),
    })
}

/// Read a sealed manifest's header without any key (what the cloud does to index a share).
pub fn read_header(sealed: &[u8]) -> Result<Header, ShareError> {
    parse_header(sealed)
}

/// Encrypt and sign a manifest with a random nonce.
pub fn seal(
    manifest: &Manifest,
    share_id: [u8; 16],
    manifest_version: u32,
    key: &ShareKey,
    signer: &KeyPair,
) -> Vec<u8> {
    let mut nonce = [0u8; 24];
    getrandom::fill(&mut nonce).expect("operating system randomness");
    seal_with_nonce(manifest, share_id, manifest_version, key, signer, nonce)
}

/// [`seal`] with the nonce given (golden tests). Reusing a nonce under one key breaks the encryption.
pub fn seal_with_nonce(
    manifest: &Manifest,
    share_id: [u8; 16],
    manifest_version: u32,
    key: &ShareKey,
    signer: &KeyPair,
    nonce: [u8; 24],
) -> Vec<u8> {
    let body = postcard::to_stdvec(manifest).expect("postcard encoding of a manifest cannot fail");
    let mut out = Vec::with_capacity(HEADER + 4 + body.len() + 16 + SIG);
    out.extend_from_slice(MAGIC);
    out.push(FORMAT);
    out.extend_from_slice(&share_id);
    out.extend_from_slice(&manifest_version.to_le_bytes());
    out.extend_from_slice(&nonce);
    let ct = key
        .cipher()
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &body,
                aad: &out,
            },
        )
        .expect("sealing a manifest cannot fail");
    out.extend_from_slice(&(ct.len() as u32).to_le_bytes());
    out.extend_from_slice(&ct);
    let sig = signer.sign(&out);
    out.extend_from_slice(&sig);
    out
}

/// Check, decrypt and decode a sealed manifest. `signer` is the sharer's public key from the link.
///
/// The failures are told apart: not a share, cut short, newer format, signed by someone else, a key that
/// does not open it, and damaged contents.
pub fn open(
    sealed: &[u8],
    key: &ShareKey,
    signer: &[u8; 32],
) -> Result<(Header, Manifest), ShareError> {
    let header = parse_header(sealed)?;
    let ct_len = u32::from_le_bytes(sealed[HEADER..HEADER + 4].try_into().unwrap()) as usize;
    if sealed.len() != HEADER + 4 + ct_len + SIG {
        return Err(ShareError::Truncated);
    }
    let (signed, sig) = sealed.split_at(sealed.len() - SIG);
    if !verify_signature(signer, signed, sig) {
        return Err(ShareError::BadSignature);
    }
    let body = key
        .cipher()
        .decrypt(
            XNonce::from_slice(&sealed[HEADER - 24..HEADER]),
            Payload {
                msg: &sealed[HEADER + 4..HEADER + 4 + ct_len],
                aad: &sealed[..HEADER],
            },
        )
        .map_err(|_| ShareError::WrongKey)?;
    let manifest = postcard::from_bytes(&body).map_err(|_| ShareError::Malformed)?;
    Ok((header, manifest))
}

/// What an owner uploads to put a share on Shoebox Cloud (`PUT /v1/shares/{share_id}`): the sealed manifest, the
/// blob ids the share may serve (the service serves nothing else through it), and when it stops.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct ShareUpload {
    pub manifest: Vec<u8>,
    pub cids: Vec<[u8; 32]>,
    pub expires_ms: Option<i64>,
}

/// How a link opens a share. The fragment is `<mode>.<locator>.<key>`, the last two in unpadded URL-safe base64;
/// a browser never sends it to any server.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ShareLink {
    pub mode: LinkMode,
    /// Cloud: the 16-byte share id. (Live links carry their ticket here; W1-08.)
    pub locator: Vec<u8>,
    pub key: ShareKey,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LinkMode {
    Cloud,
    Live,
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn b64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..=c.len() {
            out.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    out
}

fn b64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for ch in text.bytes() {
        let v = B64.iter().position(|&b| b == ch)? as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    // Unused low bits must be zero, so one value has one spelling.
    (acc == 0).then_some(out)
}

impl ShareLink {
    pub fn cloud(share_id: [u8; 16], key: ShareKey) -> Self {
        Self {
            mode: LinkMode::Cloud,
            locator: share_id.to_vec(),
            key,
        }
    }

    /// The text after the `#`.
    pub fn to_fragment(&self) -> String {
        let mode = match self.mode {
            LinkMode::Cloud => "c",
            LinkMode::Live => "l",
        };
        format!(
            "{mode}.{}.{}",
            b64_encode(&self.locator),
            b64_encode(&self.key.0)
        )
    }

    /// Read the text after the `#` (a leading `#` is fine).
    pub fn parse(fragment: &str) -> Option<Self> {
        let mut parts = fragment.trim_start_matches('#').split('.');
        let mode = match parts.next()? {
            "c" => LinkMode::Cloud,
            "l" => LinkMode::Live,
            _ => return None,
        };
        let locator = b64_decode(parts.next()?)?;
        let key: [u8; 32] = b64_decode(parts.next()?)?.try_into().ok()?;
        if parts.next().is_some() || locator.is_empty() {
            return None;
        }
        if mode == LinkMode::Cloud && locator.len() != 16 {
            return None;
        }
        Some(Self {
            mode,
            locator,
            key: ShareKey(key),
        })
    }

    pub fn share_id(&self) -> Option<[u8; 16]> {
        (self.mode == LinkMode::Cloud).then(|| self.locator.clone().try_into().ok())?
    }
}
