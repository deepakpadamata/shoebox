//! Holdings and copy counts (M1-11): "if I lose this device, what do I lose?"

use crate::library_node::LibraryNode;
use anyhow::{bail, Result};
use shoebox_core::ids::Blake3Hash;
use shoebox_core::model::{BlobKind, Holding, PhotoId};
use shoebox_core::store::{HoldingsDelta, LibraryStore, Page};

/// Library setting holding the number of copies every original should have.
pub const REDUNDANCY_KEY: &str = "redundancy.target";
/// Default number of copies (this device plus one more).
pub const DEFAULT_REDUNDANCY: u8 = 2;

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

    /// Photos (not deleted) with fewer originals than the redundancy target, in id order.
    pub async fn at_risk(&self, page: Page) -> Result<Vec<PhotoId>> {
        let target = self.redundancy_target().await?;
        Ok(self.store().at_risk(target, page).await?)
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
