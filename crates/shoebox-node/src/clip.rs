//! Motion-photo clips that are not stored (S2-06).
//!
//! A Samsung or Google motion photo is one JPEG with its video appended. The original is already
//! kept (copied, or referenced in the gallery), so a second copy of the clip as a blob of its own
//! is pure duplication (about 4.7 MB a photo). The library's [`MotionRecord`] still names the clip
//! by hash and size; a device that holds the original finds the clip in it (`shoebox_media::motion`)
//! and reads the byte range on demand. The clip is never a blob here, so it is never advertised as
//! one: another device that wants it fetches the original (which has it) and cuts the clip itself;
//! only a device that still stores the clip from before (a copy made by an older build) is asked
//! for it as a blob, until it releases it ([`LibraryNode::release_embedded_clips`]).
//!
//! A paired Live Photo's video (an iPhone's separate .MOV) is a file of its own and stays a blob
//! (referenced in place when the gallery file is).

use crate::blobs::{BlobHandle, Protection};
use crate::library_node::LibraryNode;
use anyhow::Result;
use bytes::Bytes;
use shoebox_core::ids::Blake3Hash;
use shoebox_core::model::{MotionRecord, MotionSource, PhotoId, PhotoRecord};
use shoebox_core::store::LibraryStore;
use std::ops::Range;
use tracing::{debug, warn};

impl<L: LibraryStore> LibraryNode<L> {
    /// Where the clip of `photo` can be read on this device: a blob of its own, or a part of the
    /// photo's original. `None` while neither is here.
    pub async fn clip_source(&self, photo: &PhotoId) -> Result<Option<BlobHandle>> {
        let Some(m) = self.store().motion(photo).await? else {
            return Ok(None);
        };
        self.clip_handle(&m).await
    }

    async fn clip_handle(&self, m: &MotionRecord) -> Result<Option<BlobHandle>> {
        let blobs = self.node().blobs();
        if blobs.has(&m.video.hash).await? {
            return Ok(Some(BlobHandle::whole(m.video.hash)));
        }
        if m.source != MotionSource::Embedded {
            return Ok(None);
        }
        let Some(rec) = self.store().photo(&m.photo).await? else {
            return Ok(None);
        };
        if !blobs.kept(&rec.original.hash).await? {
            return Ok(None);
        }
        let known = self
            .inner
            .clip_extents
            .lock()
            .unwrap()
            .get(&m.photo)
            .copied();
        let (offset, len) = match known {
            Some(e) => e,
            None => {
                let found = match self.find_clip(m, &rec.original.hash).await? {
                    Some(e) => Some(e),
                    // The record may name the wrong clip (an earlier build): correct it now, so
                    // the photo plays when it is opened, not when the next pass gets to it.
                    None => self.repair_one(m, &rec).await?,
                };
                let Some(e) = found else {
                    return Ok(None);
                };
                self.inner.clip_extents.lock().unwrap().insert(m.photo, e);
                e
            }
        };
        let handle = BlobHandle {
            hash: rec.original.hash,
            offset,
            len: Some(len),
        };
        // The original may be a gallery file that went away or shrank since.
        Ok(blobs.handle_size(&handle).await?.map(|_| handle))
    }

    /// Find the clip inside the original and make sure it is the clip the record names (same size).
    async fn find_clip(
        &self,
        m: &MotionRecord,
        original: &Blake3Hash,
    ) -> Result<Option<(u64, u64)>> {
        let bytes = match self.node().blobs().get(original).await {
            Ok(b) => b,
            Err(e) => {
                debug!("reading the original of {} for its clip: {e:#}", m.photo);
                return Ok(None);
            }
        };
        let found =
            tokio::task::spawn_blocking(move || shoebox_media::find_embedded_video(&bytes)).await?;
        Ok(found
            .filter(|v| v.len == m.video.size)
            .map(|v| (v.offset, v.len)))
    }

    /// Whether the clip of `photo` can be played on this device.
    pub async fn clip_available(&self, photo: &PhotoId) -> Result<bool> {
        Ok(self.clip_source(photo).await?.is_some())
    }

    /// Bytes `range` of the clip of `photo`; `None` while it is not on this device.
    pub async fn read_clip(&self, photo: &PhotoId, range: Range<u64>) -> Result<Option<Bytes>> {
        match self.clip_source(photo).await? {
            Some(h) => Ok(Some(self.node().blobs().read_handle(&h, range).await?)),
            None => Ok(None),
        }
    }

    /// The whole clip, checked against the hash in its record.
    pub async fn clip_bytes(&self, photo: &PhotoId) -> Result<Option<Bytes>> {
        let Some(m) = self.store().motion(photo).await? else {
            return Ok(None);
        };
        let Some(h) = self.clip_handle(&m).await? else {
            return Ok(None);
        };
        let bytes = self.node().blobs().read_handle(&h, 0..u64::MAX).await?;
        anyhow::ensure!(
            Blake3Hash::of_bytes(&bytes) == m.video.hash,
            "the video of {photo} does not match its record"
        );
        Ok(Some(bytes))
    }

