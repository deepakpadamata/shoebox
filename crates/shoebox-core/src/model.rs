//! Domain records shared by every crate. Pure data: merge behaviour lives in [`crate::merge`],
//! byte encoding in [`crate::codec`].

use crate::hlc::Hlc;
use crate::ids::{Blake3Hash, EndpointId, Timestamp};
use crate::merge::{Lww, OrSet};
use serde::{Deserialize, Serialize};

/// Identity of a photo: the BLAKE3 hash of the original bytes. Importing the same file on two
/// devices converges on one entry (F2.4).
pub type PhotoId = Blake3Hash;

/// Identity of an album (random 16 bytes, chosen by the creator).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct AlbumId(pub [u8; 16]);

/// A device's storage role (F4.3).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub enum Role {
    /// Keeps every original: the backstop.
    Keeper,
    /// Keeps what is recent, what you favourite and what you pin; the rest lives on keepers.
    /// (Called `Phone` until 0.1.0: older config files still say so.)
    #[serde(alias = "Phone")]
    Companion,
    /// Previews only.
    Viewer,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub enum BlobKind {
    Original,
    Preview,
    Thumb,
}

/// A reference to a blob in iroh-blobs.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct BlobRef {
    pub hash: Blake3Hash,
    pub size: u64,
    pub mime: String,
}

/// GPS position in degrees * 1e7 (integer, so equality and ordering are exact).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct Gps {
    pub lat_e7: i32,
    pub lon_e7: i32,
}

/// The EXIF fields the app needs (info panel, search); the full EXIF stays in the original.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default, Serialize, Deserialize)]
pub struct ExifSummary {
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub lens: Option<String>,
    pub width: u32,
    pub height: u32,
    pub gps: Option<Gps>,
}

/// One photo in the library. Immutable fields come from the original; `favorite` and `deleted_at`
/// are LWW registers (see [`crate::merge`]).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct PhotoRecord {
    pub id: PhotoId,
    pub original: BlobRef,
    pub preview: BlobRef,
    pub thumb: BlobRef,
    pub taken_at: Timestamp,
    pub exif: ExifSummary,
    pub imported_by: EndpointId,
    pub favorite: Lww<bool>,
    /// `Some(t)` = in Recently Deleted since `t`; restoring is an explicit write of `None`.
    pub deleted_at: Lww<Option<Timestamp>>,
}

/// A personal album: LWW name/cover plus an add-wins set of members.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct AlbumMeta {
    pub id: AlbumId,
    pub name: Lww<String>,
    pub cover: Lww<Option<PhotoId>>,
    pub items: OrSet<PhotoId>,
}

/// An item in a shared album. `shared_hash` is set only when the contributor chose to strip
/// location (decision #8): then `shared_hash` is the stripped copy, `original` is untouched.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct SharedItem {
    pub id: PhotoId,
    pub original: BlobRef,
    pub shared_hash: Option<Blake3Hash>,
    pub preview: BlobRef,
    pub thumb: BlobRef,
    pub added_by: EndpointId,
    pub added_at: Timestamp,
    pub taken_at: Timestamp,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub enum MemberRole {
    Owner,
    Contributor,
    Viewer,
}

/// A shared-album member, keyed by an endpoint-derived member key.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Member {
    pub key: EndpointId,
    pub display_name: String,
    pub role: MemberRole,
}

/// Library-root-signed statement that `device` belongs to the library with `role`.
/// `signature` is empty until M2-01 wires up real signing/verification.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct DeviceCert {
    pub device: EndpointId,
    pub role: Role,
    pub name: String,
    pub added_by: EndpointId,
    pub hlc: Hlc,
    pub revoked: bool,
    pub signature: Vec<u8>,
}

/// Root-signed delegation (decision #18): an admin device may sign [`DeviceCert`]s.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct AdminCert {
    pub admin: EndpointId,
    pub hlc: Hlc,
    /// The delegation stops being valid after this time (`None` = never expires).
    pub expires_at: Option<Timestamp>,
    pub signature: Vec<u8>,
}

/// "Device `device` holds blob `blob` of kind `kind`" — written only by `device` itself.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Holding {
    pub device: EndpointId,
    pub blob: Blake3Hash,
    pub kind: BlobKind,
    pub since: Hlc,
}
