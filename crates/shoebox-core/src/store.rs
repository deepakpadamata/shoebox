//! Storage boundary (M0-06): `LibraryStore`, `AlbumStore` and `StoreBackend`.
//!
//! The traits sit at the *domain* level (photos, albums, devices, holdings) so a CRDT backend can
//! model them natively instead of emulating a KV store. Every backend must reproduce the merge
//! semantics in [`crate::merge`]; the shared conformance suite (feature `testing`) enforces it.
//!
//! General contracts that hold for **every** method unless stated otherwise:
//! - *Convergence:* after two replicas exchange everything (`sync_with` in both directions), they
//!   expose identical state, regardless of operation order.
//! - *Idempotency:* replaying a write (same arguments, or merging the same record again) never
//!   changes state beyond the first application, except where a fresh HLC is issued (noted).
//! - Writes on a [`Capability::Read`] store fail with [`StoreError::ReadOnly`].

use crate::hlc::Hlc;
use crate::ids::{Blake3Hash, EndpointId, Timestamp};
use crate::model::{
    AdminCert, AlbumId, AlbumMeta, BlobKind, DeviceCert, Holding, Member, MemberRole, PhotoId,
    PhotoRecord, SharedItem,
};
use async_trait::async_trait;
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};

/// How long a soft-deleted photo stays in Recently Deleted before it may be purged.
pub const RETENTION_MS: i64 = 30 * 24 * 60 * 60 * 1000;

pub type Result<T> = std::result::Result<T, StoreError>;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum StoreError {
    #[error("not found")]
    NotFound,
    #[error("store is read-only")]
    ReadOnly,
    #[error("bad signature")]
    BadSignature,
    #[error("invite expired")]
    Expired,
    #[error("peer unreachable: {0}")]
    Unreachable(EndpointId),
    #[error("invalid request: {0}")]
    Invalid(String),
    #[error("backend error: {0}")]
    Backend(String),
}

/// Whether a handle may write.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Capability {
    Write,
    Read,
}

/// Backend-agnostic peer address (the iroh backend maps it to an `EndpointAddr`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerAddr {
    pub id: EndpointId,
    pub relay_url: Option<String>,
    pub direct_addrs: Vec<String>,
}

impl PeerAddr {
    pub fn id_only(id: EndpointId) -> Self {
        Self {
            id,
            relay_url: None,
            direct_addrs: vec![],
        }
    }
}

/// Half-open time range `[start, end)`; `None` is unbounded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TimeRange {
    pub start: Option<Timestamp>,
    pub end: Option<Timestamp>,
}

impl TimeRange {
    pub const ALL: TimeRange = TimeRange {
        start: None,
        end: None,
    };

    pub fn contains(&self, t: Timestamp) -> bool {
        self.start.is_none_or(|s| t >= s) && self.end.is_none_or(|e| t < e)
    }
}

/// Position after which a page continues. `taken_at` is used by time-ordered listings (newest
/// first, ties by id); id-ordered listings (`at_risk`, ascending) use `id` only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub taken_at: Timestamp,
    pub id: PhotoId,
}

