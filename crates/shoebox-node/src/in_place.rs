//! Originals that live outside the store (a phone's gallery): read where they are, never copied,
//! and gone from the library when the person deletes them there.
//!
//! Importing with [`ImportMode::Reference`] tags the original `place/<hash>` (see `blobs.rs`).
//! Free-up skips those blobs: the file is the gallery's, so releasing it reclaims nothing.

use crate::blobs::{ImportMode, Protection};
use crate::library_node::LibraryNode;
use anyhow::{ensure, Result};
use shoebox_core::ids::Blake3Hash;
use shoebox_core::store::LibraryStore;
use std::path::Path;

impl<L: LibraryStore> LibraryNode<L> {
    /// This device holds `hash` and holds it as a file outside the store.
    pub async fn is_in_place(&self, hash: &Blake3Hash) -> Result<bool> {
        Ok(self
            .node()
            .blobs()
            .protections(hash)
            .await?
            .contains(&Protection::InPlace))
    }

    /// First half of turning a stored copy of a gallery file back into a reference: drop the
    /// copy's protections so the store's garbage collection deletes it. iroh-blobs keeps a stored
    /// copy over a reference to the same content, so the copy has to go before
    /// [`LibraryNode::finish_move_in_place`] can reference the file. The gallery file still
    /// exists meanwhile, and this device's holdings stay as they are.
    pub async fn begin_move_in_place(&self, hash: &Blake3Hash) -> Result<()> {
        let blobs = self.node().blobs();
        blobs.unprotect(hash, Protection::Library).await?;
        blobs.unprotect(hash, Protection::Pin).await
    }

    /// Second half: once the copy is collected, reference `path` in its place. `Ok(false)` while
    /// the copy is still there (try again after the next collection).
    pub async fn finish_move_in_place(&self, hash: &Blake3Hash, path: &Path) -> Result<bool> {
        let blobs = self.node().blobs();
        if blobs.has(hash).await? {
            if self.is_in_place(hash).await? {
                return Ok(true);
            }
            let protections = blobs.protections(hash).await?;
            if protections.contains(&Protection::Library) {
                // Something protected the copy again (an import of the same content): start over.
                self.begin_move_in_place(hash).await?;
            }
            return Ok(false);
        }
        let _ingesting = self.inner.ingest.read().await;
        let stored = blobs
            .add_path(path, ImportMode::Reference, Protection::Library)
            .await?;
        ensure!(
            stored == *hash,
            "{} changed since it was imported",
            path.display()
        );
        blobs.protect(hash, Protection::InPlace).await?;
        if self.is_pinned(hash) {
            blobs.protect(hash, Protection::Pin).await?;
        }
        Ok(true)
    }

    /// The person deleted these gallery files: delete the photos from the library (they go to
    /// Recently Deleted on every device, as if deleted in the app) and stop holding them here.
    pub async fn gallery_removed(&self, hashes: &[Blake3Hash]) -> Result<()> {
        if hashes.is_empty() {
            return Ok(());
        }
        self.delete_photos(hashes).await?;
        self.release_in_place(hashes).await
    }

    /// These gallery files are no longer on this device: stop referencing them and say so, so
    /// the copy counts other devices see are true at once. The photos stay in the library.
    pub async fn release_in_place(&self, hashes: &[Blake3Hash]) -> Result<()> {
        if hashes.is_empty() {
            return Ok(());
        }
        {
            let _exclusive = self.inner.ingest.write().await;
            for hash in hashes {
                self.node().blobs().remove(hash).await?;
            }
        }
        let hexes: std::collections::HashSet<String> =
            hashes.iter().map(|h| h.to_string()).collect();
        self.node()
            .update_config(|c| c.pinned_originals.retain(|p| !hexes.contains(p)))?;
        self.retract_holdings(hashes.to_vec()).await
    }
}
