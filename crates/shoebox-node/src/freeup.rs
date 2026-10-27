//! Safe "free up space" (M2-08): drop this device's copy of an original only when enough *other*
//! devices really hold it.
//!
//! "Really hold" means either of two things, each confirmed: other devices' originals number at
//! least the redundancy target (counting only originals on other devices) and at least one of
//! them answered a `shoebox/has/0` probe for that blob within the last 7 days; **or** a keeper (a
//! certified, non-revoked device with the Keeper role) holds the original and itself answered such
//! a probe within 7 days. The second is the same rule as "backed up" (a keeper is the backstop),
//! so a photo the app shows as backed up can also be freed (R1-03). Pinned originals and blobs
//! that a shared album still references are never freed. Keepers never free space.

use crate::addr::to_iroh_id;
use crate::blobs::{BlobStore, Protection};
use crate::node::{Node, HAS_ALPN};
use crate::LibraryNode;
use anyhow::{bail, ensure, Context, Result};
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use shoebox_core::ids::{Blake3Hash, EndpointId};
use shoebox_core::model::{BlobKind, PhotoId, Role};
use shoebox_core::store::{LibraryStore, Page, TimeRange};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long a holder's confirmation counts.
pub const CONFIRMATION_TTL_MS: u64 = 7 * 24 * 60 * 60 * 1000;
const PROBE_BATCH: usize = 1024;
const PROBE_TIMEOUT: Duration = Duration::from_secs(8);
const FILE: &str = "holder-confirmations.json";

/// What a free-up run frees (or would free).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FreeReport {
    pub photos: Vec<PhotoId>,
    pub bytes: u64,
}

// ---------- the probe protocol ----------

/// Answers "do you hold these blobs completely?" for up to [`PROBE_BATCH`] hashes.
#[derive(Clone)]
pub(crate) struct HasProtocol {
    pub blobs: BlobStore,
}

impl std::fmt::Debug for HasProtocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HasProtocol")
    }
}

impl ProtocolHandler for HasProtocol {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let (mut send, mut recv) = connection.accept_bi().await?;
        let request = recv
            .read_to_end(PROBE_BATCH * 32 + 8)
            .await
            .map_err(AcceptError::from_err)?;
        let hashes: Vec<Blake3Hash> = postcard::from_bytes(&request).unwrap_or_default();
        let mut have = Vec::with_capacity(hashes.len().min(PROBE_BATCH));
        for h in hashes.iter().take(PROBE_BATCH) {
            have.push(self.blobs.has(h).await.unwrap_or(false));
        }
        let reply = postcard::to_stdvec(&have).map_err(AcceptError::from_err)?;
        send.write_all(&reply)
            .await
            .map_err(AcceptError::from_err)?;
        send.finish()?;
        // The peer closes once it has read the reply; do not wait on it for longer than that.
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), connection.closed()).await;
        Ok(())
    }
}

impl Node {
    /// Ask `peer` which of `hashes` it holds completely.
    pub async fn peer_has(
        &self,
        peer: iroh::EndpointId,
        hashes: &[Blake3Hash],
    ) -> Result<Vec<bool>> {
        ensure!(hashes.len() <= PROBE_BATCH, "too many hashes in one probe");
        let probe = async {
            let conn = self
                .endpoint()
                .connect(peer, HAS_ALPN)
                .await
                .context("connecting")?;
            let (mut send, mut recv) = conn.open_bi().await?;
            send.write_all(&postcard::to_stdvec(hashes)?).await?;
            send.finish()?;
            let reply = recv.read_to_end(PROBE_BATCH * 2).await?;
            conn.close(0u32.into(), b"done");
            let have: Vec<bool> = postcard::from_bytes(&reply)?;
            ensure!(have.len() == hashes.len(), "malformed probe reply");
            Ok(have)
        };
        tokio::time::timeout(PROBE_TIMEOUT, probe)
            .await
            .map_err(|_| anyhow::anyhow!("probe timed out"))?
    }
}

// ---------- confirmations ----------

#[derive(Default)]
pub(crate) struct Confirmations {
    when: HashMap<(EndpointId, Blake3Hash), u64>,
    path: Option<PathBuf>,
}

