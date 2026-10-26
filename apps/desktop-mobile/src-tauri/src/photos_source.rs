//! The camera roll as the app sees it (M4-03/04). Each platform provides a [`PhotoSource`]
//! (Android: a Kotlin plugin over MediaStore; iOS: a Swift plugin over PhotoKit); the backup
//! pipeline in `backup.rs` only talks to this trait, so it is tested with [`FakePhotoSource`].

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
    /// Ids deleted from the camera roll since then (the library keeps its copy).
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

pub trait PhotoSource: Send + Sync {
    fn permission(&self) -> BoxFuture<'_, Result<Permission>>;
    fn request_permission(&self) -> BoxFuture<'_, Result<Permission>>;
    /// Everything (`token: None`) or only what changed since `token`.
    fn scan(&self, token: Option<String>) -> BoxFuture<'_, Result<Scan>>;
    /// A readable file holding the original bytes; the caller deletes it when done. Fails with a
    /// reason for assets that cannot be read (for example an iCloud-only photo that is offline).
    fn read_original(&self, id: &str) -> BoxFuture<'_, Result<PathBuf>>;
    fn conditions(&self) -> BoxFuture<'_, Result<Conditions>>;
    /// Let the person choose photos and videos to add: from the gallery (`files: false`, any
    /// album, whatever the backup permission) or from files anywhere (`true`). Returns handles
    /// for [`PhotoSource::read_picked`]; none when cancelled.
    fn pick(&self, _files: bool) -> BoxFuture<'_, Result<Vec<String>>> {
        Box::pin(async { Err(anyhow!("choosing photos is not available on this device")) })
    }
    /// A readable copy of a picked file, named like the original; the caller deletes it.
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
    conditions: Conditions,
    generation: u64,
    /// `(asset, file path, generation it last changed in)`
    assets: Vec<(Asset, PathBuf, u64)>,
    removed: Vec<(String, u64)>,
    unreadable: Vec<String>,
    reads: Vec<String>,
    /// What the next [`PhotoSource::pick`] returns.
    picks: Vec<PathBuf>,
}

impl FakePhotoSource {
    pub fn new(scratch: impl Into<PathBuf>) -> Self {
        Self {
            dir: scratch.into(),
            state: Mutex::new(FakeState {
                permission: Permission::Granted,
                conditions: Conditions {
                    on_wifi: true,
                    charging: true,
                    battery_pct: 80,
                },
                generation: 1,
                assets: vec![],
                removed: vec![],
                unreadable: vec![],
                reads: vec![],
                picks: vec![],
            }),
        }
    }

    /// Add or replace an asset whose original is the file at `path`.
    pub fn put(&self, id: &str, path: impl Into<PathBuf>, modified_ms: i64) {
        let path = path.into();
        let (size, mime) = (
            std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0),
            match path.extension().and_then(|e| e.to_str()) {
                Some("mp4") => "video/mp4",
                Some("png") => "image/png",
                _ => "image/jpeg",
            },
        );
        let mut s = self.state.lock().unwrap();
        s.generation += 1;
        let g = s.generation;
        let asset = Asset {
            id: id.into(),
            modified_ms,
            size,
            mime: mime.into(),
        };
        s.assets.retain(|(a, _, _)| a.id != id);
        s.assets.push((asset, path, g));
    }

    pub fn remove(&self, id: &str) {
        let mut s = self.state.lock().unwrap();
        s.generation += 1;
        let g = s.generation;
        s.assets.retain(|(a, _, _)| a.id != id);
        s.removed.push((id.into(), g));
    }

    pub fn set_permission(&self, p: Permission) {
        self.state.lock().unwrap().permission = p;
    }

    pub fn set_conditions(&self, c: Conditions) {
        self.state.lock().unwrap().conditions = c;
    }

    pub fn make_unreadable(&self, id: &str) {
        self.state.lock().unwrap().unreadable.push(id.into());
    }

    /// The files the person "chooses" next time the app asks.
    pub fn set_picks(&self, files: Vec<PathBuf>) {
        self.state.lock().unwrap().picks = files;
    }

    /// Ids whose original was read, in order (to check what the pipeline touched).
    pub fn reads(&self) -> Vec<String> {
        self.state.lock().unwrap().reads.clone()
    }
}

impl PhotoSource for FakePhotoSource {
    fn permission(&self) -> BoxFuture<'_, Result<Permission>> {
        Box::pin(async move { Ok(self.state.lock().unwrap().permission) })
    }
    fn request_permission(&self) -> BoxFuture<'_, Result<Permission>> {
        self.permission()
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
                    .filter(|(_, _, g)| *g > since)
                    .map(|(a, _, _)| a.clone())
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
    fn read_original(&self, id: &str) -> BoxFuture<'_, Result<PathBuf>> {
        let id = id.to_string();
        Box::pin(async move {
            let src = {
                let mut s = self.state.lock().unwrap();
                s.reads.push(id.clone());
                if s.unreadable.contains(&id) {
                    return Err(anyhow!(
                        "{id} is only in iCloud and cannot be downloaded now"
                    ));
                }
                s.assets
                    .iter()
                    .find(|(a, _, _)| a.id == id)
                    .map(|(_, p, _)| p.clone())
                    .ok_or_else(|| anyhow!("no such asset"))?
            };
            std::fs::create_dir_all(&self.dir)?;
            let copy = self.dir.join(format!("read-{id}"));
            std::fs::copy(src, &copy)?;
            Ok(copy)
        })
    }
    fn conditions(&self) -> BoxFuture<'_, Result<Conditions>> {
        Box::pin(async move { Ok(self.state.lock().unwrap().conditions) })
    }
    fn pick(&self, _files: bool) -> BoxFuture<'_, Result<Vec<String>>> {
        Box::pin(async move {
            let picks = std::mem::take(&mut self.state.lock().unwrap().picks);
            Ok(picks.iter().map(|p| p.display().to_string()).collect())
        })
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
