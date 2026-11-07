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
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::warn;

fn hash(file: &ModelFile) -> Blake3Hash {
    Blake3Hash(file.blake3)
}

/// Where a device may look for a model besides what its library offers (S2-02).
#[derive(Clone, Default)]
pub struct ModelSources {
    /// Folders to look in first: the app's resources and its own `models/` folder.
    pub dirs: Vec<PathBuf>,
    /// Where to download a file nobody has to; `None` never downloads (a metered network, or a
    /// device that is not to).
    pub download_to: Option<PathBuf>,
    /// Told how far a download has got: the file, bytes in hand, its size.
    #[allow(clippy::type_complexity)]
    pub progress: Option<Arc<dyn Fn(&ModelFile, u64, u64) + Send + Sync>>,
}

/// A copy of `file` in one of `dirs` that really is the model (size and hash checked).
async fn local_copy(file: &ModelFile, dirs: &[PathBuf]) -> Option<(PathBuf, Bytes)> {
    for dir in dirs {
        let path = dir.join(file.name);
        match tokio::fs::metadata(&path).await {
            Ok(m) if m.len() == file.size => {}
            _ => continue,
        }
        let Ok(bytes) = tokio::fs::read(&path).await else {
            continue;
        };
        if *blake3::hash(&bytes).as_bytes() == file.blake3 {
            return Some((path, bytes.into()));
        }
        warn!("{} is not the model it should be: ignored", path.display());
    }
    None
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

    /// Stop offering `file` and let its copy in the blob store go: the feature it serves is off on
    /// this device. The caller removes the file in its folder.
    pub async fn forget_model(&self, file: &ModelFile) -> Result<()> {
        let h = hash(file);
        if self.node().blobs().kept(&h).await? {
            self.node().blobs().unprotect(&h, Protection::Index).await?;
        }
        if self
            .store()
            .holders(&h)
            .await?
            .iter()
            .any(|x| x.device == self.device())
        {
            self.retract_holdings(vec![h]).await?;
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
        self.model_bytes_from(file, &ModelSources::default()).await
    }

    /// The model's bytes, from the first place that has them:
    ///
    /// 1. this device's blob store;
    /// 2. a copy in one of `sources.dirs` (then offered to the other devices);
    /// 3. a device of the library that offers it;
    /// 4. a download from the file's published address, when `sources.download_to` is set (then
    ///    offered to the other devices too).
    ///
    /// `None` when none of those has it.
    pub async fn model_bytes_from(
        &self,
        file: &ModelFile,
        sources: &ModelSources,
    ) -> Result<Option<Bytes>> {
        let h = hash(file);
        if self.node().blobs().has(&h).await? {
            return self.stored_model(file).await.map(Some);
        }
        if let Some((path, bytes)) = local_copy(file, &sources.dirs).await {
            if let Err(e) = self.share_model(file, &path).await {
                warn!("could not offer {}: {e:#}", file.name);
            }
            return Ok(Some(bytes));
        }
        if self.model_offered(file).await? {
            match self.fetch_offered_model(file).await {
                Ok(()) => return self.stored_model(file).await.map(Some),
                // The device that offers it may be away: a download can still do.
                Err(e) if sources.download_to.is_some() => {
                    warn!("could not get {} from another device: {e:#}", file.name);
                }
                Err(e) => return Err(e),
            }
        }
        let Some(dir) = &sources.download_to else {
            return Ok(None);
        };
        let progress = sources.progress.clone();
        let path = crate::model_download::download_model(file, dir, &move |done, total| {
            if let Some(p) = &progress {
                p(file, done, total);
            }
        })
        .await?;
        if let Err(e) = self.share_model(file, &path).await {
            warn!("could not offer {}: {e:#}", file.name);
        }
        let bytes = tokio::fs::read(&path).await?;
        Ok(Some(bytes.into()))
    }

    async fn fetch_offered_model(&self, file: &ModelFile) -> Result<()> {
        let h = hash(file);
        let blob = BlobRef {
            hash: h,
            size: file.size,
            mime: "application/onnx".into(),
        };
        self.fetch_blob_as_index(&blob).await?;
        self.publish_holdings(vec![(h, BlobKind::Index)]).await
    }

    async fn stored_model(&self, file: &ModelFile) -> Result<Bytes> {
        let bytes = self.node().blobs().get(&hash(file)).await?;
        if bytes.len() as u64 != file.size {
            bail!("{} has the wrong size", file.name);
        }
        Ok(bytes)
    }
}
