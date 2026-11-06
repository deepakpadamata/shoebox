//! Types shared with the UI. `cargo test -p desktop-mobile` regenerates their TypeScript
//! definitions in `ui/src/bindings/` (CI fails if the committed files differ), so the Rust and
//! TypeScript sides cannot drift apart silently.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

macro_rules! ts_type {
    ($(#[$m:meta])* pub struct $name:ident { $($body:tt)* }) => {
        $(#[$m])*
        #[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
        #[ts(export)]
        pub struct $name { $($body)* }
    };
    ($(#[$m:meta])* pub enum $name:ident { $($body:tt)* }) => {
        $(#[$m])*
        #[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
        #[ts(export)]
        pub enum $name { $($body)* }
    };
}

ts_type! {
    /// What the shell needs to decide between onboarding and the library.
    pub struct AppState {
        pub has_library: bool,
        pub device_name: String,
        pub device_id: String,
        pub role: String,
        /// `member`, `unknown` or `removed` (see `Membership`).
        pub membership: String,
        pub recovery_kit_saved: bool,
        pub library_id: Option<String>,
        /// New admin devices this person has not acknowledged yet (newest last).
        pub alerts: Vec<SecurityAlert>,
        /// Someone is rejoining through this device with the recovery phrase and waits for an
        /// answer (`answer_recovery`).
        pub recovery_request: Option<RecoveryQuestion>,
        /// A newer release is out (and its banner was not dismissed).
        pub update: Option<UpdateInfo>,
    }
}

ts_type! {
    /// A newer release of Shoebox (R1-07).
    pub struct UpdateInfo {
        pub version: String,
        /// The download page.
        pub page: String,
    }
}

ts_type! {
    /// A device became an admin of the library (it can add and remove devices). Shown until
    /// dismissed: if it was not this person, their recovery phrase may be known to someone else.
    pub struct SecurityAlert {
        /// `admin_added` (the device below became an admin) or `phrase_replaced` (the recovery
        /// phrase was replaced on another device; no device named).
        #[serde(default = "admin_added")]
        pub kind: String,
        pub device_id: String,
        pub device_name: String,
        pub at_ms: f64,
        /// A system notification was shown for it (R1-06): once is enough.
        #[serde(default)]
        pub notified: bool,
    }
}

fn admin_added() -> String {
    "admin_added".into()
}

ts_type! {
    /// Settings → About.
    pub struct AboutInfo {
        pub version: String,
        /// The commit this build comes from (`unknown` outside a git checkout).
        pub commit: String,
        /// Where the source code is (the AGPL offer).
        pub source_url: String,
    }
}

ts_type! {
    pub struct RecoveryQuestion {
        pub device_id: String,
        pub device_name: String,
        pub role: String,
    }
}

ts_type! {
    /// Shown once after creating a library: the phrase and the printable kit.
    pub struct CreatedLibrary {
        pub phrase: String,
        pub kit_text: String,
        pub library_id: String,
    }
}

ts_type! {
    pub struct PageCursor {
        pub taken_at: f64,
        pub id: String,
    }
}

ts_type! {
    /// One tile of the timeline.
    pub struct PhotoSummary {
        pub id: String,
        pub taken_at: f64,
        pub favorite: bool,
        pub is_video: bool,
        /// A Live Photo or motion photo: it has a video (F2.10).
        pub live: bool,
        /// Devices holding the original (this one included).
        pub copies: u32,
        /// Enough devices hold the original, or a keeper does.
        pub backed_up: bool,
        pub has_thumb: bool,
        /// The video's poster frame is on this device (videos only; see `set_poster`).
        pub has_poster: bool,
        /// No device has made this video's poster frame yet: one that can play it should.
        pub needs_poster: bool,
        pub has_original: bool,
        pub width: u32,
        pub height: u32,
    }
}

ts_type! {
    pub struct PhotoPage {
        pub items: Vec<PhotoSummary>,
        /// Pass back as `cursor` for the next page; `None` at the end.
        pub next: Option<PageCursor>,
    }
}

ts_type! {
    /// A recognised piece of a search, shown as a removable chip.
    pub struct SearchChip {
        /// `date`, `kind`, `favourite`, `person`, `album`, `place`, `camera` or `lens`.
        pub kind: String,
        pub label: String,
        /// Span in the query, in characters (`start..end`).
        pub start: u32,
        pub end: u32,
    }
}

ts_type! {
    pub struct SearchPage {
        pub items: Vec<PhotoSummary>,
        /// Pass back as `cursor` for the next page; `None` at the end.
        pub next: Option<u32>,
        pub total: u32,
        /// Best matches first (otherwise newest first, like the timeline).
        pub ranked: bool,
        pub filters: Vec<SearchChip>,
        /// The words no filter took.
        pub free_text: String,
    }
}

ts_type! {
    /// A device that indexes photos for search, as the search status shows it.
    pub struct SearchIndexer {
        pub name: String,
        pub photos_per_min: u32,
        pub this_device: bool,
        pub reads_text: bool,
        pub finds_faces: bool,
        /// The device's id, as the devices list has it.
        pub device_id: String,
    }
}

ts_type! {
    /// How much of the library each kind of search covers on this device (S1-16).
    pub struct SearchStatus {
        pub total: u32,
        /// Searchable by what is in them (a picture vector).
        pub content: u32,
        pub labelled: u32,
        pub text_read: u32,
        pub faces: u32,
        /// Photos likely to hold text, and how many of them have been read.
        pub text_wanted: u32,
        pub text_wanted_read: u32,
        /// Photos faces are looked for in (not videos).
        pub faces_wanted: u32,
        /// People is on for the library.
        pub people_on: bool,
        /// This device can search by meaning (it has the text model).
        pub meaning: bool,
        pub indexers: Vec<SearchIndexer>,
        pub eta_secs: Option<u32>,
        /// Search data on this device, in bytes.
        pub held_bytes: f64,
        /// This device's setting: index here (desktop: on power when idle; phone: while charging).
        pub index_here: bool,
        /// Indexing is running on this device now.
        pub indexing: bool,
        /// What this device is doing now: pictures, text, faces, grouping, compacting, or empty.
        pub stage: String,
        /// Why this device is not indexing now (waiting for power, say), if so.
        pub waiting: Option<String>,
        /// This device has nothing left to do.
        pub caught_up: bool,
        /// A search model being downloaded now (S2-02), as the indexing job reports it.
        pub model_download: Option<ModelDownload>,
    }
}

ts_type! {
    /// A search model this device is downloading because no device of the library offers it (S2-02).
    pub struct ModelDownload {
        pub name: String,
        pub done: f64,
        pub total: f64,
    }
}

ts_type! {
    /// A face in a photo (S1-19): where it is (fractions of the photo) and who it is.
    pub struct FaceOut {
        pub photo: String,
        pub rect: Vec<f32>,
        /// The photo's size, to crop the face from its preview.
        pub width: u32,
        pub height: u32,
        pub person: Option<String>,
        pub name: Option<String>,
        /// An unnamed group, when no person.
        pub group: Option<u32>,
    }
}

ts_type! {
    /// A person, or an unnamed group of faces, on the People screen.
    pub struct PersonCard {
        /// A person's id (hex), or `group:<n>` for an unnamed group.
        pub id: String,
        pub name: String,
        /// Photos they are in.
        pub count: u32,
        pub cover: FaceOut,
        pub me: bool,
        pub hidden: bool,
    }
}

ts_type! {
    pub struct PeopleList {
        /// People is on for the library.
        pub on: bool,
        pub people: Vec<PersonCard>,
        /// Unnamed groups with enough faces to show.
        pub groups: Vec<PersonCard>,
    }
}

ts_type! {
    pub struct SearchSuggestion {
        /// `person`, `album`, `place`, `camera`, `lens` or `thing`.
        pub kind: String,
        pub name: String,
    }
}

ts_type! {
    pub struct LibraryCounts {
        pub photos: u32,
        pub favorites: u32,
        pub deleted: u32,
        pub at_risk: u32,
        pub redundancy_target: u32,
    }
}

ts_type! {
    pub struct StoredOn {
        pub device_id: String,
        pub name: String,
        pub role: String,
        pub online: bool,
        pub is_this_device: bool,
        /// `original`, `preview` or `thumb`.
        pub kinds: Vec<String>,
    }
}

ts_type! {
    pub struct PhotoDetail {
        pub id: String,
        pub taken_at: f64,
        pub favorite: bool,
        pub deleted: bool,
        pub is_video: bool,
        /// A Live Photo or motion photo: it has a video (F2.10).
        pub live: bool,
        /// The video is on this device; if not, `fetch_now` with kind `motion` gets it.
        pub has_motion: bool,
        /// How long the video runs.
        pub motion_ms: u32,
        pub mime: String,
        pub size: f64,
        pub width: u32,
        pub height: u32,
        pub camera: Option<String>,
        pub lens: Option<String>,
        pub gps: Option<(f64, f64)>,
        pub imported_by: String,
        pub pinned: bool,
        pub copies: u32,
        pub redundancy_target: u32,
        pub has_thumb: bool,
        pub has_preview: bool,
        pub has_original: bool,
        pub stored_on: Vec<StoredOn>,
        pub albums: Vec<String>,
    }
}

ts_type! {
    pub struct AlbumInfo {
        pub id: String,
        pub name: String,
        pub cover: Option<String>,
        pub item_count: u32,
        /// The shared album this album is shared as, if it is shared.
        pub shared_as: Option<String>,
    }
}

ts_type! {
    pub struct DeviceInfo {
        pub id: String,
        pub name: String,
        pub role: String,
        pub online: bool,
        pub is_this_device: bool,
        pub admin: bool,
        pub revoked: bool,
        /// `lan`, `direct`, `relay` or `none`.
        pub path: String,
        pub latency_ms: Option<u32>,
    }
}

ts_type! {
    /// How much of the library one device holds (Devices screen), from the replicated holdings.
    pub struct DeviceStorageInfo {
        pub id: String,
        /// Photos and videos whose original it holds, and their size in bytes.
        pub originals: u32,
        pub original_bytes: f64,
        /// Previews and thumbnails, in bytes.
        pub other_bytes: f64,
    }
}

ts_type! {
    /// One blob on its way in or out right now (Devices screen).
    pub struct TransferItem {
        /// The photo it belongs to; `None` for library metadata.
        pub photo: Option<String>,
        /// `thumb`, `preview`, `original`, or `other` (library metadata).
        pub kind: String,
        /// The device (hex id) it comes from or goes to, when known.
        pub peer: Option<String>,
        pub size: Option<f64>,
        pub done: f64,
    }
}

ts_type! {
    /// What this device is moving and has still to fetch.
    pub struct Transfers {
        pub downloading: Vec<TransferItem>,
        pub sending: Vec<TransferItem>,
        /// Bytes per second over the last few seconds.
        pub download_rate: f64,
        pub upload_rate: f64,
        /// Still to fetch here, not counting what is in flight.
        pub waiting_thumbs: u32,
        pub waiting_previews: u32,
        pub waiting_originals: u32,
        pub waiting_bytes: f64,
        /// Why this device's fetching is paused, if it is.
        pub paused: Option<String>,
    }
}

ts_type! {
    pub struct PairStart {
        /// The ticket text (what the QR encodes).
        pub ticket: String,
        /// The QR code as an SVG document.
        pub qr_svg: String,
        pub expires_at: f64,
    }
}

ts_type! {
    pub struct ImportSummary {
        pub imported: u32,
        pub duplicates: u32,
        pub skipped: u32,
        pub failed: Vec<String>,
    }
}

ts_type! {
    pub struct ExportReport {
        pub written: u32,
        /// Photos whose original is not on this device (nothing was exported for them).
        pub missing: u32,
        pub bytes: f64,
        pub folder: String,
    }
}

ts_type! {
    /// Camera-roll backup rules (mockup 5e). Off until the user turns it on.
    pub struct BackupPolicy {
        pub enabled: bool,
        pub wifi_only: bool,
        pub charging_only: bool,
        /// Pause below this battery level unless charging.
        pub min_battery: u8,
        /// Which gallery folders are backed up.
        #[serde(default)]
        pub scope: BackupScope,
    }
}

/// Which gallery folders backup takes. A photo already in the library keeps following its edits
/// and deletions whatever the scope. (Written out: `ts_type!` puts attributes before its derive.)
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case", tag = "kind", content = "folders")]
pub enum BackupScope {
    /// What the camera takes: `DCIM`, without its screen-capture folders. The default.
    #[default]
    Camera,
    /// Every folder, new ones included.
    All,
    /// Exactly these folders ([`BackupFolder::path`]).
    Folders(Vec<String>),
}

ts_type! {
    /// A gallery folder with photos or videos in it, for choosing what to back up.
    pub struct BackupFolder {
        /// The folder on this device: what [`BackupScope::Folders`] lists.
        pub path: String,
        /// Its own name ("Camera", "Screenshots").
        pub name: String,
        /// Where it is from the top of its storage ("DCIM/Camera", "SD card/DCIM/Camera"), to tell
        /// folders of one name apart.
        pub location: String,
        pub photos: u32,
        /// A camera folder: backed up by default.
        pub camera: bool,
        /// Backed up under the current scope.
        pub included: bool,
    }
}

ts_type! {
    pub struct BackupStatus {
        pub policy: BackupPolicy,
        /// `granted`, `limited`, `denied` or `unavailable` (no camera roll on this device).
        pub permission: String,
        /// What is backed up from here: `camera_roll` (a phone's gallery), `photos_library` (the
        /// Photos app on a Mac), or empty when there is nothing.
        pub source: String,
        /// Photos waiting because their original is in iCloud only: downloaded on mains power.
        pub cloud_waiting: u32,
        pub imported: u32,
        pub pending: u32,
        /// Why it is waiting right now, if it is.
        pub paused: Option<String>,
        pub last_scan_ms: f64,
        pub last_error: Option<String>,
        /// Photos gone from the gallery all at once, waiting for the person to say whether
        /// to delete them from the library too.
        pub gallery_deletions_held: u32,
    }
}

ts_type! {
    /// What backing up would take, before it is turned on (the Photos library on a Mac, P1-04).
    pub struct BackupPreview {
        pub photos: u32,
        pub videos: u32,
        /// Bytes of the originals, those in iCloud only included.
        pub bytes: f64,
        /// Originals in iCloud only, downloaded on mains power.
        pub cloud_only: u32,
        /// `granted` or `limited` (only the photos the person chose).
        pub permission: String,
    }
}

ts_type! {
    #[derive(Default)]
    pub struct BackupReport {
        pub imported: u32,
        pub duplicates: u32,
        pub pending: u32,
        pub failed: Vec<String>,
    }
}

ts_type! {
    /// What a background wake-up achieved (M4-08/09).
    pub struct BurstReport {
        pub peers_total: u32,
        pub peers_reached: u32,
        pub at_risk_before: u32,
        pub at_risk_after: u32,
        pub elapsed_ms: f64,
        /// Nothing is at risk any more.
        pub finished: bool,
        /// Stopped because the time budget ended (the input of the notification rule).
        pub budget_exhausted: bool,
        pub reason: Option<String>,
    }
}

ts_type! {
    pub struct FreeUpReport {
        pub photos: u32,
        pub bytes: f64,
    }
}

ts_type! {
    /// Library health (mockup 6a).
    pub struct Health {
        pub photos: u32,
        /// `copies_histogram[n]` = photos whose original is held by `n` devices (last bucket = n or more).
        pub copies_histogram: Vec<u32>,
        pub at_risk: u32,
        pub redundancy_target: u32,
        pub originals_here: u32,
        pub originals_here_bytes: f64,
        pub devices_online: u32,
        pub devices_total: u32,
        pub can_free_bytes: f64,
        pub can_free_photos: u32,
    }
}

ts_type! {
    pub struct Settings {
        pub device_name: String,
        pub redundancy_target: u32,
        /// `default`, `off` or a custom relay URL.
        pub relay: String,
        pub watched_folders: Vec<String>,
        pub role: String,
        /// Look for a newer release now and then.
        pub update_checks: bool,
        /// A Companion keeps the originals of photos taken in the last this many days (0: off).
        pub keep_recent_days: u32,
        /// A Companion also keeps the originals of favourites.
        pub keep_favorites: bool,
    }
}

ts_type! {
    /// Everything the UI is told about as it happens.
    pub enum AppEvent {
        /// The library changed (local write or sync): refetch what is on screen.
        LibraryChanged,
        DevicesChanged,
        PeerConnected { id: String },
        PeerDisconnected { id: String },
        Sync { peer: String, phase: String },
        Fetched { photo: Option<String>, kind: String },
        ImportProgress { done: u32, total: u32 },
        PairingCode { sas: String, peer_name: String, peer_role: String },
        PairingDone,
        PairingFailed { reason: String },
        RemovedFromLibrary,
        LibraryRotated,
        /// A recovery waits for this device's answer, an alert was added or a newer release
        /// appeared: reload the state.
        SecurityChanged,
        BackupProgress { done: u32, total: u32 },
        BackupPaused { reason: Option<String> },
        /// Many photos left the gallery at once; `count` wait for an answer (0: answered).
        GalleryDeletionsHeld { count: u32 },
        /// Something changed in a shared album (new photos, members, downloads).
        SharedChanged { album: String },
    }
}

ts_type! {
    /// The report a person can send to get help (see `diagnostics.rs`).
    pub struct DiagnosticsBundle {
        pub file_name: String,
        pub text: String,
    }
}

ts_type! {
    /// A shared album this device belongs to.
    pub struct SharedAlbumInfo {
        pub id: String,
        pub name: String,
        /// `owner`, `contributor` or `viewer`.
        pub role: String,
        pub item_count: u32,
        pub member_count: u32,
        pub keep_originals: bool,
        /// What to do with GPS data when adding photos: `ask`, `keep` or `remove`.
        pub location: String,
        /// Photos other members added since this album was last opened.
        pub unread: u32,
        /// Item id of the cover (the newest photo).
        pub cover: Option<String>,
    }
}

ts_type! {
    pub struct SharedItemInfo {
        pub id: String,
        pub taken_at: f64,
        pub added_at: f64,
        pub added_by: String,
        pub is_video: bool,
        pub has_thumb: bool,
        pub has_original: bool,
        /// Added by this device.
        pub mine: bool,
        /// The location was taken out of the shared copy.
        pub location_removed: bool,
    }
}

ts_type! {
    pub struct SharedItemsPage {
        pub items: Vec<SharedItemInfo>,
        /// Pass back as `after` for the next page.
        pub next: Option<String>,
        pub total: u32,
    }
}

ts_type! {
    pub struct SharedMemberInfo {
        pub key: String,
        pub name: String,
        pub role: String,
        pub me: bool,
        /// How many of this person's devices are in the album.
        pub devices: u32,
    }
}

ts_type! {
    /// One line of an album's activity: several adds by the same person close together are one.
    pub struct FeedEntry {
        /// `added` or `joined`.
        pub kind: String,
        pub member: String,
        pub count: u32,
        pub at: f64,
    }
}

ts_type! {
    pub struct InviteOut {
        pub text: String,
        pub qr_svg: String,
        pub expires_at: f64,
    }
}

ts_type! {
    /// What would happen if these photos were added: how many carry a location.
    pub struct AddCheck {
        pub total: u32,
        pub with_location: u32,
        /// The album's setting: `ask`, `keep` or `remove`.
        pub policy: String,
    }
}

ts_type! {
    pub struct AddResult {
        pub added: u32,
        pub location_removed: u32,
        /// Photos left out, with the reason.
        pub skipped: Vec<String>,
    }
}

ts_type! {
    /// The name people you share with see (the same on all your devices).
    pub struct Profile {
        pub name: String,
        /// False while it is still this device's name (no name was set).
        pub is_set: bool,
    }
}

ts_type! {
    /// One web link to an album (W1-07).
    pub struct WebLinkInfo {
        pub share_id: String,
        /// A Live link: served from the sharer's own device while it is online, nothing stored with us.
        pub live: bool,
        /// Live only: this device is holding the album and answering for it.
        pub online: bool,
        /// The whole link, key included.
        pub text: String,
        pub qr_svg: String,
        pub full_size: bool,
        pub expires_at: Option<f64>,
        pub created_at: f64,
        pub items: u32,
        /// Photos left out because their previews are not on this device yet.
        pub skipped: u32,
        /// Originals left out because their location could not be removed.
        pub withheld: u32,
        /// The album has changed since this version was made.
        pub stale: bool,
        /// Someone reported this link and the service switched it off; their words.
        pub reported: Option<String>,
    }
}

ts_type! {
    /// An album's web links, and whether the cloud is set up for this library.
    pub struct WebLinksInfo {
        /// The library has its cloud secret.
        pub enabled: bool,
        /// The service's address, if one is set.
        pub url: Option<String>,
        pub links: Vec<WebLinkInfo>,
    }
}

ts_type! {
    /// What making a link would upload, so the person can decide before anything is sent.
    pub struct WebLinkCheck {
        pub photos: u32,
        pub videos: u32,
        /// Ciphertext to upload, at most (what the service already holds is not sent again).
        pub bytes: f64,
        /// Photos with a place, whose original would go up with it removed (only with full size).
        pub with_location: u32,
        /// Of those, ones in a format whose location cannot be removed: left out.
        pub unremovable: u32,
    }
}
