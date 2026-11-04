//! The camera roll as the app sees it (M4-03/04). Each platform provides a [`PhotoSource`]
//! (Android: a Kotlin plugin over MediaStore; iOS: a Swift plugin over PhotoKit); the backup
//! pipeline in `backup.rs` only talks to this trait, so it is tested with [`FakePhotoSource`].
//!
//! Gallery photos are read where they are and never copied (A-092): on Android an asset carries
//! the path of its file, and the library references that file. PhotoKit has no paths: its assets
//! are [`Original::Library`], read through [`PhotoSource::read_original`] (P1-02).

use anyhow::{anyhow, Result};
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// One photo or video in the camera roll.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "AssetWire", into = "AssetWire")]
pub struct Asset {
    /// Stable id on this device (MediaStore `_ID`, PhotoKit local identifier).
    pub id: String,
    /// Last modification, ms since the epoch: a changed asset is imported again.
    pub modified_ms: i64,
    pub size: u64,
    pub mime: String,
    pub original: Original,
}

/// Where an asset's original is read from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Original {
    /// A file this app can read where it is (Android: MediaStore `DATA`).
    File(String),
    /// Only through the platform's photo library, by the asset's id (PhotoKit). `local: false`
    /// when the original is not on this device (iCloud "Optimize Storage").
    Library { local: bool },
    /// Not readable where it is. The photo cannot be added: copying it would keep it twice on
    /// the phone.
    Unreadable,
}

impl Asset {
    /// The file, for an [`Original::File`] asset.
    pub fn path(&self) -> Option<&str> {
        match &self.original {
            Original::File(p) => Some(p),
            _ => None,
        }
    }
}

/// What the platform plugins send: Android's `path` (absent when unreadable), and PhotoKit's
/// `library` and `cloud_only` flags.
#[derive(Clone, Serialize, Deserialize)]
struct AssetWire {
    id: String,
    modified_ms: i64,
    size: u64,
    mime: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    library: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    cloud_only: bool,
}

impl From<AssetWire> for Asset {
    fn from(w: AssetWire) -> Self {
        let original = match (w.library, w.path) {
            (true, _) => Original::Library {
                local: !w.cloud_only,
            },
            (false, Some(p)) => Original::File(p),
            (false, None) => Original::Unreadable,
        };
        Self {
            id: w.id,
            modified_ms: w.modified_ms,
            size: w.size,
            mime: w.mime,
            original,
        }
    }
}

impl From<Asset> for AssetWire {
    fn from(a: Asset) -> Self {
        let (path, library, cloud_only) = match a.original {
            Original::File(p) => (Some(p), false, false),
            Original::Library { local } => (None, true, !local),
            Original::Unreadable => (None, false, false),
        };
        Self {
            id: a.id,
            modified_ms: a.modified_ms,
            size: a.size,
            mime: a.mime,
            path,
            library,
            cloud_only,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Permission {
    Granted,
    /// The user shared only some photos (Android 14+, iOS "limited").
    Limited,
    Denied,
}

/// What changed since the token passed to [`PhotoSource::scan`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scan {
    /// New or modified assets.
    pub assets: Vec<Asset>,
    /// Ids deleted from the camera roll since then, when the platform says so. This is how a
    /// [`Original::Library`] photo's deletion is known; a file's is found by the file going
    /// (`backup.rs`).
    pub removed: Vec<String>,
    /// Pass back next time to get only later changes (an opaque platform change token).
    pub token: String,
}

/// The device state the sync rules look at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Conditions {
    pub on_wifi: bool,
    pub charging: bool,
    pub battery_pct: u8,
}

/// One item the person chose to add.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Picked {
    /// A gallery photo this app can read where it is: added like a camera-roll photo.
    InPlace(Asset),
    /// Not on this phone's gallery (a cloud drive, a USB stick): a handle for
    /// [`PhotoSource::read_picked`], and the library keeps the only copy here.
    Copy(String),
    /// A gallery photo this app may not read where it is (only some photos are shared with it).
    /// Adding it would mean a second copy, so it is not added; the name is for the message.
    Unreadable(String),
}

/// A video on this device, for the platform's player and thumbnailer (`video.rs`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VideoSource {
    /// Its file in this phone's gallery, played where it is; `None` when it is in the blob store.
    pub path: Option<String>,
    /// The blob (hex hash), read through the app core when there is no `path`.
    pub blob: String,
    pub mime: String,
    pub size: u64,
}

