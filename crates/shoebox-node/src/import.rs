//! Folder import (M1-06): walk, analyze, store blobs, upsert records, publish holdings, dedupe by
//! photo id, and keep watched folders in sync.

use crate::blobs::{ImportMode, Protection};
use crate::library_node::LibraryNode;
use crate::node::NodeEvent;
use anyhow::{Context, Result};
use futures::StreamExt;
use notify::{EventKind, RecursiveMode, Watcher};
use shoebox_core::hlc::Hlc;
use shoebox_core::ids::Blake3Hash;
use shoebox_core::merge::Lww;
use shoebox_core::model::{BlobKind, BlobRef, PhotoId, PhotoRecord};
use shoebox_core::store::LibraryStore;
use shoebox_media::{analyze, AnalyzeError, ImportResult, MediaKind, Rendition};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tracing::{debug, warn};

/// Wait this long after the last file-system event before importing (files are often still
/// being written when the first event arrives).
const WATCH_DEBOUNCE: Duration = Duration::from_millis(400);
/// Emit a progress event every this many files (and always at the end).
const PROGRESS_EVERY: usize = 10;
/// Publish holdings after this many newly stored blobs, so a crash loses little.
const HOLDINGS_BATCH: usize = 300;

#[derive(Clone, Debug)]
pub struct ImportOptions {
    /// How originals enter the blob store.
    pub mode: ImportMode,
    /// Files analysed in parallel (analysis is CPU-bound).
    pub concurrency: usize,
    /// This call is one part of a larger run: `(files already done, the run's total)`. Progress
    /// events then count the whole run, so whoever forwards them needs no state of its own (that
    /// state, read after the call returned, once mislabelled its last event).
    pub part_of: Option<(usize, usize)>,
}

impl Default for ImportOptions {
    fn default() -> Self {
        let cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(2);
        Self {
            mode: ImportMode::Copy,
            concurrency: cpus.clamp(1, 8),
            part_of: None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportReport {
    /// Photos added to the library by this run.
    pub imported: Vec<PhotoId>,
    /// Files whose content is already in the library (no new record).
    pub duplicates: Vec<PhotoId>,
    /// Files that are not media this app understands.
    pub skipped: Vec<PathBuf>,
    /// Files that look like media but could not be read or decoded, with the reason.
    pub failed: Vec<(PathBuf, String)>,
    /// Every file that is in the library after this run (imported or a duplicate), with its photo.
    pub by_path: Vec<(PathBuf, PhotoId)>,
}

/// Every regular, non-hidden file under `paths` (directories are walked recursively), sorted.
fn collect_files(paths: &[PathBuf], failed: &mut Vec<(PathBuf, String)>) -> Vec<PathBuf> {
    fn visit(dir: &Path, out: &mut BTreeSet<PathBuf>, failed: &mut Vec<(PathBuf, String)>) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) => {
                failed.push((dir.to_path_buf(), e.to_string()));
                return;
            }
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let hidden = entry.file_name().to_string_lossy().starts_with('.');
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if hidden || kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                visit(&path, out, failed);
            } else if kind.is_file() {
                out.insert(path);
            }
        }
    }
    let mut out = BTreeSet::new();
    for path in paths {
        match std::fs::metadata(path) {
            Ok(m) if m.is_dir() => visit(path, &mut out, failed),
            Ok(_) => {
                out.insert(path.clone());
            }
            Err(e) => failed.push((path.clone(), e.to_string())),
        }
    }
    out.into_iter().collect()
}

fn rendition_ref(r: &Rendition, hash: Blake3Hash) -> BlobRef {
    BlobRef {
        hash,
        size: r.bytes.len() as u64,
        mime: r.mime.clone(),
    }
}

fn record_from(
    r: &ImportResult,
    device: shoebox_core::ids::EndpointId,
    preview: BlobRef,
    thumb: BlobRef,
    register_hlc: Hlc,
) -> PhotoRecord {
    PhotoRecord {
        id: r.hash,
        original: BlobRef {
            hash: r.hash,
            size: r.size,
            mime: r.mime.clone(),
        },
        preview,
        thumb,
        taken_at: r.taken_at,
        exif: r.exif.clone(),
        imported_by: device,
        favorite: Lww::new(false, register_hlc),
        deleted_at: Lww::new(None, register_hlc),
    }
}

enum Outcome {
    Imported(PhotoId),
    Duplicate(PhotoId),
}

