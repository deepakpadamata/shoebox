//! The camera roll as the app sees it (M4-03/04). Each platform provides a [`PhotoSource`]
//! (Android: a Kotlin plugin over MediaStore; iOS: a Swift plugin over PhotoKit); the backup
//! pipeline in `backup.rs` only talks to this trait, so it is tested with [`FakePhotoSource`].
//!
//! Gallery photos are read where they are and never copied (A-092): an asset carries the path of
//! its file, and the library references that file.

use anyhow::{anyhow, Result};
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

/// One photo or video in the camera roll.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Asset {
    /// Stable id on this device (MediaStore `_ID`, PhotoKit local identifier).
    pub id: String,
    /// Last modification, ms since the epoch: a changed asset is imported again.
    pub modified_ms: i64,
    pub size: u64,
    pub mime: String,
    /// The file, when this app can read it where it is. Without one the photo cannot be added
    /// (copying it would keep it twice on the phone).
    #[serde(default)]
    pub path: Option<String>,
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
    /// Ids deleted from the camera roll since then, when the platform says so. Deletions are
    /// found by the files themselves going (`backup.rs`); this only tidies the waiting list.
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
            path: Some(path.display().to_string()),
        }
    }

    /// Add or replace an asset whose original is the file at `path`.
    pub fn put(&self, id: &str, path: impl Into<PathBuf>, modified_ms: i64) {
        let asset = Self::asset(id, path, modified_ms);
        let mut s = self.state.lock().unwrap();
        s.generation += 1;
        let g = s.generation;
        s.assets.retain(|(a, _)| a.id != id);
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

    /// The app may see the asset but not read its file where it is.
    pub fn make_unreadable(&self, id: &str) {
        let mut s = self.state.lock().unwrap();
        for (a, _) in s.assets.iter_mut() {
            if a.id == id {
                a.path = None;
            }
        }
    }

    /// What the person "chooses" next time the app asks.
    pub fn set_picks(&self, picks: Vec<Picked>) {
        self.state.lock().unwrap().picks = picks;
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
