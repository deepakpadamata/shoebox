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
use shoebox_core::model::{BlobKind, BlobRef, MotionRecord, MotionSource, PhotoId, PhotoRecord};
use shoebox_core::store::LibraryStore;
use shoebox_media::{analyze, AnalyzeError, EmbeddedVideo, ImportResult, MediaKind, Rendition};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tracing::{debug, warn};

/// Wait this long after the last file-system event before importing (files are often still
/// being written when the first event arrives).
const WATCH_DEBOUNCE: Duration = Duration::from_millis(400);
/// Emit a progress event every this many files (and always at the end).
pub(crate) const PROGRESS_EVERY: usize = 10;
/// Publish holdings after this many newly stored blobs, so a crash loses little.
pub(crate) const HOLDINGS_BATCH: usize = 300;
/// A Live Photo's video is a few seconds; a longer file next to a picture of the same name is a clip of
/// its own.
const MAX_COMPANION_MS: u32 = 10_000;
/// Pictures a Live Photo's video can go with, in the order one is chosen when several share a name.
const STILL_EXTENSIONS: [&str; 4] = ["heic", "heif", "jpg", "jpeg"];
/// An iPhone writes `.MOV`; Google Photos' export names the video of a Live Photo `.MP4`.
const VIDEO_EXTENSIONS: [&str; 2] = ["mov", "mp4"];
/// A poster frame is a small JPEG (480 px long edge, tens of KB); anything near this is not one.
const MAX_POSTER_BYTES: usize = 512 * 1024;

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
    /// How many of `skipped` each reason covers ("unrecognised file type (.psd)", "AVIF is not
    /// supported yet"), so a report can say what was left out by format.
    pub skipped_why: BTreeMap<String, usize>,
    /// Files that look like media but could not be read or decoded, with the reason.
    pub failed: Vec<(PathBuf, String)>,
    /// Photos that were given a video by this run (Live Photos, motion photos): not new photos.
    pub with_video: Vec<PhotoId>,
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

/// Whether a video is short enough to be the moving half of a Live Photo.
pub(crate) fn is_companion_clip(r: &ImportResult) -> bool {
    r.kind == MediaKind::Video && r.duration_ms.is_some_and(|d| d <= MAX_COMPANION_MS)
}

fn extension_of(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
}

/// Files next to each other in `dir`, by lower-case name without extension.
fn siblings(dir: &Path) -> HashMap<String, Vec<PathBuf>> {
    let mut by_stem: HashMap<String, Vec<PathBuf>> = HashMap::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let hidden = entry.file_name().to_string_lossy().starts_with('.');
        if hidden || !entry.file_type().is_ok_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.path();
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            by_stem
                .entry(stem.to_ascii_lowercase())
                .or_default()
                .push(path);
        }
    }
    by_stem
}

