//! Blob store wrapper: content-addressed bytes with a protection (tag) model.
//!
//! iroh-blobs only deletes through garbage collection, which reclaims every blob that carries no
//! tag. We therefore express "who needs this blob" as named tags, and removal means dropping the
//! tags:
//!
//! | tag name | meaning |
//! |---|---|
//! | `lib/<hash>` | referenced by the library (an original, preview or thumb of a photo) |
//! | `pin/<hash>` | the user pinned this original on this device |
//! | `album/<album>/<hash>` | a shared album references it (M5 hooks in here) |
//! | `place/<hash>` | the original is a file outside the store, read where it is (a phone's gallery photo): never freed, and released when that file goes |
//!
//! A blob is collected only once all its tags are gone, so a shared-album reference blocks the
//! GC of a blob even after the library purges the photo.

use anyhow::{anyhow, bail, Context, Result};
use bytes::Bytes;
use futures::StreamExt;
use iroh_blobs::api::blobs::AddPathOptions;
use iroh_blobs::api::proto::BlobStatus;
use iroh_blobs::store::fs::FsStore;
use iroh_blobs::{BlobFormat, Hash, HashAndFormat};
use shoebox_core::ids::Blake3Hash;
use shoebox_core::model::AlbumId;
use std::ops::Range;
use std::path::Path;
use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};

/// Why a blob is being kept.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Protection {
    Library,
    Pin,
    /// The original is a file outside the store, referenced where it is (a phone's gallery).
    InPlace,
    Album(AlbumId),
}

/// How [`BlobStore::add_path`] brings a file in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportMode {
    /// Copy the bytes into the store.
    Copy,
    /// Reference the file in place (no second copy). The file must not change afterwards.
    Reference,
}

fn hex(h: &Blake3Hash) -> String {
    h.to_string()
}

impl Protection {
    fn tag_name(&self, h: &Blake3Hash) -> String {
        match self {
            Protection::Library => format!("lib/{}", hex(h)),
            Protection::Pin => format!("pin/{}", hex(h)),
            Protection::InPlace => format!("place/{}", hex(h)),
            Protection::Album(a) => format!("album/{}/{}", ::hex::encode(a.0), hex(h)),
        }
    }
}

pub(crate) fn iroh_hash(h: &Blake3Hash) -> Hash {
    Hash::from_bytes(h.0)
}

pub(crate) fn core_hash(h: Hash) -> Blake3Hash {
    Blake3Hash(*h.as_bytes())
}

#[derive(Clone)]
pub struct BlobStore {
    store: FsStore,
    activity: crate::activity::Activity,
}

impl BlobStore {
    pub(crate) fn new(store: FsStore, activity: crate::activity::Activity) -> Self {
        Self { store, activity }
    }

    /// The underlying iroh-blobs store (for protocols and low-level access).
    pub fn raw(&self) -> &FsStore {
        &self.store
    }

    async fn tag(&self, hash: &Blake3Hash, why: &Protection) -> Result<()> {
        let value = HashAndFormat {
            hash: iroh_hash(hash),
            format: BlobFormat::Raw,
        };
        self.store
            .tags()
            .set(why.tag_name(hash), value)
            .await
            .map_err(|e| anyhow!("tagging {hash}: {e}"))
    }

    /// Store `data` and protect it. Returns its BLAKE3 hash.
    pub async fn add_bytes(&self, data: impl Into<Bytes>, why: Protection) -> Result<Blake3Hash> {
        self.activity.touch();
        let temp = self
            .store
            .blobs()
            .add_bytes(data.into())
            .temp_tag()
            .await
            .map_err(|e| anyhow!("adding bytes: {e}"))?;
        let hash = core_hash(temp.hash());
        self.tag(&hash, &why).await?;
        Ok(hash) // `temp` drops here, after the permanent tag exists
    }

    /// Import a file (hashing it as a stream) and protect it. With [`ImportMode::Reference`] the
    /// file is read in place instead of copied.
    pub async fn add_path(
        &self,
        path: &Path,
        mode: ImportMode,
        why: Protection,
    ) -> Result<Blake3Hash> {
        self.activity.touch();
        let path = std::path::absolute(path)?;
        if !path.is_file() {
            bail!("{} is not a file", path.display());
        }
        let mode = match mode {
            ImportMode::Copy => iroh_blobs::api::proto::ImportMode::Copy,
            ImportMode::Reference => iroh_blobs::api::proto::ImportMode::TryReference,
        };
        let temp = self
            .store
            .blobs()
            .add_path_with_opts(AddPathOptions {
                path: path.clone(),
                format: BlobFormat::Raw,
                mode,
            })
            .temp_tag()
            .await
            .map_err(|e| anyhow!("importing {}: {e}", path.display()))?;
        let hash = core_hash(temp.hash());
        self.tag(&hash, &why).await?;
        Ok(hash)
    }

