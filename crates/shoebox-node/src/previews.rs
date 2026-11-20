//! Previews that are not kept next to an original (`NodeConfig::drop_previews`).
//!
//! A device that holds a JPEG or PNG original shows and indexes the original, so a stored preview
//! is only needed by another device that has no original. (A 1280 px AVIF preview is about 0.05
//! MB a photo; the 2048 px JPEG previews of photos imported before were about 0.45 MB, four fifths
//! of what the library took on a phone that references its gallery.) The record keeps the
//! preview's hash. When another device needs it, the preview is made again from the original in
//! the format the record names: the same decoder, resize and encoder give the same bytes, so the
//! hash still matches.
//!
//! Previews made before the switch to AVIF are converted (A-150): for each photo one device that
//! holds the original (a keeper first) makes the AVIF preview and writes it into the record. The
//! merge prefers an AVIF preview, so every device takes it; each fetches the new preview as for
//! any changed record, and its maintenance releases the old JPEG, which nothing references now.

use crate::blobs::Protection;
use crate::LibraryNode;
use anyhow::Result;
use shoebox_core::ids::Blake3Hash;
use shoebox_core::ids::EndpointId;
use shoebox_core::model::{BlobKind, BlobRef, PhotoId, PhotoRecord, Role};
use shoebox_core::policy::shows_directly;
use shoebox_core::store::{LibraryStore, Page, TimeRange};
use std::collections::{HashMap, HashSet};
use std::sync::Weak;
use std::time::Duration;
use tracing::{debug, info, warn};

/// Previews made again in one maintenance pass (each decodes a whole original).
const RESTORE_PER_PASS: usize = 40;

/// Previews converted to AVIF in one batch. Each decodes a whole original and encodes on one
/// thread: about a second a photo, so a batch is a minute or so.
const CONVERT_PER_BATCH: usize = 50;
/// Between two batches, so a conversion of the whole library leaves room for everything else.
const CONVERT_PAUSE: Duration = Duration::from_secs(2);
/// After a pass that found nothing to convert (or before the first).
const CONVERT_IDLE: Duration = Duration::from_secs(3600);
const AVIF: &str = "image/avif";

/// Whether `rec` has a preview of the old kind that can be made again as AVIF: a JPEG preview of
/// its own (not the original shown as is) of a still picture.
fn has_old_preview(rec: &PhotoRecord) -> bool {
    rec.preview.mime == "image/jpeg"
        && rec.preview.hash != rec.original.hash
        && rec.original.mime.starts_with("image/")
}

/// The device that converts the preview of a photo whose original `holders` hold: a keeper
/// first (it holds the whole library and is plugged in), then the lowest device id. Every device
/// computes the same answer, so a photo is converted once.
fn converter(holders: &[EndpointId], roles: &HashMap<EndpointId, Role>) -> Option<EndpointId> {
    holders
        .iter()
        .filter_map(|d| roles.get(d).map(|r| (*r != Role::Keeper, *d)))
        .min()
        .map(|(_, d)| d)
}

impl<L: LibraryStore> LibraryNode<L> {
    /// Whether this device keeps no preview next to an original it holds.
    pub(crate) fn drops_previews(&self) -> bool {
        let c = self.node().config();
        c.drop_previews && c.role != Role::Keeper
    }

    /// True when no other device that is still part of the library exists.
    pub(crate) async fn is_alone(&self) -> Result<bool> {
        let me = self.device();
        Ok(!self
            .store()
            .device_list()
            .await?
            .devices
            .iter()
            .any(|d| d.device != me && !d.revoked))
    }

    /// Make the preview of `rec` from its original if this device holds the original and no
    /// preview. True if the preview is held afterwards. A render that does not give the hash in
    /// the record (another build of the decoder) is not kept: the record would not find it.
    pub async fn ensure_preview(&self, rec: &PhotoRecord) -> Result<bool> {
        let blobs = self.node().blobs();
        let hash = rec.preview.hash;
        if hash == rec.original.hash || blobs.has(&hash).await? {
            return Ok(true);
        }
        if !shows_directly(&rec.original.mime) || !blobs.kept(&rec.original.hash).await? {
            return Ok(false);
        }
        let original = blobs.get(&rec.original.hash).await?;
        let mime = rec.preview.mime.clone();
        let made = tokio::task::spawn_blocking(move || {
            shoebox_media::remake_preview(&original, &mime)
        })
        .await?;
        let bytes = match made {
            Ok(b) => b,
            Err(e) => {
                warn!("could not make the preview of {}: {e}", rec.id);
                return Ok(false);
            }
        };
        if Blake3Hash::of_bytes(&bytes) != hash {
            warn!("the preview of {} came out different; not kept", rec.id);
            return Ok(false);
        }
        blobs.add_bytes(bytes, Protection::Library).await?;
        self.publish_holdings(vec![(hash, BlobKind::Preview)])
            .await?;
        Ok(true)
    }