/// Videos that may be the moving half of a picture (`IMG_0001.MOV` next to `IMG_0001.HEIC`), each
/// with its picture. Decided from the folder, not from this batch: a watched folder can hand the two
/// over separately. Both files are added to `files` so a pair is always imported together; whether a
/// video really is a companion (short enough) is decided once it is analysed.
fn find_pairs(files: &mut Vec<PathBuf>) -> BTreeMap<PathBuf, PathBuf> {
    let mut dirs: HashMap<PathBuf, HashMap<String, Vec<PathBuf>>> = HashMap::new();
    let mut candidates: BTreeSet<PathBuf> = BTreeSet::new();
    let mut videos_of_stills: Vec<(PathBuf, String)> = vec![];
    for f in files.iter() {
        match extension_of(f).as_deref() {
            Some(e) if VIDEO_EXTENSIONS.contains(&e) => {
                candidates.insert(f.clone());
            }
            Some(e) if STILL_EXTENSIONS.contains(&e) => {
                if let (Some(dir), Some(stem)) =
                    (f.parent(), f.file_stem().and_then(|s| s.to_str()))
                {
                    videos_of_stills.push((dir.to_path_buf(), stem.to_ascii_lowercase()));
                }
            }
            _ => {}
        }
    }
    // A video may sit next to a picture that was handed over without it.
    for (dir, stem) in videos_of_stills {
        let group = dirs.entry(dir.clone()).or_insert_with(|| siblings(&dir));
        for e in VIDEO_EXTENSIONS {
            if let Some(v) = group
                .get(&stem)
                .into_iter()
                .flatten()
                .find(|p| extension_of(p).as_deref() == Some(e))
            {
                candidates.insert(v.clone());
            }
        }
    }
    let mut pairs = BTreeMap::new();
    for video in candidates {
        let (Some(dir), Some(stem)) = (video.parent(), video.file_stem().and_then(|s| s.to_str()))
        else {
            continue;
        };
        let group = dirs
            .entry(dir.to_path_buf())
            .or_insert_with(|| siblings(dir));
        let still = STILL_EXTENSIONS.iter().find_map(|e| {
            group
                .get(&stem.to_ascii_lowercase())?
                .iter()
                .find(|p| extension_of(p).as_deref() == Some(e))
        });
        if let Some(still) = still {
            pairs.insert(video, still.clone());
        }
    }
    let have: BTreeSet<PathBuf> = files.iter().cloned().collect();
    files.extend(
        pairs
            .iter()
            .flat_map(|(v, s)| [v, s])
            .filter(|p| !have.contains(*p))
            .cloned(),
    );
    files.sort();
    files.dedup();
    pairs
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
        let mut files = collect_files(paths, &mut report.failed);
        let pairs = find_pairs(&mut files);
        let mut clips: HashMap<PathBuf, ImportResult> = HashMap::new();
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
                Ok(Err(AnalyzeError::Unsupported(why))) => {
                    *report.skipped_why.entry(why).or_default() += 1;
                    report.skipped.push(path);
                }
                Ok(Err(e)) => report.failed.push((path, e.to_string())),
                // The moving half of a Live Photo waits for its picture: it is no item of its own.
                Ok(Ok(result)) if pairs.contains_key(&path) && is_companion_clip(&result) => {
                    clips.insert(path, result);
                }
                Ok(Ok(result)) => {
                    self.ingest_into(&path, &result, &opts, &mut held, &mut report)
                        .await;
                }
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
        // Live Photos: each clip goes with its picture, once that has an id.
        for (video, result) in clips {
            let photo = pairs.get(&video).and_then(|still| {
                report
                    .by_path
                    .iter()
                    .find(|(p, _)| p == still)
                    .map(|(_, id)| *id)
            });
            let Some(photo) = photo else {
                // The picture did not come in (skipped, damaged): the clip is a video like any other.
                self.ingest_into(&video, &result, &opts, &mut held, &mut report)
                    .await;
                continue;
            };
            match self
                .attach_clip(&video, &result, photo, &opts, &mut held)
                .await
            {
                Ok(true) => report.with_video.push(photo),
                Ok(false) => {}
                Err(e) => {
                    report.failed.push((video, format!("{e:#}")));
                    continue;
                }
            }
            report.by_path.push((video, photo));
        }
        self.publish_holdings(held).await?;
        if !report.imported.is_empty() {
            // Best effort: peers also sync on their own schedule.
            let _ = self.announce_new_items(report.imported.len() as u32).await;
        }
        Ok(report)
    }

    /// Give a video its poster frame (a JPEG): it becomes the video's thumb in the library, so every
    /// device shows it and fetches it like a photo's thumbnail. A video that already has one keeps
    /// it; returns whether this one was taken.
    pub async fn set_video_poster(&self, id: &PhotoId, jpeg: Vec<u8>) -> Result<bool> {
        anyhow::ensure!(
            jpeg.starts_with(&[0xFF, 0xD8, 0xFF]),
            "a poster frame must be a JPEG"
        );
        anyhow::ensure!(
            jpeg.len() <= MAX_POSTER_BYTES,
            "the poster frame is too large"
        );
        let _ingesting = self.inner.ingest.read().await;
        let store = self.store();
        let Some(rec) = store.photo(id).await? else {
            anyhow::bail!("unknown photo {id}");
        };
        if !rec.original.mime.starts_with("video/") || rec.thumb.hash != rec.original.hash {
            return Ok(false);
        }
        let size = jpeg.len() as u64;
        let hash = self
            .node()
            .blobs()
            .add_bytes(jpeg, Protection::Library)
            .await?;
        store
            .upsert_photo(PhotoRecord {
                thumb: BlobRef {
                    hash,
                    size,
                    mime: "image/jpeg".into(),
                },
                ..rec
            })
            .await?;
        self.publish_holdings(vec![(hash, BlobKind::Thumb)]).await?;
        Ok(true)
    }

    /// Ingest one analysed file and record the outcome in `report`; a motion photo's video goes in
    /// with it. Returns the photo and whether it is new, unless it failed.
    pub(crate) async fn ingest_into(
        &self,
        path: &Path,
        result: &ImportResult,
        opts: &ImportOptions,
        held: &mut Vec<(Blake3Hash, BlobKind)>,
        report: &mut ImportReport,
    ) -> Option<(PhotoId, bool)> {
        self.ingest_as(path, path, result, opts, held, report).await
    }

    /// As [`ingest_into`](Self::ingest_into), reading the file at `path` but reporting it as `name`
    /// (an archive entry is read from a temporary file).
    pub(crate) async fn ingest_as(
        &self,
        path: &Path,
        name: &Path,
        result: &ImportResult,
        opts: &ImportOptions,
        held: &mut Vec<(Blake3Hash, BlobKind)>,
        report: &mut ImportReport,
    ) -> Option<(PhotoId, bool)> {
        let (id, new) = match self.ingest(path, result, opts, held).await {
            Ok(Outcome::Imported(id)) => {
                report.imported.push(id);
                (id, true)
            }
            Ok(Outcome::Duplicate(id)) => {
                report.duplicates.push(id);
                (id, false)
            }
            Err(e) => {
                report.failed.push((name.to_path_buf(), format!("{e:#}")));
                return None;
            }
        };
        report.by_path.push((name.to_path_buf(), id));
        if let Some(video) = &result.embedded_video {
            // The picture is in; a video that cannot be kept does not undo that.
            match self.attach_embedded(path, video, id, held).await {
                Ok(true) => report.with_video.push(id),
                Ok(false) => {}
                Err(e) => warn!("the video in {} was not kept: {e:#}", name.display()),
            }
        }
        Some((id, new))
    }

    /// Give `photo` the video `r` (a clip imported from its own file). `Ok(false)`: it had it already.
    pub(crate) async fn attach_clip(
        &self,
        path: &Path,
        r: &ImportResult,
        photo: PhotoId,
        opts: &ImportOptions,
        held: &mut Vec<(Blake3Hash, BlobKind)>,
    ) -> Result<bool> {
        let _ingesting = self.inner.ingest.read().await;
        let store = self.store();
        if store
            .motion(&photo)
            .await?
            .is_some_and(|m| m.video.hash == r.hash)
        {
            return Ok(false);
        }
        let blobs = self.node().blobs();
        if !blobs.has(&r.hash).await? {
            let stored = blobs.add_path(path, opts.mode, Protection::Library).await?;
            anyhow::ensure!(
                stored == r.hash,
                "{} changed while it was being imported",
                path.display()
            );
            if opts.mode == ImportMode::Reference {
                blobs.protect(&r.hash, Protection::InPlace).await?;
            }
            held.push((r.hash, BlobKind::Motion));
        }
        self.put_motion(
            photo,
            BlobRef {
                hash: r.hash,
                size: r.size,
                mime: r.mime.clone(),
            },
            r.duration_ms,
            MotionSource::Paired,
        )
        .await?;
        Ok(true)
    }

    /// Give `photo` the video found inside its file (a Samsung or Google motion photo). The record
    /// names the clip by hash and size; its bytes stay in the original, which this device holds
    /// (copied or referenced in place), and are read from there when played (`clip.rs`). Only if
    /// the original is not held here (it should be: the photo was just ingested) is the clip
    /// copied out into a blob of its own, so it can still be played.
    async fn attach_embedded(
        &self,
        path: &Path,
        video: &EmbeddedVideo,
        photo: PhotoId,
        held: &mut Vec<(Blake3Hash, BlobKind)>,
    ) -> Result<bool> {
        let _ingesting = self.inner.ingest.read().await;
        let store = self.store();
        // The photo's id is the hash of the whole file, so its video is always the same one.
        if store.motion(&photo).await?.is_some() {
            return Ok(false);
        }
        let (path, offset, len) = (path.to_path_buf(), video.offset, video.len);
        let bytes = tokio::task::spawn_blocking(move || -> std::io::Result<Vec<u8>> {
            use std::io::{Read, Seek, SeekFrom};
            let mut file = std::fs::File::open(path)?;
            file.seek(SeekFrom::Start(offset))?;
            let mut bytes = vec![0u8; usize::try_from(len).map_err(std::io::Error::other)?];
            file.read_exact(&mut bytes)?;
            Ok(bytes)
        })
        .await??;
        let size = bytes.len() as u64;
        let blobs = self.node().blobs();
        let hash = Blake3Hash::of_bytes(&bytes);
        if !blobs.kept(&photo).await? && !blobs.has(&hash).await? {
            blobs.add_bytes(bytes, Protection::Library).await?;
            held.push((hash, BlobKind::Motion));
        }
        self.put_motion(
            photo,
            BlobRef {
                hash,
                size,
                mime: "video/mp4".into(),
            },
            Some(video.duration_ms),
            MotionSource::Embedded,
        )
        .await?;
        Ok(true)
    }

    async fn put_motion(
        &self,
        photo: PhotoId,
        video: BlobRef,
        duration_ms: Option<u32>,
        source: MotionSource,
    ) -> Result<()> {
        self.store()
            .put_motion(MotionRecord {
                photo,
                video,
                duration_ms: duration_ms.unwrap_or(0),
                source,
                // Set by the store.
                made_by: self.device(),
                hlc: Hlc::GENESIS,
            })
            .await?;
        Ok(())
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
                // A device that is the whole library and holds this original keeps no preview of
                // it; the record has its hash and `restore_previews` makes it if another device
                // joins.
                let ph = if self.drops_previews()
                    && shoebox_core::policy::shows_directly(&r.mime)
                    && self.is_alone().await?
                {
                    Blake3Hash::of_bytes(&p.bytes)
                } else {
                    let ph = blobs
                        .add_bytes(p.bytes.clone(), Protection::Library)
                        .await?;
                    held.push((ph, BlobKind::Preview));
                    ph
                };
                let th = blobs
                    .add_bytes(t.bytes.clone(), Protection::Library)
                    .await?;
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
