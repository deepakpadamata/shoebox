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
    /// An index shard (S1-06). Appended: postcard encodes variants by index.
    Index,
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

/// Identity of a search model: BLAKE3 over its weight files and preprocessing description (see
/// `docs/notes/search-models.md`). Results from different models are never compared.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct ModelId(pub [u8; 32]);

/// What an index shard holds (epic S1 §3.1). Appended to only: the byte is in shard headers.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub enum ShardKind {
    /// Picture embeddings, 8-bit with a scale per photo.
    Vec8,
    /// The same embeddings as sign bits.
    Vec1,
    /// Labels with scores and text read from the photo.
    Text,
    /// Detected faces with their embeddings.
    Faces,
}

impl ShardKind {
    pub const ALL: [ShardKind; 4] = [Self::Vec8, Self::Vec1, Self::Text, Self::Faces];

    pub fn byte(self) -> u8 {
        self as u8
    }

    pub fn from_byte(b: u8) -> Option<Self> {
        Self::ALL.get(usize::from(b)).copied()
    }
}

/// The library's record of one index shard blob (`index/<blob hash>`). Immutable: the blob it
/// names is content addressed, and `replaces` is copied from inside it.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct IndexShard {
    pub blob: BlobRef,
    pub model: ModelId,
    pub kind: ShardKind,
    /// Photos in the shard.
    pub count: u32,
    /// Shards of the same model and kind this one supersedes (from the blob's header).
    pub replaces: Vec<Blake3Hash>,
    pub made_by: EndpointId,
    pub hlc: Hlc,
}

/// What a device can index and how fast (`indexers/<device>`), written only by that device. The
/// one input to sharing indexing work, with the device list.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct IndexerStatus {
    pub device: EndpointId,
    /// Models this build can index with.
    pub models: Vec<ModelId>,
    pub kinds: Vec<ShardKind>,
    /// Measured and rounded; 0 = not indexing.
    pub photos_per_min: u32,
    /// A keeper, or a desktop set to index while plugged in.
    pub always_on: bool,
    pub hlc: Hlc,
}

/// Identity of a person (random 16 bytes, chosen by whoever named them first).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct PersonId(pub [u8; 16]);

/// A person people have named (`persons/<id>`, S1-18). Every field is last-writer-wins.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Person {
    pub id: PersonId,
    pub name: Lww<String>,
    /// Left out of suggestions and the People screen.
    pub hidden: Lww<bool>,
    /// Merged into another person (followed to its end; within a cycle the lowest id is it).
    pub merged_into: Lww<Option<PersonId>>,
}

impl Person {
    /// Field by field, the later write wins.
    pub fn merge(&self, other: &Person) -> Option<Person> {
        (self.id == other.id).then(|| Person {
            id: self.id,
            name: self.name.merge(&other.name),
            hidden: self.hidden.merge(&other.hidden),
            merged_into: self.merged_into.merge(&other.merged_into),
        })
    }
}

/// What people said about one face (`facelabels/<photo>/<face key>`, S1-18): who it is
/// (last writer wins), and who it is not (an add-wins set). `face` is the face's box on a 1/64
/// grid of the photo (`shoebox_search::faces::FaceKey`).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct FaceLabel {
    pub photo: PhotoId,
    pub face: [u8; 4],
    pub is: Lww<Option<PersonId>>,
    pub is_not: OrSet<PersonId>,
}

impl FaceLabel {
    pub fn merge(&self, other: &FaceLabel) -> Option<FaceLabel> {
        (self.photo == other.photo && self.face == other.face).then(|| FaceLabel {
            photo: self.photo,
            face: self.face,
            is: self.is.merge(&other.is),
            is_not: self.is_not.merge(&other.is_not),
        })
    }
}
