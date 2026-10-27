//! Camera-roll backup (M4-05): incremental import from a [`PhotoSource`] with the sync
//! conditions of mockup 5e (Wi-Fi only, charging only, pause on low battery).
//!
//! State lives in `backup.json`: the policy, the platform change token, the assets still to
//! import (`pending`) and the ones in the library (`tracked`: id → photo, file and modified time,
//! so an edited photo is imported again). A scan merges what changed into `pending` and saves it
//! *before* the token advances, so nothing is lost if the app dies between enumerating and
//! importing.
//!
//! The gallery stays the owner of its photos (A-092): originals are referenced where they are,
//! never copied, and when a gallery file goes the photo is deleted from the library too (Recently
//! Deleted, on every device). Copies an earlier build made are turned back into references
//! (`moving`).

use crate::core::{now_ms, AppCore};
use crate::photos_source::{Asset, Conditions, Permission, PhotoSource, Picked};
use crate::types::{AppEvent, BackupPolicy, BackupReport, BackupStatus, ImportSummary};
use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use shoebox_core::ids::Blake3Hash;
use shoebox_node::{ImportMode, ImportOptions, LibraryNode};
use shoebox_store::Library;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

const BATCH: usize = 24;
const TICK: Duration = Duration::from_secs(30);
/// More gallery deletions than this at once, and more than a quarter of the photos, wait for
/// the person to confirm: a card swapped out or a storage glitch must not empty the library.
const MASS_DELETION: usize = 50;

impl Default for BackupPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            wifi_only: true,
            charging_only: false,
            min_battery: 20,
        }
    }
}

/// A camera-roll photo in the library.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Tracked {
    /// The photo (its original's hash).
    pub hash: String,
    /// Its file in the gallery.
    pub path: String,
    pub modified_ms: i64,
}

#[derive(Serialize, Deserialize, Default)]
pub(crate) struct BackupState {
    pub policy: BackupPolicy,
    pub token: Option<String>,
    pub pending: BTreeMap<String, Asset>,
    /// Builds before in-place originals: asset id → modified time. Replaced by `tracked`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub imported: BTreeMap<String, i64>,
    /// Camera-roll photos in the library, by asset id.
    #[serde(default)]
    pub tracked: BTreeMap<String, Tracked>,
    /// Stored copies being turned back into references: photo hash → gallery path.
    #[serde(default)]
    pub moving: BTreeMap<String, String>,
    /// Gallery deletions waiting for the person to confirm (asset ids), see [`MASS_DELETION`].
    #[serde(default)]
    pub held_deletions: Vec<String>,
    /// The photo access the last gallery check ran with: a file that vanishes when access
    /// shrinks was unshared, not deleted.
    #[serde(default)]
    pub last_permission: Option<Permission>,
    pub last_scan_ms: u64,
    pub last_error: Option<String>,
}

