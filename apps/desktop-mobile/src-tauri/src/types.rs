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
        /// Devices holding the original (this one included).
        pub copies: u32,
        /// Enough devices hold the original, or a keeper does.
        pub backed_up: bool,
        pub has_thumb: bool,
        /// A poster frame exists on this device (videos only; see `set_poster`).
        pub has_poster: bool,
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
    }
}

ts_type! {
    pub struct BackupStatus {
        pub policy: BackupPolicy,
        /// `granted`, `limited`, `denied` or `unavailable` (no camera roll on this device).
        pub permission: String,
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
