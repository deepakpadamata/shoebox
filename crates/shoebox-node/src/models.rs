//! Search model files between devices (S1-08). Desktops and keepers ship the image model and
//! offer it as a blob; a phone fetches it from any of them the first time it indexes. The hash
//! is compiled in, so a copy from anywhere is verified like any blob (iroh-blobs checks every
//! chunk against it), and it is kept as search data (`idx/<hash>`).

use crate::blobs::{ImportMode, Protection};
use crate::library_node::LibraryNode;
use anyhow::{bail, Result};
use bytes::Bytes;
use shoebox_core::ids::Blake3Hash;
use shoebox_core::model::{BlobKind, BlobRef};
use shoebox_core::store::LibraryStore;
use shoebox_search::models::ModelFile;
use std::path::Path;

fn hash(file: &ModelFile) -> Blake3Hash {
    Blake3Hash(file.blake3)
}

impl<L: LibraryStore> LibraryNode<L> {
    /// Offer the model file at `path` (already verified by the caller) to the other devices:
    /// referenced in place, not copied, and announced as a holding.
    pub async fn share_model(&self, file: &ModelFile, path: &Path) -> Result<()> {
        let h = hash(file);
        if !self.node().blobs().kept(&h).await? {
            let added = self
                .node()
                .blobs()
                .add_path(path, ImportMode::Reference, Protection::Index)
                .await?;
            if added != h {
                self.node()
                    .blobs()
                    .unprotect(&added, Protection::Index)
                    .await?;
                bail!("{} is not {}", path.display(), file.name);
            }
        }
        if !self
            .store()
            .holders(&h)
            .await?
            .iter()
            .any(|x| x.device == self.device())
        {
            self.publish_holdings(vec![(h, BlobKind::Index)]).await?;
        }
        Ok(())
    }

    /// Whether any other device offers `file`.
    pub async fn model_offered(&self, file: &ModelFile) -> Result<bool> {
        Ok(self
            .store()
            .holders(&hash(file))
            .await?
            .iter()
            .any(|x| x.device != self.device()))
    }

    /// The model's bytes: from this device's store, or fetched from a device that offers it.
    /// `None` when this device does not hold it and nobody offers it (then it does not index).
    pub async fn model_bytes(&self, file: &ModelFile) -> Result<Option<Bytes>> {
        let h = hash(file);
        if !self.node().blobs().has(&h).await? {
            if !self.model_offered(file).await? {
                return Ok(None);
            }
            let blob = BlobRef {
                hash: h,
                size: file.size,
                mime: "application/onnx".into(),
            };
            self.fetch_blob_as_index(&blob).await?;
            self.publish_holdings(vec![(h, BlobKind::Index)]).await?;
        }
        let bytes = self.node().blobs().get(&h).await?;
        if bytes.len() as u64 != file.size {
            bail!("{} has the wrong size", file.name);
        }
        Ok(Some(bytes))
    }
}
