//! Poster frames made by a keeper (A-154).
//!
//! A video's thumb is its original until some device gives it a poster frame
//! ([`LibraryNode::set_video_poster`]). The apps make them with the platform's thumbnailer, but
//! only for tiles on screen on a device holding the original; a headless keeper made none, so
//! the videos it imported stayed without one (765 of 1,645 in the owner's library). A keeper
//! holds every original and is always on, so it now makes them itself with `ffmpeg` when one is
//! installed (`SHOEBOX_FFMPEG`, else `ffmpeg` on the `PATH`): newest videos first, a batch at a
//! time, then hourly for videos that arrive later. Without `ffmpeg` nothing changes.
//!
//! `ffmpeg` reads a file and seeks in it (a phone's MP4 keeps its index at the end), so each
//! video is copied out of the store to a temporary file next to it, read, and deleted.
//!
//! Before this, a tile or album cover that asked for such a video's thumb fetched the whole video
//! as a "thumb", which nothing released (6 videos, 260 MB on the owner's phone).
//! [`LibraryNode::release_placeholder_videos`] lets those go on devices other than keepers, or
//! counts them as originals when the device's keep rules want the original anyway.

use crate::blobs::Protection;
use crate::LibraryNode;
use anyhow::{bail, Context, Result};
use shoebox_core::ids::Blake3Hash;
use shoebox_core::model::{BlobKind, PhotoId, PhotoRecord, Role};
use shoebox_core::policy::fetch_plan;
use shoebox_core::store::{LibraryStore, Page, TimeRange};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Weak;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tracing::{debug, info, warn};

/// Posters made in one batch (each copies a video out of the store and decodes a frame).
const PER_BATCH: usize = 20;
/// Between two batches, so a backlog of hundreds leaves room for everything else.
const PAUSE: Duration = Duration::from_secs(5);
/// After a pass that found nothing to do (or before the first).
const IDLE: Duration = Duration::from_secs(3600);
/// Longest one `ffmpeg` run may take.
const FFMPEG_TIMEOUT: Duration = Duration::from_secs(60);
/// The poster's longest side, as the apps make them (`ui/src/lib/poster.ts`).
const MAX_SIDE: u32 = 480;

/// A video that still uses its original as its thumbnail.
pub(crate) fn lacks_poster(rec: &PhotoRecord) -> bool {
    rec.original.mime.starts_with("video/") && rec.thumb.hash == rec.original.hash
}