impl Cursor {
    pub fn after_photo(p: &PhotoRecord) -> Self {
        Self {
            taken_at: p.taken_at,
            id: p.id,
        }
    }
    pub fn after_id(id: PhotoId) -> Self {
        Self {
            taken_at: Timestamp(0),
            id,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Page {
    pub after: Option<Cursor>,
    pub limit: usize,
}

impl Page {
    pub fn first(limit: usize) -> Self {
        Self { after: None, limit }
    }
    pub fn all() -> Self {
        Self {
            after: None,
            limit: usize::MAX,
        }
    }
}

/// Something observable changed (also emitted for changes that arrive by sync).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    PhotoUpserted(PhotoId),
    PhotoDeleted(PhotoId),
    AlbumChanged(AlbumId),
    DeviceListChanged,
    /// A library setting was written (locally or by a peer).
    SettingChanged(String),
    HoldingsChanged {
        device: EndpointId,
    },
    RemoteSyncProgress {
        peer: EndpointId,
        done: u64,
        total: Option<u64>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlbumSummary {
    pub id: AlbumId,
    pub name: String,
    pub cover: Option<PhotoId>,
    pub item_count: usize,
}

/// The current set of device certs and admin delegations.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedDeviceList {
    pub devices: Vec<DeviceCert>,
    pub admins: Vec<AdminCert>,
}

/// A signed change to the device registry. Signature verification is a stub until M2-01: the
/// store only rejects an *empty* signature ([`StoreError::BadSignature`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignedDeviceChange {
    /// Add, rename or revoke (`revoked: true`) a device; per device the cert with the later HLC wins.
    Device(DeviceCert),
    /// Delegate admin rights to a device.
    Admin(AdminCert),
}

/// What a device holds. Written only by that device (single writer per key).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HoldingsDelta {
    /// HLC of this write; orders it against earlier writes for the same `(device, blob)`.
    pub hlc: Hlc,
    pub added: Vec<(Blake3Hash, BlobKind)>,
    pub removed: Vec<Blake3Hash>,
}

/// Identity of the library's root signing key (decision #9/#18).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RootPublicKey(pub [u8; 32]);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct LibraryId(pub [u8; 16]);

/// Grants access to a library space.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryCapability {
    pub library: LibraryId,
    pub root: RootPublicKey,
    pub mode: Capability,
    /// Opaque, backend-specific access material (e.g. the iroh-docs namespace capability).
    /// Empty for backends that do not need any.
    pub backend: Vec<u8>,
}

