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
        pub has_thumb: bool,
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
    }
}