impl Confirmations {
    pub(crate) fn load(data_dir: &Path) -> Self {
        let path = data_dir.join(FILE);
        let when = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Vec<(String, u64)>>(&b).ok())
            .map(|rows| {
                rows.into_iter()
                    .filter_map(|(k, t)| {
                        let (d, h) = k.split_once(':')?;
                        let d: [u8; 32] = hex::decode(d).ok()?.try_into().ok()?;
                        let h: [u8; 32] = hex::decode(h).ok()?.try_into().ok()?;
                        Some(((EndpointId(d), Blake3Hash(h)), t))
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self {
            when,
            path: Some(path),
        }
    }

    fn save(&self) {
        let Some(path) = &self.path else { return };
        let rows: Vec<(String, u64)> = self
            .when
            .iter()
            .map(|((d, h), t)| (format!("{}:{}", hex::encode(d.0), hex::encode(h.0)), *t))
            .collect();
        if let Ok(bytes) = serde_json::to_vec(&rows) {
            let _ = std::fs::write(path, bytes);
        }
    }

    fn record(&mut self, device: EndpointId, hashes: &[Blake3Hash], now: u64) {
        for h in hashes {
            self.when.insert((device, *h), now);
        }
        self.save();
    }

    fn forget(&mut self, device: EndpointId, hash: Blake3Hash) {
        self.when.remove(&(device, hash));
    }

    fn fresh(&self, device: &EndpointId, hash: &Blake3Hash, now: u64) -> bool {
        self.when
            .get(&(*device, *hash))
            .is_some_and(|t| now.saturating_sub(*t) <= CONFIRMATION_TTL_MS)
    }
}

impl<L: LibraryStore> LibraryNode<L> {
    /// Ask `device` which of `hashes` it holds. Hashes it confirms are remembered (for seven
    /// days) as confirmed holdings; ones it denies are forgotten.
    pub async fn probe_holder(
        &self,
        device: &EndpointId,
        hashes: &[Blake3Hash],
    ) -> Result<Vec<bool>> {
        let peer = to_iroh_id(device)?;
        let mut answers = Vec::with_capacity(hashes.len());
        for chunk in hashes.chunks(PROBE_BATCH) {
            let got = self.node().peer_has(peer, chunk).await?;
            let now = self.now_ms();
            let mut c = self.inner.confirmations.lock().unwrap();
            let yes: Vec<Blake3Hash> = chunk
                .iter()
                .zip(&got)
                .filter(|(_, g)| **g)
                .map(|(h, _)| *h)
                .collect();
            c.record(*device, &yes, now);
            for (h, g) in chunk.iter().zip(&got) {
                if !g {
                    c.forget(*device, *h);
                }
            }
            answers.extend(got);
        }
        Ok(answers)
    }

    /// The originals this device could drop right now. Probes the other holders first (failures
    /// are ignored: an unreachable holder simply stops being confirmed).
    async fn freeable(&self) -> Result<Vec<(PhotoId, u64)>> {
        let role = self.node().config().role;
        if role == Role::Keeper {
            bail!("a keeper never frees space: it is the backstop");
        }
        let me = self.device();
        let config = self.node().config();
        let target = self.redundancy_target().await? as usize;
        let photos = self
            .store()
            .photos_by_time(TimeRange::ALL, Page::all())
            .await?;

        let keepers: HashSet<EndpointId> = self
            .store()
            .device_list()
            .await?
            .devices
            .iter()
            .filter(|d| d.role == Role::Keeper && !d.revoked)
            .map(|d| d.device)
            .collect();

        // (photo, size, other holders, enough of them for the target)
        let mut candidates: Vec<(PhotoId, u64, Vec<EndpointId>, bool)> = vec![];
        for rec in &photos {
            let hash = rec.original.hash;
            let Some(size) = self.node().blobs().size(&hash).await? else {
                continue;
            };
            if self.is_pinned(&rec.id) {
                continue;
            }
            if self.kept_by_policy(rec, &config) {
                continue; // recent, or a favourite this device keeps
            }
            let protections = self.node().blobs().protections(&hash).await?;
            if protections
                .iter()
                .any(|p| matches!(p, Protection::Album(_)))
            {
                continue; // a shared album still needs this blob here
            }
            if protections.contains(&Protection::InPlace) {
                continue; // a gallery file: freeing would only drop the reference, not the file
            }
            let others: Vec<EndpointId> = self
                .store()
                .holders(&hash)
                .await?
                .into_iter()
                .filter(|h| h.kind == BlobKind::Original && h.device != me)
                .map(|h| h.device)
                .collect();
            let enough = others.len() >= target.max(1);
            if enough || others.iter().any(|d| keepers.contains(d)) {
                candidates.push((rec.id, size, others, enough));
            }
        }

        // Confirm with each holder that it really has what it claims.
        let mut by_holder: HashMap<EndpointId, Vec<Blake3Hash>> = HashMap::new();
        for (id, _, others, _) in &candidates {
            for d in others {
                by_holder.entry(*d).or_default().push(*id);
            }
        }
        for (device, hashes) in by_holder {
            let _ = self.probe_holder(&device, &hashes).await;
        }

        let now = self.now_ms();
        let confirmations = self.inner.confirmations.lock().unwrap();
        Ok(candidates
            .into_iter()
            .filter(|(id, _, others, enough)| {
                let confirmed = |d: &&EndpointId| confirmations.fresh(d, id, now);
                (*enough && others.iter().any(|d| confirmed(&d)))
                    || others
                        .iter()
                        .filter(|d| keepers.contains(d))
                        .any(|d| confirmed(&d))
            })
            .map(|(id, size, _, _)| (id, size))
            .collect())
    }

    /// Free the originals that are safe to drop. With `dry_run` nothing changes; the report is
    /// exactly what a real run would free. A real run releases this device's protection (the blob
    /// store collects the bytes shortly after) and retracts the holdings.
    pub async fn free_up_space(&self, dry_run: bool) -> Result<FreeReport> {
        let freeable = self.freeable().await?;
        let report = FreeReport {
            bytes: freeable.iter().map(|(_, s)| s).sum(),
            photos: freeable.iter().map(|(id, _)| *id).collect(),
        };
        if !dry_run && !report.photos.is_empty() {
            let _exclusive = self.inner.ingest.write().await;
            for id in &report.photos {
                self.node().blobs().remove(id).await?;
            }
            let hexes: HashSet<String> = report.photos.iter().map(|h| h.to_string()).collect();
            self.node()
                .update_config(|c| c.pinned_originals.retain(|p| !hexes.contains(p)))?;
            drop(_exclusive);
            self.retract_holdings(report.photos.clone()).await?;
        }
        Ok(report)
    }
}