impl<L: LibraryStore> LibraryNode<L> {
    /// Import files and folders. Safe to repeat: content already in the library only adds a
    /// holding if this device did not hold it yet.
    pub async fn import_paths(
        &self,
        paths: &[PathBuf],
        opts: ImportOptions,
    ) -> Result<ImportReport> {
        let mut report = ImportReport::default();
        let files = collect_files(paths, &mut report.failed);
        let total = files.len();
        let mut held: Vec<(Blake3Hash, BlobKind)> = vec![];

        let mut results = futures::stream::iter(files.into_iter().map(|path| async move {
            let p = path.clone();
            let analysed = tokio::task::spawn_blocking(move || analyze(&p)).await;
            (path, analysed)
        }))
        .buffered(opts.concurrency.max(1));

        let mut done = 0usize;
        while let Some((path, analysed)) = results.next().await {
            done += 1;
            match analysed {
                Err(join) => report
                    .failed
                    .push((path, format!("analysis task failed: {join}"))),
                Ok(Err(AnalyzeError::Unsupported(_))) => report.skipped.push(path),
                Ok(Err(e)) => report.failed.push((path, e.to_string())),
                Ok(Ok(result)) => match self.ingest(&path, &result, &opts, &mut held).await {
                    Ok(Outcome::Imported(id)) => {
                        report.imported.push(id);
                        report.by_path.push((path, id));
                    }
                    Ok(Outcome::Duplicate(id)) => {
                        report.duplicates.push(id);
                        report.by_path.push((path, id));
                    }
                    Err(e) => report.failed.push((path, format!("{e:#}"))),
                },
            }
            if held.len() >= HOLDINGS_BATCH {
                self.publish_holdings(std::mem::take(&mut held)).await?;
            }
            if done.is_multiple_of(PROGRESS_EVERY) || done == total {
                let (done, total) = match opts.part_of {
                    Some((before, run)) => (before + done, run),
                    None => (done, total),
                };
                self.node().emit(NodeEvent::ImportProgress {
                    done,
                    total,
                    imported: report.imported.len(),
                    duplicates: report.duplicates.len(),
                    skipped: report.skipped.len(),
                    failed: report.failed.len(),
                });
            }
        }
        self.publish_holdings(held).await?;
        if !report.imported.is_empty() {
            // Best effort: peers also sync on their own schedule.
            let _ = self.announce_new_items(report.imported.len() as u32).await;
        }
        Ok(report)
    }

    async fn ingest(
        &self,
        path: &Path,
        r: &ImportResult,
        opts: &ImportOptions,
        held: &mut Vec<(Blake3Hash, BlobKind)>,
    ) -> Result<Outcome> {
        // A blob is protected before its record exists; keep reconcile from releasing it meanwhile.
        let _ingesting = self.inner.ingest.read().await;
        let blobs = self.node().blobs();
        let store = self.store();
        let id = r.hash;

        if store.photo(&id).await?.is_some() {
            // Known photo: keep the existing record. If only its metadata reached us (synced from
            // another device) and we now hold the file too, record that we have the original.
            if !blobs.has(&id).await? {
                let stored = blobs.add_path(path, opts.mode, Protection::Library).await?;
                anyhow::ensure!(
                    stored == id,
                    "{} changed while it was being imported",
                    path.display()
                );
                if opts.mode == ImportMode::Reference {
                    blobs.protect(&id, Protection::InPlace).await?;
                }
                held.push((id, BlobKind::Original));
            }
            return Ok(Outcome::Duplicate(id));
        }

        let stored = blobs.add_path(path, opts.mode, Protection::Library).await?;
        anyhow::ensure!(
            stored == id,
            "{} changed while it was being imported",
            path.display()
        );
        if opts.mode == ImportMode::Reference {
            blobs.protect(&id, Protection::InPlace).await?;
        }
        let original_ref = BlobRef {
            hash: id,
            size: r.size,
            mime: r.mime.clone(),
        };
        let (preview, thumb) = match (&r.preview, &r.thumb) {
            (Some(p), Some(t)) => {
                let ph = blobs
                    .add_bytes(p.bytes.clone(), Protection::Library)
                    .await?;
                let th = blobs
                    .add_bytes(t.bytes.clone(), Protection::Library)
                    .await?;
                held.push((ph, BlobKind::Preview));
                held.push((th, BlobKind::Thumb));
                (rendition_ref(p, ph), rendition_ref(t, th))
            }
            // Videos have no poster frame yet (M3-10): the original stands in.
            _ => {
                debug_assert_eq!(r.kind, MediaKind::Video);
                (original_ref.clone(), original_ref)
            }
        };
        held.push((id, BlobKind::Original));

        // Registers start at GENESIS so a favorite or delete another device already made for this
        // same file is never overridden by the act of importing it.
        let device = self.device();
        let record = record_from(r, device, preview.clone(), thumb.clone(), Hlc::GENESIS);
        store.upsert_photo(record).await?;
        if store.photo(&id).await?.is_none() {
            // A purge marker newer than GENESIS suppressed the record: this is a deliberate
            // re-import of a purged photo, so write fresh registers that outrank the marker.
            let fresh = Hlc {
                physical_ms: self.now_ms(),
                counter: 0,
                device,
            };
            store
                .upsert_photo(record_from(r, device, preview, thumb, fresh))
                .await?;
            anyhow::ensure!(
                store.photo(&id).await?.is_some(),
                "could not re-import purged photo"
            );
        }
        Ok(Outcome::Imported(id))
    }

