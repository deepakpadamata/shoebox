//! Key layout and value encoding for the iroh-docs backend.
//!
//! Library namespace (one per library):
//!
//! ```text
//! photos/<photoId hex>                 → StateEntry::Photo
//! albums/<albumId hex>                 → StateEntry::Album
//! devices/<deviceId hex>               → StateEntry::Device
//! admins/<deviceId hex>                → StateEntry::Admin
//! holdings/<deviceId hex>/<blob hex>   → StateEntry::Holding   (written only by that device)
//! purged/<photoId hex>                 → StateEntry::Purged
//! ```
//!
//! Shared-album namespace (one per album):
//!
//! ```text
//! meta                                 → AlbumEntry::Meta
//! items/<photoId hex>                  → AlbumEntry::Item
//! members/<memberKey hex>              → AlbumEntry::Member
//! events/<hlc: ms:016x counter:08x device hex> → AlbumEntry::Event
//! ```
//!
//! Values are `[version byte = 1] ++ postcard(entry)`. The key is derived from the record, and a
//! stored value is only accepted if its key names the record it contains, so a peer cannot park
//! one record under another's key.

use serde::{de::DeserializeOwned, Serialize};
use shoebox_core::album_state::{AlbumEntry, AlbumTouched};
use shoebox_core::hlc::Hlc;
use shoebox_core::library_state::{StateEntry, Touched};

pub const VERSION: u8 = 1;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CodecError {
    #[error("empty value")]
    Empty,
    #[error("unsupported value version {0}")]
    UnsupportedVersion(u8),
    #[error("malformed value: {0}")]
    Malformed(String),
    #[error("value does not belong under key `{0}`")]
    KeyMismatch(String),
}

fn hex32(b: &[u8; 32]) -> String {
    hex::encode(b)
}

fn hlc_key(h: &Hlc) -> String {
    format!(
        "{:016x}{:08x}{}",
        h.physical_ms,
        h.counter,
        hex::encode(h.device.0)
    )
}

pub fn library_key(e: &StateEntry) -> String {
    match e {
        StateEntry::Photo(p) => format!("photos/{}", hex32(&p.id.0)),
        StateEntry::Album(a) => format!("albums/{}", hex::encode(a.id.0)),
        StateEntry::Device(c) => format!("devices/{}", hex32(&c.device.0)),
        StateEntry::Admin(c) => format!("admins/{}", hex32(&c.admin.0)),
        StateEntry::Holding { device, blob, .. } => {
            format!("holdings/{}/{}", hex32(&device.0), hex32(&blob.0))
        }
        StateEntry::Purged { id, .. } => format!("purged/{}", hex32(&id.0)),
    }
}

/// Key of the record a [`Touched`] refers to (even when the record no longer exists).
pub fn touched_key(t: &Touched) -> String {
    match t {
        Touched::Photo(id) => format!("photos/{}", hex32(&id.0)),
        Touched::Album(id) => format!("albums/{}", hex::encode(id.0)),
        Touched::Device(id) => format!("devices/{}", hex32(&id.0)),
        Touched::Admin(id) => format!("admins/{}", hex32(&id.0)),
        Touched::Holding(device, blob) => {
            format!("holdings/{}/{}", hex32(&device.0), hex32(&blob.0))
        }
        Touched::Purged(id) => format!("purged/{}", hex32(&id.0)),
    }
}

/// Key of the shared-album record a [`AlbumTouched`] refers to.
pub fn album_touched_key(t: &AlbumTouched) -> String {
    match t {
        AlbumTouched::Item(id) => format!("items/{}", hex32(&id.0)),
        AlbumTouched::Member(k) => format!("members/{}", hex32(&k.0)),
        AlbumTouched::Event(h) => format!("events/{}", hlc_key(h)),
    }
}

pub fn album_key(e: &AlbumEntry) -> String {
    match e {
        AlbumEntry::Meta(_) => "meta".to_string(),
        AlbumEntry::Item(i) => format!("items/{}", hex32(&i.id.0)),
        AlbumEntry::Member { member, .. } => format!("members/{}", hex32(&member.key.0)),
        AlbumEntry::Event(ev) => format!("events/{}", hlc_key(&ev.hlc)),
    }
}

fn encode<T: Serialize>(value: &T) -> Vec<u8> {
    let mut out = vec![VERSION];
    out.extend(postcard::to_stdvec(value).expect("postcard encoding cannot fail"));
    out
}

fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, CodecError> {
    let (&version, body) = bytes.split_first().ok_or(CodecError::Empty)?;
    if version != VERSION {
        return Err(CodecError::UnsupportedVersion(version));
    }
    postcard::from_bytes(body).map_err(|e| CodecError::Malformed(e.to_string()))
}

pub fn encode_library_entry(e: &StateEntry) -> Vec<u8> {
    encode(e)
}

pub fn decode_library_entry(key: &str, bytes: &[u8]) -> Result<StateEntry, CodecError> {
    let entry: StateEntry = decode(bytes)?;
    if library_key(&entry) != key {
        return Err(CodecError::KeyMismatch(key.to_string()));
    }
    Ok(entry)
}

pub fn encode_album_entry(e: &AlbumEntry) -> Vec<u8> {
    encode(e)
}

pub fn decode_album_entry(key: &str, bytes: &[u8]) -> Result<AlbumEntry, CodecError> {
    let entry: AlbumEntry = decode(bytes)?;
    if album_key(&entry) != key {
        return Err(CodecError::KeyMismatch(key.to_string()));
    }
    Ok(entry)
}
