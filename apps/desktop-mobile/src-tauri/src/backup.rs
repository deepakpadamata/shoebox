//! Camera-roll backup (M4-05): incremental import from a [`PhotoSource`] with the sync
//! conditions of mockup 5e (Wi-Fi only, charging only, pause on low battery).
//!
//! State lives in `backup.json`: the policy, the platform change token, the assets still to
//! import (`pending`) and the ones already imported (`imported`, id → modified time, so an edited
//! photo is imported again). A scan merges what changed into `pending` and saves it *before* the
//! token advances, so nothing is lost if the app dies between enumerating and importing.

use crate::core::{now_ms, AppCore};
use crate::photos_source::{Asset, Conditions, Permission, PhotoSource};
use crate::types::{AppEvent, BackupPolicy, BackupReport, BackupStatus};
use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use shoebox_node::ImportOptions;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const BATCH: usize = 24;
const TICK: Duration = Duration::from_secs(30);

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

#[derive(Serialize, Deserialize, Default)]
pub(crate) struct BackupState {
    pub policy: BackupPolicy,
    pub token: Option<String>,
    pub pending: BTreeMap<String, Asset>,
    pub imported: BTreeMap<String, i64>,
    pub last_scan_ms: u64,
    pub last_error: Option<String>,
}

impl BackupState {
    pub(crate) fn load(dir: &std::path::Path) -> Self {
        std::fs::read(dir.join("backup.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
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
            imported: st.imported.len() as u32,
            pending: st.pending.len() as u32,
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
                if st.imported.get(&a.id) != Some(&a.modified_ms) {
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

        // 2. Import in batches: read each original, hand the files to the importer, clean up.
        let todo: Vec<Asset> = self.backup.lock().await.pending.values().cloned().collect();
        let total = todo.len() as u32;
        let mut done = 0u32;
        for chunk in todo.chunks(BATCH) {
            let mut files: Vec<(Asset, PathBuf)> = vec![];
            for a in chunk {
                match source.read_original(&a.id).await {
                    Ok(p) => files.push((a.clone(), p)),
                    Err(e) => report.failed.push(format!("{}: {e:#}", a.id)),
                }
            }
            let paths: Vec<PathBuf> = files.iter().map(|(_, p)| p.clone()).collect();
            let result = ln.import_paths(&paths, ImportOptions::default()).await;
            for p in &paths {
                let _ = std::fs::remove_file(p);
            }
            let r = result?;
            report.imported += r.imported.len() as u32;
            report.duplicates += r.duplicates.len() as u32;
            let failed_paths: std::collections::HashSet<&PathBuf> =
                r.failed.iter().map(|(p, _)| p).collect();
            {
                let mut st = self.backup.lock().await;
                for (a, p) in &files {
                    if failed_paths.contains(p) {
                        let why = r
                            .failed
                            .iter()
                            .find(|(fp, _)| fp == p)
                            .map(|(_, w)| w.clone())
                            .unwrap_or_default();
                        report.failed.push(format!("{}: {why}", a.id));
                    } else {
                        // Imported, a duplicate, or not media: nothing more to do for it.
                        st.pending.remove(&a.id);
                        st.imported.insert(a.id.clone(), a.modified_ms);
                    }
                }
                st.last_error = report.failed.last().cloned();
                st.save(self.data_dir())?;
            }
            done += chunk.len() as u32;
            self.emit(AppEvent::BackupProgress { done, total });
        }
        report.pending = self.backup.lock().await.pending.len() as u32;
        Ok(report)
    }
}