pub trait PhotoSource: Send + Sync {
    fn permission(&self) -> BoxFuture<'_, Result<Permission>>;
    fn request_permission(&self) -> BoxFuture<'_, Result<Permission>>;
    /// Everything (`token: None`) or only what changed since `token`.
    fn scan(&self, token: Option<String>) -> BoxFuture<'_, Result<Scan>>;
    fn conditions(&self) -> BoxFuture<'_, Result<Conditions>>;
    /// Let the person choose photos and videos to add: from the gallery (`files: false`) or from
    /// files anywhere (`true`). None when cancelled.
    fn pick(&self, _files: bool) -> BoxFuture<'_, Result<Vec<Picked>>> {
        Box::pin(async { Err(anyhow!("choosing photos is not available on this device")) })
    }
    /// A readable copy of a [`Picked::Copy`] file, named like the original; the caller deletes it.
    fn read_picked(&self, _handle: &str) -> BoxFuture<'_, Result<PathBuf>> {
        Box::pin(async { Err(anyhow!("choosing photos is not available on this device")) })
    }
    /// A readable copy of the original of the [`Original::Library`] asset `id`, made without the
    /// network, alone in a new directory that the caller deletes with everything in it. A Live
    /// Photo's video is written beside it under the same name stem (`IMG_0001.HEIC` and
    /// `IMG_0001.MOV`), so the import pairs them as it does in a folder (P1-08).
    fn read_original(&self, _id: &str) -> BoxFuture<'_, Result<PathBuf>> {
        Box::pin(async { Err(anyhow!("this device has no photo library to read from")) })
    }
    /// [`PhotoSource::read_original`] for an original that is in the cloud only, downloading it
    /// (iCloud "Optimize Storage"). Backup asks only on mains power (D3).
    fn download_original(&self, _id: &str) -> BoxFuture<'_, Result<PathBuf>> {
        Box::pin(async { Err(anyhow!("this device cannot download originals")) })
    }
    /// What the person backs up from here: `camera_roll` (a phone's gallery) or `photos_library`
    /// (the Photos app on a Mac), for the settings screen.
    fn kind(&self) -> &'static str {
        "camera_roll"
    }
    /// A poster frame (JPEG) for the video `id` (its file `path`, for an [`Original::File`]
    /// asset), made by the system's own thumbnailer; `None` where there is none.
    fn video_frame(
        &self,
        _id: &str,
        _path: Option<&str>,
    ) -> BoxFuture<'_, Result<Option<Vec<u8>>>> {
        Box::pin(async { Ok(None) })
    }
    /// Videos play in the platform's own player rather than the web view (Android, R1-16).
    fn plays_video(&self) -> bool {
        false
    }
    /// Open `video` in the platform's player, over the app.
    fn play_video(&self, _video: &VideoSource) -> BoxFuture<'_, Result<()>> {
        Box::pin(async { Err(anyhow!("this device plays videos in the app")) })
    }
    /// A poster frame (JPEG) of `video` from the system's thumbnailer; `None` where there is none.
    fn video_frame_of(&self, _video: &VideoSource) -> BoxFuture<'_, Result<Option<Vec<u8>>>> {
        Box::pin(async { Ok(None) })
    }
}

/// The JPEG in a base64 reply from the platform side; empty means none.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub(crate) fn decode_frame(base64_jpeg: &str) -> Result<Option<Vec<u8>>> {
    use base64::Engine;
    if base64_jpeg.is_empty() {
        return Ok(None);
    }
    Ok(Some(
        base64::engine::general_purpose::STANDARD.decode(base64_jpeg)?,
    ))
}

/// An in-memory camera roll for tests and demos: assets are files the test creates.
pub struct FakePhotoSource {
    dir: PathBuf,
    state: Mutex<FakeState>,
}

struct FakeState {
    permission: Permission,
    /// What [`PhotoSource::request_permission`] turns `permission` into.
    on_request: Option<Permission>,
    conditions: Conditions,
    generation: u64,
    /// `(asset, generation it last changed in)`
    assets: Vec<(Asset, u64)>,
    removed: Vec<(String, u64)>,
    /// What the next [`PhotoSource::pick`] returns.
    picks: Vec<Picked>,
    /// Files the person stops sharing at the next prompt (deleted here, as Android hides them).
    unshare_on_request: Vec<PathBuf>,
    /// Library assets the person stops sharing at the next prompt (hidden, as PhotoKit does).
    hide_on_request: Vec<String>,
    /// The file behind each [`Original::Library`] asset, which only [`PhotoSource::read_original`]
    /// reads.
    library_files: BTreeMap<String, PathBuf>,
    /// A Live Photo's video, by asset id.
    library_videos: BTreeMap<String, PathBuf>,
    reads: u64,
    /// Originals downloaded from the cloud so far, by asset id.
    downloads: Vec<String>,
    /// What [`PhotoSource::kind`] says.
    kind: &'static str,
}