impl LibraryCapability {
    /// The same library with `mode` set to read-only. **Does not strip `backend` material**; use
    /// [`StoreBackend::downgrade`] before handing a capability to someone who must not write.
    pub fn read_only(&self) -> Self {
        Self {
            mode: Capability::Read,
            ..self.clone()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedAlbumMeta {
    pub id: AlbumId,
    pub name: String,
    pub cover: Option<PhotoId>,
    pub created_by: EndpointId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Expiry {
    Never,
    At(Timestamp),
}

/// Bearer ticket to join a shared album.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InviteTicket {
    pub album: AlbumId,
    pub role: MemberRole,
    pub expires: Expiry,
    pub inviter: PeerAddr,
    pub secret: [u8; 16],
    /// Opaque, backend-specific join material (e.g. an iroh-docs ticket). Empty if unused.
    pub backend: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlbumEventKind {
    ItemsAdded { count: u32 },
    MemberJoined,
}

/// An entry in a shared album's activity feed (F6.4).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlbumEvent {
    pub hlc: Hlc,
    pub member: EndpointId,
    pub kind: AlbumEventKind,
}

/// A user's personal library: photos, personal albums, device registry and holdings.
#[async_trait]
pub trait LibraryStore: Send + Sync + 'static {
    // ----- photos -----

    /// Insert or merge a photo. **Idempotent by `rec.id`:** merged with any existing record using
    /// [`PhotoRecord::merge`]; re-upserting the same record changes nothing. Ignored if the photo
    /// was purged after the record's own tombstone HLC. Emits `PhotoUpserted`.
    async fn upsert_photo(&self, rec: PhotoRecord) -> Result<()>;

    /// The merged record, or `None` if unknown or purged. Includes soft-deleted photos.
    async fn photo(&self, id: &PhotoId) -> Result<Option<PhotoRecord>>;

    /// **Non-deleted** photos with `taken_at` in `range`, newest first (ties by id ascending),
    /// following `page`. Deterministic, so paging with [`Cursor::after_photo`] never repeats or
    /// skips an item absent concurrent writes.
    async fn photos_by_time(&self, range: TimeRange, page: Page) -> Result<Vec<PhotoRecord>>;

    /// Soft-deleted photos (Recently Deleted), most recently deleted first.
    async fn deleted_photos(&self) -> Result<Vec<PhotoRecord>>;

    /// LWW write of `favorite` with a fresh HLC (so a later call always wins locally).
    /// `NotFound` if the photo is unknown. Merges as an independent LWW register.
    async fn set_favorite(&self, id: &PhotoId, fav: bool) -> Result<()>;

    /// Soft-delete (LWW write of `deleted_at = now`). A no-op if the photo is already deleted (the
    /// original deletion time, and so the retention clock, is kept). Never undone by field edits,
    /// only by [`LibraryStore::restore_photo`]. `NotFound` if unknown. Emits `PhotoDeleted`.
    async fn mark_deleted(&self, id: &PhotoId) -> Result<()>;

    /// Explicit restore from Recently Deleted: an LWW write of `deleted_at = None` with a fresh
    /// HLC. Wins over any earlier delete, loses to a later one. Emits `PhotoUpserted`.
    async fn restore_photo(&self, id: &PhotoId) -> Result<()>;

    /// Permanently drop photos soft-deleted at least [`RETENTION_MS`] before `now`. Returns the
    /// purged ids (the caller garbage-collects their blobs). The purge replicates: a purge marker
    /// with an HLC is merged by max, and a record whose latest tombstone/edit HLC is not after the
    /// marker is dropped everywhere, so a stale replica cannot resurrect it. A *re-import* after
    /// the purge (later HLC) is a new record and survives. Idempotent: a second call returns `[]`.
    async fn purge_expired(&self, now: Timestamp) -> Result<Vec<PhotoId>>;

    // ----- personal albums -----

    /// Create or merge album metadata ([`AlbumMeta::merge`]); idempotent by `album.id`.
    async fn upsert_album(&self, album: AlbumMeta) -> Result<()>;

    /// Add items to an album's **add-wins set** (fresh unique tag per item). A concurrent remove
    /// of the same item on another replica does not remove it. `NotFound` if the album is unknown.
    async fn album_add(&self, album: &AlbumId, items: &[PhotoId]) -> Result<()>;

    /// Remove items by tombstoning only the add tags this replica has observed; unobserved
    /// concurrent adds survive. Removing an absent item is a no-op.
    async fn album_remove(&self, album: &AlbumId, items: &[PhotoId]) -> Result<()>;

    /// Present members of an album in id order, skipping soft-deleted or purged photos, following `page`
    /// (`page.after.id` is the last id seen).
    async fn album_items(&self, album: &AlbumId, page: Page) -> Result<Vec<PhotoId>>;

    async fn albums(&self) -> Result<Vec<AlbumSummary>>;

    // ----- devices (signed by the library root; the store persists and verifies) -----

    /// The merged device registry: per device the cert with the greatest HLC, sorted by device id.
    async fn device_list(&self) -> Result<SignedDeviceList>;

    /// Apply a signed change. **Idempotent** and order-independent (LWW per device by cert HLC).
    /// Rejects bad signatures with `BadSignature` (stubbed until M2-01: empty signature = bad).
    async fn apply_device_change(&self, change: SignedDeviceChange) -> Result<()>;

    // ----- holdings / redundancy -----

    /// Record what `device` holds. **Single writer:** `device` must be this store's own device
    /// (`Invalid` otherwise); remote devices' holdings arrive only through sync. Per
    /// `(device, blob)` the write with the later HLC wins, so replays are idempotent.
    async fn put_holdings(&self, device: EndpointId, delta: HoldingsDelta) -> Result<()>;

    /// Devices currently holding `blob` (any kind), ordered by device id.
    async fn holders(&self, blob: &Blake3Hash) -> Result<Vec<Holding>>;

    /// Non-deleted photos whose *original* is held by fewer than `min_copies` distinct devices,
    /// in id order after `page.after.id`.
    async fn at_risk(&self, min_copies: u8, page: Page) -> Result<Vec<PhotoId>>;

    // ----- settings -----

    /// A library-wide setting (e.g. `redundancy.target`), or `None` if never written. Keys are
    /// 1-64 chars of `[a-z0-9._-]`.
    async fn setting(&self, key: &str) -> Result<Option<Vec<u8>>>;

    /// Write a setting with a fresh HLC. **Last writer wins** per key, on every replica. An invalid
    /// key is `Invalid`; the value is opaque bytes (callers define the encoding).
    async fn put_setting(&self, key: &str, value: Vec<u8>) -> Result<()>;

    // ----- sync plumbing -----

    /// Changes from local writes and from sync. Each call returns an independent stream that
    /// only sees changes after it was created.
    fn subscribe(&self) -> BoxStream<'static, Change>;

    /// Bidirectionally reconcile with `peer`. Idempotent; after it returns both replicas are equal.
    async fn sync_with(&self, peer: &PeerAddr) -> Result<()>;

    fn capability(&self) -> Capability;
}

/// A shared album space with its own membership and capabilities.
#[async_trait]
pub trait AlbumStore: Send + Sync + 'static {
    async fn meta(&self) -> Result<SharedAlbumMeta>;

    /// Add items. **Idempotent by item id:** the first writer by `(added_at, added_by)` wins on
    /// every replica; re-adding changes nothing. Requires a Contributor/Owner role (`ReadOnly`
    /// otherwise). Records an `ItemsAdded` event only for items that were actually new.
    async fn add_items(&self, items: Vec<SharedItem>) -> Result<()>;

    /// Items newest-added first (ties by id), following `page` (`page.after.id` = last id seen).
    async fn items(&self, page: Page) -> Result<Vec<SharedItem>>;

    /// Members ordered by member key.
    async fn members(&self) -> Result<Vec<Member>>;

    /// Activity feed ordered by HLC, strictly after `since` when given.
    async fn events(&self, since: Option<Hlc>) -> Result<Vec<AlbumEvent>>;

    /// Create a bearer invite. Only Owners may invite (`ReadOnly` otherwise).
    async fn invite(&self, role: MemberRole, expiry: Expiry) -> Result<InviteTicket>;

    fn subscribe(&self) -> BoxStream<'static, Change>;

    /// Bidirectionally reconcile with `peer` (extension to the planning sketch, see A-018).
    async fn sync_with(&self, peer: &PeerAddr) -> Result<()>;
}

/// Backend entry point: create/open/join spaces. A backend instance belongs to one device.
#[async_trait]
pub trait StoreBackend: Send + Sync + 'static {
    type Library: LibraryStore;
    type Album: AlbumStore;

    /// This device's identity.
    fn device(&self) -> EndpointId;

    /// Create a brand-new library owned by this device, returning the store and a write capability.
    async fn create_library(
        &self,
        root: &RootPublicKey,
    ) -> Result<(Self::Library, LibraryCapability)>;

    /// A read-only version of `cap` that carries no write material. The default only flips the
    /// mode, which is enough for backends whose capability has no secrets.
    fn downgrade(&self, cap: &LibraryCapability) -> LibraryCapability {
        cap.read_only()
    }

    /// Open (or join as an empty replica) the library `cap` refers to. A `Read` capability yields a
    /// read-only store.
    async fn open_library(&self, cap: &LibraryCapability) -> Result<Self::Library>;

    /// Create a shared album; this device becomes its Owner.
    async fn create_album(&self, meta: SharedAlbumMeta) -> Result<Self::Album>;

    /// Join via a ticket: fails with `Expired` past the ticket's expiry, `Unreachable` if the
    /// inviter cannot be contacted. The joiner gets the ticket's role and a `MemberJoined` event.
    async fn join_album(&self, ticket: &InviteTicket) -> Result<Self::Album>;
}