impl BackupState {
    pub(crate) fn load(dir: &std::path::Path) -> Self {
        let mut st: Self = std::fs::read(dir.join("backup.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        if !st.imported.is_empty() && st.tracked.is_empty() {
            // An earlier build copied what it imported and did not note which photo each asset
            // became: look at the whole roll again. Each comes back as a duplicate, is tracked,
            // and its copy is turned into a reference.
            st.imported.clear();
            st.token = None;
        }
        st
    }

    fn save(&self, dir: &std::path::Path) -> Result<()> {
        let tmp = dir.join("backup.json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(self)?)?;
        std::fs::rename(tmp, dir.join("backup.json"))?;
        Ok(())
    }
}

/// Why backup is waiting, if it is. Pure, so the rules are tested without a phone.
pub fn pause_reason(policy: &BackupPolicy, c: &Conditions) -> Option<&'static str> {
    if policy.wifi_only && !c.on_wifi {
        Some("Waiting for Wi-Fi")
    } else if policy.charging_only && !c.charging {
        Some("Waiting for the charger")
    } else if !c.charging && c.battery_pct < policy.min_battery {
        Some("Battery is low")
    } else {
        None
    }
}

impl AppCore {
    /// Give the app the camera roll of this device and start the background loop.
    pub fn set_photo_source(self: &Arc<Self>, source: Arc<dyn PhotoSource>) {
        *self.photo_source.lock().unwrap() = Some(source);
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(TICK).await;
                let Some(core) = weak.upgrade() else { break };
                if core.backup.lock().await.policy.enabled {
                    let _ = core.backup_run(true).await;
                } else {
                    // Photos added before backup was turned off still follow the gallery.
                    let _ = core.gallery_check().await;
                }
            }
        });
    }

    fn source(&self) -> Result<Arc<dyn PhotoSource>> {
        self.photo_source
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| anyhow!("this device has no camera roll to back up"))
    }

    pub async fn backup_status(&self) -> Result<BackupStatus> {
        let source = self.photo_source.lock().unwrap().clone();
        let (permission, conditions) = match source {
            None => ("unavailable".to_string(), None),
            Some(src) => {
                let perm = src.permission().await.unwrap_or(Permission::Denied);
                (
                    format!("{perm:?}").to_lowercase(),
                    src.conditions().await.ok(),
                )
            }
        };
        let st = self.backup.lock().await;
        let paused = self
            .viewer_pause()
            .or_else(|| conditions.and_then(|c| pause_reason(&st.policy, &c)))
            .map(String::from);
        Ok(self.status_of(&st, permission, paused))
    }

    /// A viewer may not add photos to the library (its role can change later, A-086).
    fn viewer_pause(&self) -> Option<&'static str> {
        (self.node().config().role == shoebox_core::model::Role::Viewer)
            .then_some("this device is a viewer, and viewers do not add photos")
    }

    fn status_of(
        &self,
        st: &BackupState,
        permission: String,
        paused: Option<String>,
    ) -> BackupStatus {
        BackupStatus {
            policy: st.policy.clone(),
            permission,
            imported: st.tracked.len() as u32,
            pending: st.pending.len() as u32,
            gallery_deletions_held: st.held_deletions.len() as u32,
            paused: if st.policy.enabled { paused } else { None },
            last_scan_ms: st.last_scan_ms as f64,
            last_error: st.last_error.clone(),
        }
    }

    /// Save the rules. Does not import by itself: call [`AppCore::backup_run`] next (the UI does,
    /// and the background loop picks it up within a minute anyway).
    pub async fn backup_configure(self: &Arc<Self>, policy: BackupPolicy) -> Result<BackupStatus> {
        let source = self.source()?;
        let mut perm = source.permission().await?;
        if policy.enabled && perm == Permission::Denied {
            perm = source.request_permission().await?;
            if perm == Permission::Denied {
                bail!("photo access was not granted; allow it in the system settings to back up your camera roll");
            }
        }
        {
            let mut st = self.backup.lock().await;
            st.policy = policy;
            st.save(self.data_dir())?;
        }
        self.backup_status().await
    }

    /// Look for new photos and (when the conditions allow and `import`) import what is waiting.
    pub async fn backup_run(self: &Arc<Self>, import: bool) -> Result<BackupReport> {
        // One scan/import at a time (the background loop and a manual run can overlap).
        let _running = self.backup_running.lock().await;
        let source = self.source()?;
        let ln = self.ln().await?;
        match source.permission().await? {
            Permission::Denied => bail!("photo access was not granted"),
            Permission::Granted | Permission::Limited => {}
        }
        let policy = self.backup.lock().await.policy.clone();
        let paused = source
            .conditions()
            .await
            .ok()
            .and_then(|c| pause_reason(&policy, &c));
        // The fetcher follows the same rules: no big downloads on mobile data or a low battery.
        if paused.is_some() {
            ln.pause_fetcher();
        } else if ln.fetcher_paused() {
            ln.resume_fetcher();
        }
        self.emit(AppEvent::BackupPaused {
            reason: paused.map(String::from),
        });

        // 1. Enumerate and remember what is new, before the token moves.
        let token = self.backup.lock().await.token.clone();
        let scan = source.scan(token).await?;
        {
            let mut st = self.backup.lock().await;
            for a in scan.assets {
                if st.tracked.get(&a.id).map(|t| t.modified_ms) != Some(a.modified_ms) {
                    st.pending.insert(a.id.clone(), a);
                }
            }
            for id in &scan.removed {
                st.pending.remove(id);
            }
            st.token = Some(scan.token);
            st.last_scan_ms = now_ms();
            st.save(self.data_dir())?;
        }
        // 2. What the gallery deleted, and copies still waiting to become references. Local work:
        // no conditions apply.
        self.check_gallery(&*source, &ln).await?;
        self.continue_moves(&ln).await?;

        let mut report = BackupReport::default();
        if let Some(reason) = self.viewer_pause() {
            self.emit(AppEvent::BackupPaused {
                reason: Some(reason.into()),
            });
            report.pending = self.backup.lock().await.pending.len() as u32;
            return Ok(report);
        }
        if !import || paused.is_some() {
            report.pending = self.backup.lock().await.pending.len() as u32;
            return Ok(report);
        }

        // 3. Import what is waiting, in place.
        let todo: Vec<Asset> = self.backup.lock().await.pending.values().cloned().collect();
        self.import_in_place(&ln, todo, &mut report, true).await?;
        report.pending = self.backup.lock().await.pending.len() as u32;
        Ok(report)
    }

    /// Add gallery photos by reference and track them. An edited photo (same asset, new content)
    /// replaces its old version, which the gallery no longer has: that one is deleted.
    async fn import_in_place(
        &self,
        ln: &LibraryNode<Library>,
        assets: Vec<Asset>,
        report: &mut BackupReport,
        progress: bool,
    ) -> Result<()> {
        let opts = ImportOptions {
            mode: ImportMode::Reference,
            ..ImportOptions::default()
        };
        let total = assets.len() as u32;
        let mut done = 0u32;
        for chunk in assets.chunks(BATCH) {
            let mut files: Vec<(Asset, PathBuf)> = vec![];
            for a in chunk {
                match &a.path {
                    Some(p) => files.push((a.clone(), PathBuf::from(p))),
                    None => report.failed.push(format!(
                        "{}: Shoebox cannot read this photo where it is, and does not keep a second copy",
                        a.id
                    )),
                }
            }
            let paths: Vec<PathBuf> = files.iter().map(|(_, p)| p.clone()).collect();
            let r = ln.import_paths(&paths, opts.clone()).await?;
            report.imported += r.imported.len() as u32;
            report.duplicates += r.duplicates.len() as u32;
            let mut replaced: Vec<String> = vec![];
            {
                let mut st = self.backup.lock().await;
                for (a, p) in &files {
                    if let Some((_, why)) = r.failed.iter().find(|(fp, _)| fp == p) {
                        report.failed.push(format!("{}: {why}", a.id));
                        continue;
                    }
                    st.pending.remove(&a.id);
                    let Some((_, id)) = r.by_path.iter().find(|(bp, _)| bp == p) else {
                        continue; // not a photo or video
                    };
                    let hash = id.to_string();
                    let before = st.tracked.insert(
                        a.id.clone(),
                        Tracked {
                            hash: hash.clone(),
                            path: p.display().to_string(),
                            modified_ms: a.modified_ms,
                        },
                    );
                    if let Some(old) = before.filter(|old| old.hash != hash) {
                        replaced.push(old.hash);
                    }
                    // Already held, but as a copy an earlier build made: make it a reference.
                    if r.duplicates.contains(id)
                        && !st.moving.contains_key(&hash)
                        && ln.node().blobs().has(id).await?
                        && !ln.is_in_place(id).await?
                    {
                        ln.begin_move_in_place(id).await?;
                        st.moving.insert(hash, p.display().to_string());
                    }
                }
                replaced.retain(|h| !st.tracked.values().any(|t| &t.hash == h));
                st.last_error = report.failed.last().cloned();
                st.save(self.data_dir())?;
            }
            ln.gallery_removed(&hashes(&replaced)).await?;
            done += chunk.len() as u32;
            if progress {
                self.emit(AppEvent::BackupProgress { done, total });
            }
        }
        Ok(())
    }

    /// Reference the gallery file for copies whose collection has happened.
    async fn continue_moves(&self, ln: &LibraryNode<Library>) -> Result<()> {
        let moving = self.backup.lock().await.moving.clone();
        if moving.is_empty() {
            return Ok(());
        }
        let mut finished = vec![];
        for (hash, path) in moving {
            let Ok(id) = crate::core::parse_hash(&hash) else {
                finished.push(hash);
                continue;
            };
            match ln.finish_move_in_place(&id, Path::new(&path)).await {
                Ok(true) => finished.push(hash),
                Ok(false) => {}
                // The file went or changed: the gallery check deals with it.
                Err(e) => {
                    tracing::warn!("referencing {path} again: {e:#}");
                    finished.push(hash);
                }
            }
        }
        let mut st = self.backup.lock().await;
        for h in &finished {
            st.moving.remove(h);
        }
        st.save(self.data_dir())
    }

    /// Look for gallery deletions now (the background loop does this when backup is off).
    pub async fn gallery_check(&self) -> Result<()> {
        let _running = self.backup_running.lock().await;
        let source = self.source()?;
        let ln = self.ln().await?;
        self.check_gallery(&*source, &ln).await
    }

    /// Photos whose gallery file is gone are deleted from the library. A file on a card that is not
    /// mounted is not gone.
    ///
    /// With only some photos shared, Android hides every other file, so a deleted file and an
    /// unshared one look the same. A shared file stops being visible when the person deletes it
    /// or unshares it, and unsharing happens when access changes: through Shoebox's own "share
    /// more" prompt (see [`AppCore::import_picked`]) or by narrowing access in the system
    /// settings. Files that vanish across such a change are unshared: they stay in the library
    /// and this phone stops holding them. Any other disappearance is a deletion. Without access,
    /// nothing is checked.
    async fn check_gallery(
        &self,
        source: &dyn PhotoSource,
        ln: &LibraryNode<Library>,
    ) -> Result<()> {
        let permission = source.permission().await?;
        if permission == Permission::Denied {
            return Ok(());
        }
        let narrowed = {
            let mut st = self.backup.lock().await;
            let before = st.last_permission.replace(permission);
            if before != Some(permission) {
                st.save(self.data_dir())?;
            }
            before == Some(Permission::Granted) && permission == Permission::Limited
        };
        if narrowed {
            return self.release_unshared(ln).await;
        }
        let (gone, total) = {
            let mut st = self.backup.lock().await;
            st.pending.retain(|_, a| {
                a.path
                    .as_deref()
                    .is_none_or(|p| !gone_from_gallery(Path::new(p)))
            });
            let gone: Vec<String> = st
                .tracked
                .iter()
                .filter(|(_, t)| gone_from_gallery(Path::new(&t.path)))
                .map(|(id, _)| id.clone())
                .collect();
            (gone, st.tracked.len())
        };
        if gone.len() > MASS_DELETION && gone.len() * 4 > total {
            let mut st = self.backup.lock().await;
            if st.held_deletions != gone {
                tracing::warn!(
                    "{} of {total} gallery photos disappeared at once: waiting for confirmation",
                    gone.len()
                );
                st.held_deletions = gone.clone();
                st.save(self.data_dir())?;
                self.emit(AppEvent::GalleryDeletionsHeld {
                    count: gone.len() as u32,
                });
            }
            return Ok(());
        }
        self.apply_gallery_deletions(ln, &gone, true).await
    }

    /// Access just shrank: what this phone can no longer see was unshared. Keep those photos in
    /// the library; this phone no longer holds them.
    async fn release_unshared(&self, ln: &LibraryNode<Library>) -> Result<()> {
        let unshared: Vec<String> = {
            let st = self.backup.lock().await;
            st.tracked
                .iter()
                .filter(|(_, t)| gone_from_gallery(Path::new(&t.path)))
                .map(|(id, _)| id.clone())
                .collect()
        };
        if !unshared.is_empty() {
            tracing::info!("{} photo(s) no longer shared with Shoebox", unshared.len());
        }
        self.apply_gallery_deletions(ln, &unshared, false).await
    }

    /// An original this phone references cannot be read (the viewer found out). Look at the
    /// gallery now: a deleted file deletes the photo. If it is still referenced after that (the
    /// file changed, or it was unshared), stop holding it so it is fetched from another device.
    pub(crate) async fn original_unreadable(&self, hash: &Blake3Hash) -> Result<()> {
        let ln = self.ln().await?;
        if self.photo_source.lock().unwrap().is_some() {
            let _ = self.gallery_check().await;
        }
        if ln.is_in_place(hash).await? {
            tracing::warn!("the gallery file of {hash} cannot be read: fetching it instead");
            let ids: Vec<String> = {
                let st = self.backup.lock().await;
                st.tracked
                    .iter()
                    .filter(|(_, t)| t.hash == hash.to_string())
                    .map(|(id, _)| id.clone())
                    .collect()
            };
            if ids.is_empty() {
                ln.release_in_place(&[*hash]).await?;
            } else {
                self.apply_gallery_deletions(&ln, &ids, false).await?;
            }
        }
        Ok(())
    }

    /// The person's answer about held gallery deletions: delete those photos from the library
    /// too, or keep them there (this phone no longer has them either way).
    pub async fn answer_gallery_deletions(&self, delete: bool) -> Result<()> {
        let ln = self.ln().await?;
        let held = std::mem::take(&mut self.backup.lock().await.held_deletions);
        self.apply_gallery_deletions(&ln, &held, delete).await?;
        self.emit(AppEvent::GalleryDeletionsHeld { count: 0 });
        Ok(())
    }

    async fn apply_gallery_deletions(
        &self,
        ln: &LibraryNode<Library>,
        ids: &[String],
        delete: bool,
    ) -> Result<()> {
        let gone: Vec<String> = {
            let mut st = self.backup.lock().await;
            let mut gone: Vec<String> = ids
                .iter()
                .filter_map(|id| st.tracked.remove(id))
                .map(|t| t.hash)
                .collect();
            st.held_deletions.retain(|id| !ids.contains(id));
            // The same photo can be in the gallery twice: it stays while one copy remains.
            gone.retain(|h| !st.tracked.values().any(|t| &t.hash == h));
            gone.sort();
            gone.dedup();
            for h in &gone {
                st.moving.remove(h);
            }
            st.save(self.data_dir())?;
            gone
        };
        if gone.is_empty() {
            return Ok(());
        }
        tracing::info!(
            "{} photo(s) gone from the gallery: {}",
            gone.len(),
            if delete {
                "deleted from the library"
            } else {
                "kept in the library"
            }
        );
        if delete {
            ln.gallery_removed(&hashes(&gone)).await
        } else {
            ln.release_in_place(&hashes(&gone)).await
        }
    }

    /// "Add photos" on a phone. Gallery photos are added where they are, like the camera roll;
    /// a file from elsewhere (a cloud drive, a USB stick) is copied, since the phone has no other
    /// copy. With access to only some photos, "from the gallery" asks Android to share more and
    /// adds what it then sees: a picked gallery photo the app cannot read in place would have to
    /// be copied, and the phone would keep it twice.
    pub async fn import_picked(self: &Arc<Self>, files: bool) -> Result<ImportSummary> {
        let ln = self.ln().await?;
        let source = self.source()?;
        let mut summary = ImportSummary {
            imported: 0,
            duplicates: 0,
            skipped: 0,
            failed: vec![],
        };
        if !files && source.permission().await? != Permission::Granted {
            // Deletions so far count as deletions; what vanishes across the prompt was unshared.
            let _ = self.check_gallery(&*source, &ln).await;
            let answer = source.request_permission().await?;
            {
                let _running = self.backup_running.lock().await;
                self.release_unshared(&ln).await?;
                self.backup.lock().await.last_permission = Some(answer);
            }
            if answer == Permission::Denied {
                bail!("Shoebox needs access to your photos to add them from the gallery");
            }
            // What is newly shared shows up in a full look at the roll.
            let _running = self.backup_running.lock().await;
            let scan = source.scan(None).await?;
            let todo: Vec<Asset> = {
                let st = self.backup.lock().await;
                scan.assets
                    .into_iter()
                    .filter(|a| st.tracked.get(&a.id).map(|t| t.modified_ms) != Some(a.modified_ms))
                    .collect()
            };
            let mut report = BackupReport::default();
            self.import_in_place(&ln, todo, &mut report, false).await?;
            summary.imported = report.imported;
            summary.duplicates = report.duplicates;
            summary.failed = report.failed;
            return Ok(summary);
        }

        let picked = source.pick(files).await?;
        let mut in_place = vec![];
        let mut copies = vec![];
        for p in picked {
            match p {
                Picked::InPlace(a) => in_place.push(a),
                Picked::Copy(handle) => copies.push(handle),
                Picked::Unreadable(name) => summary.failed.push(format!(
                    "{name}: share all photos with Shoebox to add gallery photos (it does not keep a second copy)"
                )),
            }
        }
        if !in_place.is_empty() {
            let _running = self.backup_running.lock().await;
            let mut report = BackupReport::default();
            self.import_in_place(&ln, in_place, &mut report, false)
                .await?;
            summary.imported += report.imported;
            summary.duplicates += report.duplicates;
            summary.failed.extend(report.failed);
        }
        // Copies: a batch at a time, each temporary file deleted after its batch; progress reads
        // as one run.
        let total = copies.len() as u32;
        let mut done = 0u32;
        for batch in copies.chunks(BATCH) {
            let mut paths = vec![];
            for handle in batch {
                match source.read_picked(handle).await {
                    Ok(p) => paths.push(p),
                    Err(e) => summary.failed.push(format!("{e:#}")),
                }
            }
            *self.import_batches.lock().unwrap() =
                Some((done + (batch.len() - paths.len()) as u32, total));
            let result = ln.import_paths(&paths, ImportOptions::default()).await;
            *self.import_batches.lock().unwrap() = None;
            for p in &paths {
                let _ = std::fs::remove_file(p);
            }
            done += batch.len() as u32;
            let r = result?;
            summary.imported += r.imported.len() as u32;
            summary.duplicates += r.duplicates.len() as u32;
            summary.skipped += r.skipped.len() as u32;
            summary.failed.extend(
                r.failed
                    .iter()
                    .map(|(p, why)| format!("{}: {why}", p.display())),
            );
        }
        Ok(summary)
    }
}