    /// Release the stored copies of embedded clips whose original this device holds (an older build
    /// copied each clip out): the clip is read from the original from now on. A clip goes only
    /// once it was read back out of the original and matched its hash. Returns the released clips;
    /// the blob store collects them shortly after.
    pub async fn release_embedded_clips(&self) -> Result<Vec<Blake3Hash>> {
        let mut candidates = vec![];
        for m in self.store().motions().await? {
            let blobs = self.node().blobs();
            if m.source != MotionSource::Embedded || !blobs.kept(&m.video.hash).await? {
                continue;
            }
            // A clip that is also a photo of its own (the same video imported by itself) stays.
            if self.store().photo(&m.video.hash).await?.is_some() {
                continue;
            }
            let Some(rec) = self.store().photo(&m.photo).await? else {
                continue;
            };
            if !blobs.kept(&rec.original.hash).await? {
                continue;
            }
            let protections = blobs.protections(&m.video.hash).await?;
            if protections
                .iter()
                .any(|p| !matches!(p, Protection::Library))
            {
                continue;
            }
            candidates.push((m, rec.original.hash));
        }
        let mut released = vec![];
        for (m, original) in candidates {
            let Some((offset, len)) = self.find_clip(&m, &original).await? else {
                continue;
            };
            let handle = BlobHandle {
                hash: original,
                offset,
                len: Some(len),
            };
            let Ok(bytes) = self.node().blobs().read_handle(&handle, 0..u64::MAX).await else {
                continue;
            };
            if Blake3Hash::of_bytes(&bytes) != m.video.hash {
                warn!(
                    "the video in the original of {} differs from its record",
                    m.photo
                );
                continue;
            }
            self.inner
                .clip_extents
                .lock()
                .unwrap()
                .insert(m.photo, (offset, len));
            released.push(m.video.hash);
        }
        if released.is_empty() {
            return Ok(released);
        }
        {
            let _exclusive = self.inner.ingest.write().await;
            for hash in &released {
                self.node().blobs().remove(hash).await?;
            }
        }
        self.retract_holdings(released.clone()).await?;
        debug!(
            "released {} stored clip(s) kept in their originals",
            released.len()
        );
        Ok(released)
    }
    /// Look again, in each embedded-clip photo's original, for the video the camera shows. Builds
    /// before the Samsung fix took the first MP4 after the picture, which in newer Samsung files
    /// is a 0.6 s silent clip ahead of the real one; their records name that clip. Where the
    /// original now gives another clip, the record is replaced by one that names it. A photo
    /// that is opened is corrected at once (`clip_handle`); this goes through the rest, for up to
    /// a minute each pass (each reads a whole original). Once every record has been looked at, a
    /// marker file in the data folder says so and later runs skip it.
    pub async fn repair_motion_clips(&self) -> Result<usize> {
        const BUDGET: std::time::Duration = std::time::Duration::from_secs(60);
        let marker = self.node().config().data_dir.join("motion-clips-v2");
        if marker.exists() {
            return Ok(0);
        }
        let started = std::time::Instant::now();
        let mut repaired = 0;
        let mut all_done = true;
        for m in self.store().motions().await? {
            if m.source != MotionSource::Embedded
                || self.inner.clip_checked.lock().unwrap().contains(&m.photo)
            {
                continue;
            }
            if started.elapsed() >= BUDGET {
                all_done = false;
                break;
            }
            let Some(rec) = self.store().photo(&m.photo).await? else {
                continue;
            };
            if !self.node().blobs().kept(&rec.original.hash).await? {
                continue; // another device's photo: its own device repairs it
            }
            let before = self.store().motion(&m.photo).await?.map(|x| x.video.hash);
            if let Some(extent) = self.repair_one(&m, &rec).await? {
                self.inner
                    .clip_extents
                    .lock()
                    .unwrap()
                    .insert(m.photo, extent);
            }
            repaired +=
                usize::from(self.store().motion(&m.photo).await?.map(|x| x.video.hash) != before);
        }
        if all_done {
            let _ = std::fs::write(&marker, b"2");
        }
        if repaired > 0 {
            debug!(
                "replaced the video of {repaired} motion photo(s) with the one the camera shows"
            );
        }
        Ok(repaired)
    }

    /// Look for the camera's video in the original of `rec`; if it is not the clip `m` names,
    /// replace the record. The clip's place in the original once it is right, `None` when the
    /// original cannot be read or holds none. Once per photo per run.
    async fn repair_one(&self, m: &MotionRecord, rec: &PhotoRecord) -> Result<Option<(u64, u64)>> {
        if !self.inner.clip_checked.lock().unwrap().insert(m.photo) {
            return Ok(None);
        }
        let blobs = self.node().blobs();
        let Ok(bytes) = blobs.get(&rec.original.hash).await else {
            return Ok(None);
        };
        let found =
            tokio::task::spawn_blocking(move || shoebox_media::find_embedded_video(&bytes)).await?;
        let Some(v) = found else { return Ok(None) };
        if v.len == m.video.size {
            return Ok(Some((v.offset, v.len)));
        }
        let handle = BlobHandle {
            hash: rec.original.hash,
            offset: v.offset,
            len: Some(v.len),
        };
        let Ok(clip) = blobs.read_handle(&handle, 0..u64::MAX).await else {
            return Ok(None);
        };
        let video = shoebox_core::model::BlobRef {
            hash: Blake3Hash::of_bytes(&clip),
            size: v.len,
            mime: "video/mp4".into(),
        };
        self.store()
            .put_motion(MotionRecord {
                photo: m.photo,
                video,
                duration_ms: v.duration_ms,
                source: MotionSource::Embedded,
                made_by: self.device(),
                hlc: shoebox_core::hlc::Hlc::GENESIS,
            })
            .await?;
        Ok(Some((v.offset, v.len)))
    }
}