    /// Once another device is part of the library, make again the previews that no device holds
    /// any more and this device can make (a library that began on one phone). A few each pass.
    pub async fn restore_previews(&self) -> Result<usize> {
        if self.is_alone().await? {
            return Ok(0);
        }
        let me = self.device();
        let photos = self
            .store()
            .photos_by_time(TimeRange::ALL, Page::all())
            .await?;
        let mut made = 0;
        for rec in &photos {
            if made >= RESTORE_PER_PASS {
                break;
            }
            let blobs = self.node().blobs();
            if rec.preview.hash == rec.original.hash
                || !shows_directly(&rec.original.mime)
                || blobs.has(&rec.preview.hash).await?
                || !blobs.kept(&rec.original.hash).await?
            {
                continue;
            }
            let elsewhere = self
                .store()
                .holders(&rec.preview.hash)
                .await?
                .iter()
                .any(|h| h.kind == BlobKind::Preview && h.device != me);
            if elsewhere {
                continue;
            }
            if self.ensure_preview(rec).await? {
                made += 1;
            }
        }
        if made > 0 {
            debug!("made {made} preview(s) again for the other devices");
        }
        Ok(made)
    }

    /// Convert up to `limit` previews of the old kind (JPEG) to AVIF, newest photos first, for
    /// the photos this device converts (see [`converter`]). Returns how many were converted.
    pub async fn convert_previews(&self, limit: usize) -> Result<usize> {
        self.convert_previews_skipping(limit, &mut HashSet::new())
            .await
    }

    async fn convert_previews_skipping(
        &self,
        limit: usize,
        failed: &mut HashSet<PhotoId>,
    ) -> Result<usize> {
        let me = self.device();
        let roles: HashMap<EndpointId, Role> = self
            .store()
            .device_list()
            .await?
            .devices
            .iter()
            .filter(|d| !d.revoked)
            .map(|d| (d.device, d.role))
            .collect();
        let mut photos = self
            .store()
            .photos_by_time(TimeRange::ALL, Page::all())
            .await?;
        photos.extend(self.store().deleted_photos().await?);
        let mut made = 0;
        for rec in &photos {
            if made >= limit {
                break;
            }
            if !has_old_preview(rec) || failed.contains(&rec.id) {
                continue;
            }
            let holders: Vec<EndpointId> = self
                .store()
                .holders(&rec.original.hash)
                .await?
                .iter()
                .filter(|h| h.kind == BlobKind::Original)
                .map(|h| h.device)
                .collect();
            if converter(&holders, &roles) != Some(me)
                || !self.node().blobs().kept(&rec.original.hash).await?
            {
                continue;
            }
            if self.convert_preview(rec).await? {
                made += 1;
            } else {
                failed.insert(rec.id);
            }
        }
        Ok(made)
    }

    /// Make the AVIF preview of `rec` from its original and write it into the record. False if
    /// the original cannot be decoded here.
    async fn convert_preview(&self, rec: &PhotoRecord) -> Result<bool> {
        let blobs = self.node().blobs();
        let original = blobs.get(&rec.original.hash).await?;
        let made = tokio::task::spawn_blocking(move || {
            shoebox_media::remake_preview(&original, AVIF)
        })
        .await?;
        let bytes = match made {
            Ok(b) => b,
            Err(e) => {
                debug!("could not convert the preview of {}: {e}", rec.id);
                return Ok(false);
            }
        };
        let size = bytes.len() as u64;
        // The new preview is referenced by no record until the upsert: keep reconcile out.
        let _ingesting = self.inner.ingest.read().await;
        let hash = blobs.add_bytes(bytes, Protection::Library).await?;
        self.publish_holdings(vec![(hash, BlobKind::Preview)])
            .await?;
        self.store()
            .upsert_photo(PhotoRecord {
                preview: BlobRef {
                    hash,
                    size,
                    mime: AVIF.into(),
                },
                ..rec.clone()
            })
            .await?;
        Ok(true)
    }

    /// Convert the old previews this device converts, a batch at a time, then look again hourly
    /// (photos synced from a device that had not upgraded). Old previews are released after each
    /// batch.
    pub(crate) fn start_preview_conversion(&self) {
        let weak = std::sync::Arc::downgrade(&self.inner);
        tokio::spawn(conversion_loop(weak));
    }
}

async fn conversion_loop<L: LibraryStore>(weak: Weak<crate::library_node::Inner<L>>) {
    let mut failed = HashSet::new();
    let mut total = 0usize;
    let mut wait = Duration::from_secs(90);
    loop {
        tokio::time::sleep(wait).await;
        let Some(inner) = weak.upgrade() else { break };
        let ln = LibraryNode { inner };
        let made = match ln
            .convert_previews_skipping(CONVERT_PER_BATCH, &mut failed)
            .await
        {
            Ok(n) => n,
            Err(e) => {
                warn!("converting previews failed: {e:#}");
                0
            }
        };
        if made > 0 {
            total += made;
            debug!("converted {total} preview(s) to AVIF so far");
            if let Err(e) = ln.reconcile_blobs().await {
                warn!("blob reconcile failed: {e:#}");
            }
            wait = CONVERT_PAUSE;
        } else {
            if total > 0 {
                info!("converted {total} preview(s) to AVIF");
                total = 0;
            }
            wait = CONVERT_IDLE;
        }
    }
}