    pub async fn has(&self, hash: &Blake3Hash) -> Result<bool> {
        Ok(self.size(hash).await?.is_some())
    }

    /// Size in bytes if the blob is stored completely.
    pub async fn size(&self, hash: &Blake3Hash) -> Result<Option<u64>> {
        match self.store.blobs().status(iroh_hash(hash)).await? {
            BlobStatus::Complete { size } => Ok(Some(size)),
            _ => Ok(None),
        }
    }

    /// The whole blob. Verified against its hash by the store as it is read.
    pub async fn get(&self, hash: &Blake3Hash) -> Result<Bytes> {
        if !self.has(hash).await? {
            bail!("blob {hash} is not stored");
        }
        self.store
            .blobs()
            .get_bytes(iroh_hash(hash))
            .await
            .map_err(|e| anyhow!("reading {hash}: {e}"))
    }

    /// Bytes `range` of the blob (clamped to its size).
    pub async fn open_range(&self, hash: &Blake3Hash, range: Range<u64>) -> Result<Bytes> {
        let size = self
            .size(hash)
            .await?
            .with_context(|| format!("blob {hash} is not stored"))?;
        let end = range.end.min(size);
        if range.start >= end {
            return Ok(Bytes::new());
        }
        let mut reader = self.store.blobs().reader(iroh_hash(hash));
        reader.seek(SeekFrom::Start(range.start)).await?;
        let mut buf = vec![0u8; (end - range.start) as usize];
        reader.read_exact(&mut buf).await?;
        Ok(buf.into())
    }

    /// Add a protection to a blob that is already stored.
    pub async fn protect(&self, hash: &Blake3Hash, why: Protection) -> Result<()> {
        if !self.has(hash).await? {
            bail!("cannot protect {hash}: not stored");
        }
        self.tag(hash, &why).await
    }

    /// Drop one protection. The blob is collected once none remain.
    pub async fn unprotect(&self, hash: &Blake3Hash, why: Protection) -> Result<()> {
        self.store
            .tags()
            .delete(why.tag_name(hash))
            .await
            .map_err(|e| anyhow!("untagging {hash}: {e}"))?;
        Ok(())
    }

    /// Every blob this device holds on behalf of the library (those with a `lib/` protection).
    pub async fn library_protected(&self) -> Result<Vec<Blake3Hash>> {
        let mut out = vec![];
        let mut tags = self.store.tags().list_prefix("lib/").await?;
        while let Some(tag) = tags.next().await {
            let tag = tag?;
            let name = String::from_utf8_lossy(tag.name.as_ref()).to_string();
            if let Some(hex) = name.strip_prefix("lib/") {
                if let Ok(bytes) = ::hex::decode(hex) {
                    if let Ok(arr) = <[u8; 32]>::try_from(bytes.as_slice()) {
                        out.push(Blake3Hash(arr));
                    }
                }
            }
        }
        Ok(out)
    }

    /// Every protection currently holding `hash`, sorted (Library, Pin, InPlace, then albums).
    pub async fn protections(&self, hash: &Blake3Hash) -> Result<Vec<Protection>> {
        let mut out = vec![];
        for p in [Protection::Library, Protection::Pin, Protection::InPlace] {
            if self.store.tags().get(p.tag_name(hash)).await?.is_some() {
                out.push(p);
            }
        }
        let mut albums = self.store.tags().list_prefix("album/").await?;
        while let Some(tag) = albums.next().await {
            let tag = tag?;
            let name = String::from_utf8_lossy(tag.name.as_ref()).to_string();
            if let Some(rest) = name.strip_prefix("album/") {
                if let Some((album, h)) = rest.split_once('/') {
                    if h == hex(hash) {
                        let bytes = ::hex::decode(album).context("album tag")?;
                        let id: [u8; 16] =
                            bytes.try_into().map_err(|_| anyhow!("bad album tag"))?;
                        out.push(Protection::Album(AlbumId(id)));
                    }
                }
            }
        }
        out.sort();
        Ok(out)
    }

    /// Release everything the *library and the user* hold on `hash` (library reference and pin).
    /// Album references stay: a shared album may still need the blob. GC reclaims it afterwards.
    pub async fn remove(&self, hash: &Blake3Hash) -> Result<()> {
        self.unprotect(hash, Protection::Library).await?;
        self.unprotect(hash, Protection::Pin).await?;
        self.unprotect(hash, Protection::InPlace).await
    }
}