impl FakePhotoSource {
    pub fn new(scratch: impl Into<PathBuf>) -> Self {
        Self {
            dir: scratch.into(),
            state: Mutex::new(FakeState {
                permission: Permission::Granted,
                on_request: None,
                conditions: Conditions {
                    on_wifi: true,
                    charging: true,
                    battery_pct: 80,
                },
                generation: 1,
                assets: vec![],
                removed: vec![],
                picks: vec![],
                unshare_on_request: vec![],
                hide_on_request: vec![],
                library_files: BTreeMap::new(),
                library_videos: BTreeMap::new(),
                reads: 0,
                downloads: vec![],
                kind: "camera_roll",
            }),
        }
    }

    /// The asset `id` for the file at `path`, as a scan reports it.
    pub fn asset(id: &str, path: impl Into<PathBuf>, modified_ms: i64) -> Asset {
        let path = path.into();
        Asset {
            id: id.into(),
            modified_ms,
            size: std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0),
            mime: match path.extension().and_then(|e| e.to_str()) {
                Some("mp4") => "video/mp4",
                Some("png") => "image/png",
                _ => "image/jpeg",
            }
            .into(),
            original: Original::File(path.display().to_string()),
        }
    }

    /// Add or replace an asset whose original is the file at `path`.
    pub fn put(&self, id: &str, path: impl Into<PathBuf>, modified_ms: i64) {
        self.insert(Self::asset(id, path, modified_ms));
    }

    /// Add or replace a photo-library asset (PhotoKit) whose original has the bytes of the file at
    /// `path`. The scan does not report the file; it is read through
    /// [`PhotoSource::read_original`].
    pub fn put_library(&self, id: &str, path: impl Into<PathBuf>, modified_ms: i64) {
        let path = path.into();
        let mut asset = Self::asset(id, &path, modified_ms);
        asset.original = Original::Library { local: true };
        self.state
            .lock()
            .unwrap()
            .library_files
            .insert(id.into(), path);
        self.insert(asset);
    }

    /// A Live Photo in the library: the still at `still` and its video at `video`.
    pub fn put_library_live(
        &self,
        id: &str,
        still: impl Into<PathBuf>,
        video: impl Into<PathBuf>,
        modified_ms: i64,
    ) {
        self.put_library(id, still, modified_ms);
        self.state
            .lock()
            .unwrap()
            .library_videos
            .insert(id.into(), video.into());
    }

    /// Act as a Mac's Photos library rather than a phone's camera roll.
    pub fn set_photos_library(&self) {
        self.state.lock().unwrap().kind = "photos_library";
    }

    /// Originals downloaded so far, by asset id.
    pub fn downloads(&self) -> Vec<String> {
        self.state.lock().unwrap().downloads.clone()
    }

    fn insert(&self, asset: Asset) {
        let mut s = self.state.lock().unwrap();
        s.generation += 1;
        let g = s.generation;
        s.assets.retain(|(a, _)| a.id != asset.id);
        s.assets.push((asset, g));
    }

    /// Forget an asset (the platform reports it removed). Its file is the test's to delete.
    pub fn remove(&self, id: &str) {
        let mut s = self.state.lock().unwrap();
        s.generation += 1;
        let g = s.generation;
        s.assets.retain(|(a, _)| a.id != id);
        s.removed.push((id.into(), g));
    }

    /// The library asset is no longer shared with the app: it leaves the scans without being
    /// reported removed.
    pub fn hide(&self, id: &str) {
        let mut s = self.state.lock().unwrap();
        s.assets.retain(|(a, _)| a.id != id);
        s.library_files.remove(id);
    }

    /// Whether the library asset's original is on this device (an edit of it as far as scans
    /// go, as PhotoKit reports a download).
    pub fn set_local(&self, id: &str, local: bool) {
        let mut s = self.state.lock().unwrap();
        s.generation += 1;
        let g = s.generation;
        for (a, at) in s.assets.iter_mut() {
            if a.id == id {
                a.original = Original::Library { local };
                *at = g;
            }
        }
    }

    pub fn set_permission(&self, p: Permission) {
        self.state.lock().unwrap().permission = p;
    }

    /// The person's answer the next time the app asks for access.
    pub fn answer_request(&self, p: Permission) {
        self.state.lock().unwrap().on_request = Some(p);
    }

    pub fn set_conditions(&self, c: Conditions) {
        self.state.lock().unwrap().conditions = c;
    }

    /// At the next prompt the person unshares these files: Android then hides them from the app,
    /// which the fake shows by deleting them.
    pub fn unshare_on_request(&self, files: Vec<PathBuf>) {
        self.state.lock().unwrap().unshare_on_request = files;
    }

    /// At the next prompt the person stops sharing these library assets.
    pub fn hide_on_request(&self, ids: Vec<String>) {
        self.state.lock().unwrap().hide_on_request = ids;
    }

    /// The app may see the asset but not read its file where it is.
    pub fn make_unreadable(&self, id: &str) {
        let mut s = self.state.lock().unwrap();
        for (a, _) in s.assets.iter_mut() {
            if a.id == id {
                a.original = Original::Unreadable;
            }
        }
    }

    /// What the person "chooses" next time the app asks.
    pub fn set_picks(&self, picks: Vec<Picked>) {
        self.state.lock().unwrap().picks = picks;
    }
}

