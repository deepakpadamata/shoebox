//! Delete, Recently Deleted, purge and blob garbage collection (M1-13).
//!
//! A delete is a soft tombstone replicated to every device, recoverable for 30 days. A purge
//! removes the record everywhere. Blobs are then reclaimed in two steps that every device runs
//! for itself: **reconcile** drops this device's library protection on blobs no record references
//! any more (and retracts the matching holdings), and the blob store's GC deletes whatever has no
//! protection left. A shared-album reference is a protection of its own, so it keeps a blob alive.

use crate::library_node::LibraryNode;
use anyhow::Result;
use shoebox_core::ids::{Blake3Hash, Timestamp};
use shoebox_core::model::{PhotoId, PhotoRecord};
use shoebox_core::store::{LibraryStore, Page, TimeRange};
use std::collections::HashSet;
use std::sync::Weak;
use std::time::Duration;
use tracing::{debug, warn};

impl<L: LibraryStore> LibraryNode<L> {
    /// Move photos to Recently Deleted (recoverable for 30 days). Unknown ids are skipped.
    pub async fn delete_photos(&self, ids: &[PhotoId]) -> Result<()> {
        for id in ids {
            match self.store().mark_deleted(id).await {
                Ok(()) | Err(shoebox_core::store::StoreError::NotFound) => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }

    /// Bring photos back from Recently Deleted.
    pub async fn restore_photos(&self, ids: &[PhotoId]) -> Result<()> {
        for id in ids {
            match self.store().restore_photo(id).await {
                Ok(()) | Err(shoebox_core::store::StoreError::NotFound) => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }

    /// Soft-deleted photos, most recently deleted first.
    pub async fn recently_deleted(&self) -> Result<Vec<PhotoRecord>> {
        Ok(self.store().deleted_photos().await?)
    }

    /// Permanently remove photos deleted at least 30 days ago (by this node's clock), then release
    /// the blobs nothing references any more. Returns the purged photo ids.
    pub async fn purge_expired(&self) -> Result<Vec<PhotoId>> {
        let now = Timestamp(self.now_ms() as i64);
        let purged = self.store().purge_expired(now).await?;
        if !purged.is_empty() {
            self.reconcile_blobs().await?;
        }
        Ok(purged)
    }

    /// Release this device's library protection (and pins) on blobs that no photo record
    /// references, and retract their holdings. Soft-deleted photos still count as references.
    /// Returns the released blobs; the blob store's GC reclaims them shortly after.
    pub async fn reconcile_blobs(&self) -> Result<Vec<Blake3Hash>> {
        // Importing tags a blob before its record exists; do not release one mid-import.
        let _exclusive = self.inner.ingest.write().await;
        let mut referenced: HashSet<Blake3Hash> = HashSet::new();
        let live = self
            .store()
            .photos_by_time(TimeRange::ALL, Page::all())
            .await?;
        let deleted = self.store().deleted_photos().await?;
        for rec in live.iter().chain(deleted.iter()) {
            referenced.extend([rec.original.hash, rec.preview.hash, rec.thumb.hash]);
        }
        let mut released: Vec<Blake3Hash> = self
            .node()
            .blobs()
            .library_protected()
            .await?
            .into_iter()
            .filter(|h| !referenced.contains(h))
            .collect();
        released.sort();
        for hash in &released {
            self.node().blobs().remove(hash).await?;
        }
        if !released.is_empty() {
            self.retract_holdings(released.clone()).await?;
            let hexes: HashSet<String> = released.iter().map(|h| h.to_string()).collect();
            self.node()
                .update_config(|c| c.pinned_originals.retain(|p| !hexes.contains(p)))?;
            debug!("released {} unreferenced blob(s)", released.len());
        }
        Ok(released)
    }

    /// Keep this device tidy: every `cfg.interval`, and shortly after the library reports a delete
    /// or purge (including ones that arrive from other devices), run
    /// [`LibraryNode::purge_expired`] and [`LibraryNode::reconcile_blobs`]. Every device does this
    /// for itself; purges replicate.
    pub fn start_maintenance(&self, cfg: MaintenanceConfig) {
        let wake = std::sync::Arc::new(tokio::sync::Notify::new());
        let signal = wake.clone();
        let weak_changes = std::sync::Arc::downgrade(&self.inner);
        let mut changes = self.inner.store.subscribe();
        tokio::spawn(async move {
            use shoebox_core::store::Change;
            while let Some(change) = futures::StreamExt::next(&mut changes).await {
                if weak_changes.upgrade().is_none() {
                    break;
                }
                if matches!(change, Change::PhotoDeleted(_)) {
                    signal.notify_one();
                }
            }
        });
        let weak = std::sync::Arc::downgrade(&self.inner);
        tokio::spawn(maintenance_loop(weak, cfg, wake));
    }
}

/// When [`LibraryNode::start_maintenance`] runs.
#[derive(Clone, Debug)]
pub struct MaintenanceConfig {
    /// Run at least this often.
    pub interval: Duration,
    /// After a delete/purge notification, wait this long (to batch bursts) before running.
    pub react_after: Duration,
}

impl Default for MaintenanceConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(3600),
            react_after: Duration::from_secs(5),
        }
    }
}

async fn maintenance_loop<L: LibraryStore>(
    weak: Weak<crate::library_node::Inner<L>>,
    cfg: MaintenanceConfig,
    wake: std::sync::Arc<tokio::sync::Notify>,
) {
    loop {
        let triggered = tokio::time::timeout(cfg.interval, wake.notified())
            .await
            .is_ok();
        if triggered {
            tokio::time::sleep(cfg.react_after).await;
        }
        let Some(inner) = weak.upgrade() else { break };
        let ln = LibraryNode { inner };
        if let Err(e) = ln.purge_expired().await {
            warn!("purge failed: {e:#}");
        }
        if let Err(e) = ln.reconcile_blobs().await {
            warn!("blob reconcile failed: {e:#}");
        }
    }
}
