//! Holdings and copy counts (M1-11): "if I lose this device, what do I lose?"

use crate::library_node::LibraryNode;
use anyhow::{bail, Result};
use shoebox_core::ids::{Blake3Hash, EndpointId};
use shoebox_core::model::{BlobKind, Holding, PhotoId};
use shoebox_core::store::{HoldingsDelta, LibraryStore, Page, TimeRange};
use std::collections::{BTreeMap, HashSet};

/// Library setting holding the number of copies every original should have.
pub const REDUNDANCY_KEY: &str = "redundancy.target";
/// Default number of copies (this device plus one more).
pub const DEFAULT_REDUNDANCY: u8 = 2;

/// How much of the library one device holds, from the replicated holdings.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceStorage {
    /// Photos and videos whose original it holds.
    pub originals: u32,
    pub original_bytes: u64,
    /// Previews and thumbnails.
    pub other_bytes: u64,
}

impl<L: LibraryStore> LibraryNode<L> {
    /// Devices that hold `blob` (any kind), as this replica currently knows them. Devices that
    /// are offline still count: their data exists, we just cannot reach it right now.
    pub async fn holders(&self, blob: &Blake3Hash) -> Result<Vec<Holding>> {
        Ok(self.store().holders(blob).await?)
    }

    /// Distinct devices holding the *original* of `photo`. Zero for an unknown photo.
    pub async fn copy_count(&self, photo: &PhotoId) -> Result<usize> {
        let Some(record) = self.store().photo(photo).await? else {
            return Ok(0);
        };
        let holders = self.store().holders(&record.original.hash).await?;
        Ok(holders
            .iter()
            .filter(|h| h.kind == BlobKind::Original)
            .count())
    }

    /// The library's redundancy target (replicated setting, default [`DEFAULT_REDUNDANCY`]).
    pub async fn redundancy_target(&self) -> Result<u8> {
        Ok(match self.store().setting(REDUNDANCY_KEY).await? {
            Some(bytes) if bytes.len() == 1 && bytes[0] >= 1 => bytes[0],
            _ => DEFAULT_REDUNDANCY,
        })
    }

    pub async fn set_redundancy_target(&self, copies: u8) -> Result<()> {
        if copies == 0 {
            bail!("the redundancy target must be at least 1");
        }
        self.store()
            .put_setting(REDUNDANCY_KEY, vec![copies])
            .await?;
        Ok(())
    }

    /// Photos (not deleted) with fewer originals than the redundancy target and no keeper holding
    /// the original, in id order.
    pub async fn at_risk(&self, page: Page) -> Result<Vec<PhotoId>> {
        let target = self.redundancy_target().await?;
        Ok(self.store().at_risk(target, page).await?)
    }

    /// What each device holds of the library's photos (Recently Deleted included: they take
    /// room until they are purged), as this replica knows it. A blob two records share counts
    /// once per device. Devices that hold nothing are absent.
    pub async fn storage_by_device(&self) -> Result<BTreeMap<EndpointId, DeviceStorage>> {
        let mut records = self
            .store()
            .photos_by_time(TimeRange::ALL, Page::all())
            .await?;
        records.extend(self.store().deleted_photos().await?);
        let mut seen: HashSet<(EndpointId, Blake3Hash)> = HashSet::new();
        let mut out: BTreeMap<EndpointId, DeviceStorage> = BTreeMap::new();
        for rec in &records {
            // A video's thumbnail is the video itself: `seen` counts that blob once.
            for blob in [&rec.original, &rec.preview, &rec.thumb] {
                for h in self.store().holders(&blob.hash).await? {
                    if !seen.insert((h.device, blob.hash)) {
                        continue;
                    }
                    let entry = out.entry(h.device).or_default();
                    if h.kind == BlobKind::Original {
                        entry.originals += 1;
                        entry.original_bytes += blob.size;
                    } else {
                        entry.other_bytes += blob.size;
                    }
                }
            }
        }
        Ok(out)
    }

    /// Record that this device no longer holds `blobs`.
    pub async fn retract_holdings(&self, blobs: Vec<Blake3Hash>) -> Result<()> {
        if blobs.is_empty() {
            return Ok(());
        }
        let delta = HoldingsDelta {
            hlc: self.holdings_hlc(),
            added: vec![],
            removed: blobs,
        };
        self.store().put_holdings(self.device(), delta).await?;
        Ok(())
    }
}