impl FakePhotoSource {
    /// A library original (and a Live Photo's video beside it) in a directory of its own, as
    /// PhotoKit's reads are; `network` also takes one that is in the cloud only.
    fn read_library(&self, id: &str, network: bool) -> Result<PathBuf> {
        let (src, video, n) = {
            let mut s = self.state.lock().unwrap();
            let local = s.assets.iter().find_map(|(a, _)| match a.original {
                Original::Library { local } if a.id == id => Some(local),
                _ => None,
            });
            match local {
                None => return Err(anyhow!("{id} is not in the library")),
                Some(false) if !network => return Err(anyhow!("{id} is not on this device")),
                Some(false) => s.downloads.push(id.into()),
                Some(true) => {}
            }
            s.reads += 1;
            let src = s
                .library_files
                .get(id)
                .cloned()
                .ok_or_else(|| anyhow!("{id} is not in the library"))?;
            (src, s.library_videos.get(id).cloned(), s.reads)
        };
        let dir = self.dir.join(format!("read-{n}"));
        std::fs::create_dir_all(&dir)?;
        let name = src.file_name().ok_or_else(|| anyhow!("no file name"))?;
        let copy = dir.join(name);
        std::fs::copy(&src, &copy)?;
        if let Some(v) = video {
            let stem = Path::new(name).file_stem().unwrap_or_default();
            let ext = v.extension().unwrap_or_default();
            std::fs::copy(&v, dir.join(stem).with_extension(ext))?;
        }
        Ok(copy)
    }
}

impl PhotoSource for FakePhotoSource {
    fn permission(&self) -> BoxFuture<'_, Result<Permission>> {
        Box::pin(async move { Ok(self.state.lock().unwrap().permission) })
    }
    fn request_permission(&self) -> BoxFuture<'_, Result<Permission>> {
        Box::pin(async move {
            let mut s = self.state.lock().unwrap();
            if let Some(p) = s.on_request.take() {
                s.permission = p;
            }
            for f in std::mem::take(&mut s.unshare_on_request) {
                let _ = std::fs::remove_file(f);
            }
            for id in std::mem::take(&mut s.hide_on_request) {
                s.assets.retain(|(a, _)| a.id != id);
                s.library_files.remove(&id);
            }
            Ok(s.permission)
        })
    }
    fn scan(&self, token: Option<String>) -> BoxFuture<'_, Result<Scan>> {
        Box::pin(async move {
            let s = self.state.lock().unwrap();
            if s.permission == Permission::Denied {
                return Err(anyhow!("photo access was not granted"));
            }
            let since: u64 = token.and_then(|t| t.parse().ok()).unwrap_or(0);
            Ok(Scan {
                assets: s
                    .assets
                    .iter()
                    .filter(|(_, g)| *g > since)
                    .map(|(a, _)| a.clone())
                    .collect(),
                removed: s
                    .removed
                    .iter()
                    .filter(|(_, g)| *g > since)
                    .map(|(i, _)| i.clone())
                    .collect(),
                token: s.generation.to_string(),
            })
        })
    }
    fn conditions(&self) -> BoxFuture<'_, Result<Conditions>> {
        Box::pin(async move { Ok(self.state.lock().unwrap().conditions) })
    }
    fn pick(&self, _files: bool) -> BoxFuture<'_, Result<Vec<Picked>>> {
        Box::pin(async move { Ok(std::mem::take(&mut self.state.lock().unwrap().picks)) })
    }
    fn read_original(&self, id: &str) -> BoxFuture<'_, Result<PathBuf>> {
        let id = id.to_string();
        Box::pin(async move { self.read_library(&id, false) })
    }
    fn download_original(&self, id: &str) -> BoxFuture<'_, Result<PathBuf>> {
        let id = id.to_string();
        Box::pin(async move { self.read_library(&id, true) })
    }
    fn kind(&self) -> &'static str {
        self.state.lock().unwrap().kind
    }
    fn read_picked(&self, handle: &str) -> BoxFuture<'_, Result<PathBuf>> {
        let src = PathBuf::from(handle);
        Box::pin(async move {
            std::fs::create_dir_all(&self.dir)?;
            let name = src.file_name().ok_or_else(|| anyhow!("no file name"))?;
            let copy = self.dir.join(format!("picked-{}", name.to_string_lossy()));
            std::fs::copy(&src, &copy)?;
            Ok(copy)
        })
    }
}