    // ---------- watched folders ----------

    pub fn watched_folders(&self) -> Vec<PathBuf> {
        self.node().config().watched_folders
    }

    /// Import everything in `dir` now and whenever files appear in it. Persisted in `config.toml`.
    pub async fn watch_folder(&self, dir: &Path) -> Result<()> {
        let dir = std::path::absolute(dir)?;
        anyhow::ensure!(dir.is_dir(), "{} is not a folder", dir.display());
        self.node().update_config(|c| {
            if !c.watched_folders.contains(&dir) {
                c.watched_folders.push(dir.clone());
            }
        })?;
        self.start_watch(&dir)?;
        self.scan_in_background(dir);
        Ok(())
    }

    pub async fn unwatch_folder(&self, dir: &Path) -> Result<()> {
        let dir = std::path::absolute(dir)?;
        self.inner.watchers.lock().unwrap().remove(&dir);
        self.node()
            .update_config(|c| c.watched_folders.retain(|d| d != &dir))?;
        Ok(())
    }

    pub(crate) fn restore_watches(&self) {
        for dir in self.watched_folders() {
            if dir.is_dir() {
                if let Err(e) = self.start_watch(&dir) {
                    warn!("could not watch {}: {e:#}", dir.display());
                    continue;
                }
                // Files may have arrived while the node was not running.
                self.scan_in_background(dir);
            }
        }
    }

    fn scan_in_background(&self, dir: PathBuf) {
        let ln = self.clone();
        tokio::spawn(async move {
            if let Err(e) = ln
                .import_paths(std::slice::from_ref(&dir), ImportOptions::default())
                .await
            {
                warn!("scan of {} failed: {e:#}", dir.display());
            }
        });
    }

    fn start_watch(&self, dir: &Path) -> Result<()> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<PathBuf>();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(event) = res {
                if matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_)) {
                    for path in event.paths {
                        let _ = tx.send(path);
                    }
                }
            }
        })
        .context("creating file watcher")?;
        watcher
            .watch(dir, RecursiveMode::Recursive)
            .with_context(|| format!("watching {}", dir.display()))?;
        self.inner
            .watchers
            .lock()
            .unwrap()
            .insert(dir.to_path_buf(), watcher);

        // Debounce: collect paths until the folder has been quiet for a moment, then import them.
        let weak = std::sync::Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            let mut pending: BTreeSet<PathBuf> = BTreeSet::new();
            loop {
                let next = if pending.is_empty() {
                    rx.recv().await
                } else {
                    match tokio::time::timeout(WATCH_DEBOUNCE, rx.recv()).await {
                        Ok(item) => item,
                        Err(_quiet) => {
                            let Some(inner) = weak.upgrade() else { break };
                            let ln = LibraryNode { inner };
                            let batch: Vec<PathBuf> =
                                std::mem::take(&mut pending).into_iter().collect();
                            debug!("watched folder changed: importing {} path(s)", batch.len());
                            if let Err(e) = ln.import_paths(&batch, ImportOptions::default()).await
                            {
                                warn!("import of watched changes failed: {e:#}");
                            }
                            continue;
                        }
                    }
                };
                match next {
                    Some(path) => {
                        pending.insert(path);
                    }
                    None => break, // watcher dropped (unwatched)
                }
            }
        });
        Ok(())
    }
}