/// The `ffmpeg` to use, if one runs here.
pub async fn find_ffmpeg() -> Option<PathBuf> {
    let candidate = std::env::var_os("SHOEBOX_FFMPEG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("ffmpeg"));
    let ok = tokio::process::Command::new(&candidate)
        .arg("-version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .status()
        .await
        .is_ok_and(|s| s.success());
    ok.then_some(candidate)
}

/// One frame of the video at `path` as a JPEG of at most [`MAX_SIDE`] px: a second in (past a
/// fade-in), or the first frame of a video shorter than that.
pub async fn video_frame(ffmpeg: &Path, path: &Path) -> Result<Vec<u8>> {
    for at in ["1", "0"] {
        let jpeg = run_ffmpeg(ffmpeg, path, at).await?;
        if !jpeg.is_empty() {
            return Ok(jpeg);
        }
    }
    bail!("ffmpeg found no frame")
}

async fn run_ffmpeg(ffmpeg: &Path, path: &Path, at: &str) -> Result<Vec<u8>> {
    let scale = format!(
        "scale='min({MAX_SIDE},iw)':'min({MAX_SIDE},ih)':force_original_aspect_ratio=decrease,format=yuvj420p"
    );
    let child = tokio::process::Command::new(ffmpeg)
        .args(["-nostdin", "-v", "error", "-ss", at, "-i"])
        .arg(path)
        .args(["-map", "0:v:0", "-frames:v", "1", "-vf", &scale])
        .args(["-c:v", "mjpeg", "-q:v", "4", "-f", "image2pipe", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("starting ffmpeg")?;
    let out = tokio::time::timeout(FFMPEG_TIMEOUT, child.wait_with_output())
        .await
        .context("ffmpeg took too long")??;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("ffmpeg failed: {}", err.trim());
    }
    Ok(out.stdout)
}

impl<L: LibraryStore> LibraryNode<L> {
    /// Make poster frames for up to `limit` videos this device holds that have none, newest
    /// first. Videos in `failed` are skipped; ones that fail are added to it. Returns how many
    /// were made.
    pub async fn make_posters(
        &self,
        ffmpeg: &Path,
        limit: usize,
        failed: &mut HashSet<PhotoId>,
    ) -> Result<usize> {
        let photos = self
            .store()
            .photos_by_time(TimeRange::ALL, Page::all())
            .await?;
        let tmp = self.node().config().data_dir.join("tmp-posters");
        let mut made = 0;
        for rec in photos.iter().filter(|r| lacks_poster(r)) {
            if made >= limit {
                break;
            }
            if failed.contains(&rec.id) || !self.node().blobs().kept(&rec.original.hash).await? {
                continue;
            }
            match self.make_poster(ffmpeg, rec, &tmp).await {
                Ok(true) => made += 1,
                Ok(false) => {}
                Err(e) => {
                    debug!("no poster frame for {}: {e:#}", rec.id);
                    failed.insert(rec.id);
                }
            }
        }
        let _ = tokio::fs::remove_dir_all(&tmp).await;
        Ok(made)
    }

    async fn make_poster(&self, ffmpeg: &Path, rec: &PhotoRecord, tmp: &Path) -> Result<bool> {
        tokio::fs::create_dir_all(tmp).await?;
        let path = tmp.join(rec.id.to_string());
        let result = async {
            self.copy_blob_to(&rec.original.hash, &path).await?;
            video_frame(ffmpeg, &path).await
        }
        .await;
        let _ = tokio::fs::remove_file(&path).await;
        self.set_video_poster(&rec.id, result?).await
    }

    /// Write blob `hash` to `path`, a few MB at a time.
    async fn copy_blob_to(&self, hash: &shoebox_core::ids::Blake3Hash, path: &Path) -> Result<()> {
        const PIECE: u64 = 4 << 20;
        let blobs = self.node().blobs();
        let size = blobs.size(hash).await?.context("not stored here")?;
        let mut file = tokio::fs::File::create(path).await?;
        let mut at = 0;
        while at < size {
            let bytes = blobs.open_range(hash, at..(at + PIECE).min(size)).await?;
            if bytes.is_empty() {
                bail!("the store returned nothing at {at} of {size}");
            }
            file.write_all(&bytes).await?;
            at += bytes.len() as u64;
        }
        file.flush().await?;
        Ok(())
    }

    /// Videos this device holds only because they stood in for their own thumb or preview
    /// (held as [`BlobKind::Thumb`] or [`BlobKind::Preview`], the hash of the original). Where
    /// the keep rules want the original here it is published as an original (a copy that
    /// counts); otherwise it is released, unless pinned or kept by a shared album. Keepers keep
    /// every original already. Returns the released videos.
    pub async fn release_placeholder_videos(&self) -> Result<Vec<Blake3Hash>> {
        let config = self.node().config();
        if config.role == Role::Keeper {
            return Ok(vec![]);
        }
        let me = self.device();
        let pinned: HashSet<String> = config.pinned_originals.iter().cloned().collect();
        let target = self.redundancy_target().await? as usize;
        let blobs = self.node().blobs();
        let protected = blobs.protected().await?;
        let photos = self
            .store()
            .photos_by_time(TimeRange::ALL, Page::all())
            .await?;
        let (mut release, mut recount) = (vec![], vec![]);
        for rec in photos
            .iter()
            .filter(|r| r.original.mime.starts_with("video/"))
        {
            let original = rec.original.hash;
            if !protected.contains(&original) {
                continue;
            }
            let mine = self
                .store()
                .holders(&original)
                .await?
                .into_iter()
                .find(|h| h.device == me)
                .map(|h| h.kind);
            if !matches!(mine, Some(BlobKind::Thumb | BlobKind::Preview)) {
                continue;
            }
            let ctx = self
                .plan_context(rec, &config, &pinned, target, &protected)
                .await?;
            let wanted = fetch_plan(config.role, rec, &ctx)
                .iter()
                .any(|i| i.blob.hash == original);
            if wanted {
                recount.push((original, BlobKind::Original));
            } else if blobs
                .protections(&original)
                .await?
                .iter()
                .all(|p| matches!(p, Protection::Library))
            {
                release.push(original);
            }
        }
        self.publish_holdings(recount).await?;
        if release.is_empty() {
            return Ok(release);
        }
        {
            let _exclusive = self.inner.ingest.write().await;
            for hash in &release {
                blobs.remove(hash).await?;
            }
        }
        self.retract_holdings(release.clone()).await?;
        info!(
            count = release.len(),
            "released videos fetched as their own thumbnail"
        );
        Ok(release)
    }

    /// On a keeper with `ffmpeg`, keep giving videos without one a poster frame.
    pub(crate) fn start_poster_making(&self) {
        if self.node().config().role != Role::Keeper {
            return;
        }
        let weak = std::sync::Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            let Some(ffmpeg) = find_ffmpeg().await else {
                info!("ffmpeg not found: videos get poster frames from the apps only");
                return;
            };
            poster_loop(weak, ffmpeg).await;
        });
    }
}

async fn poster_loop<L: LibraryStore>(weak: Weak<crate::library_node::Inner<L>>, ffmpeg: PathBuf) {
    let mut failed = HashSet::new();
    let mut total = 0usize;
    let mut wait = Duration::from_secs(60);
    loop {
        tokio::time::sleep(wait).await;
        let Some(inner) = weak.upgrade() else { break };
        let ln = LibraryNode { inner };
        let made = match ln.make_posters(&ffmpeg, PER_BATCH, &mut failed).await {
            Ok(n) => n,
            Err(e) => {
                warn!("making poster frames failed: {e:#}");
                0
            }
        };
        if made > 0 {
            total += made;
            debug!("made {total} poster frame(s) so far");
            wait = PAUSE;
        } else {
            if total > 0 {
                info!(total, failed = failed.len(), "poster frames made");
                total = 0;
            }
            wait = IDLE;
        }
    }
}
