//! Previews that are not kept next to an original (`NodeConfig::drop_previews`).
//!
//! A device that holds a JPEG or PNG original shows and indexes the original, so a stored preview
//! (about 0.45 MB a photo, four fifths of what the library takes on a phone that references its
//! gallery) is only needed by another device that has no original. The record keeps the preview's
//! hash. When another device needs it, the preview is made again from the original: the same
//! decoder, resize and encoder give the same bytes, so the hash still matches.

use crate::blobs::Protection;
use crate::LibraryNode;
use anyhow::Result;
use shoebox_core::ids::Blake3Hash;
use shoebox_core::model::{BlobKind, PhotoRecord, Role};
use shoebox_core::policy::shows_directly;
use shoebox_core::store::{LibraryStore, Page, TimeRange};
use tracing::{debug, warn};

/// Previews made again in one maintenance pass (each decodes a whole original).
const RESTORE_PER_PASS: usize = 40;

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
        let made =
            tokio::task::spawn_blocking(move || shoebox_media::preview_jpeg(&original)).await?;
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
}