fn hashes(hex: &[String]) -> Vec<Blake3Hash> {
    hex.iter()
        .filter_map(|h| crate::core::parse_hash(h).ok())
        .collect()
}

/// The storage a gallery file lives on: `/storage/emulated/<user>` or `/storage/<card>` on
/// Android, the file's folder elsewhere (tests).
fn volume_root(path: &Path) -> Option<PathBuf> {
    let parts: Vec<_> = path.components().collect();
    let take = match parts.get(1).and_then(|c| c.as_os_str().to_str()) {
        Some("storage") if parts.get(2).is_some_and(|c| c.as_os_str() == "emulated") => 4,
        Some("storage") => 3,
        _ => return path.parent().map(Path::to_path_buf),
    };
    (parts.len() > take).then(|| parts[..take].iter().collect())
}

/// The file is gone while the storage it was on is still there: the person deleted it.
fn gone_from_gallery(path: &Path) -> bool {
    match std::fs::symlink_metadata(path) {
        Ok(_) => false,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            volume_root(path).is_some_and(|v| v.is_dir())
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::volume_root;
    use std::path::{Path, PathBuf};

    #[test]
    fn the_volume_of_a_gallery_file() {
        assert_eq!(
            volume_root(Path::new("/storage/emulated/0/DCIM/Camera/a.jpg")),
            Some(PathBuf::from("/storage/emulated/0"))
        );
        assert_eq!(
            volume_root(Path::new("/storage/1A2B-3C4D/DCIM/a.jpg")),
            Some(PathBuf::from("/storage/1A2B-3C4D"))
        );
        assert_eq!(
            volume_root(Path::new("/tmp/gallery/a.jpg")),
            Some(PathBuf::from("/tmp/gallery"))
        );
    }
}
