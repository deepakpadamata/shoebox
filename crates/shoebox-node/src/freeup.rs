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
//!
//! Previews go the same way without being asked for ([`LibraryNode::release_previews`]): a device
//! that holds an original the app shows as it is needs no preview of it, once a keeper confirmed
//! it holds that preview (keepers keep every preview, so it is never lost).

use crate::addr::to_iroh_id;
use crate::blobs::{BlobStore, Protection};
use crate::node::{Node, HAS_ALPN};
use crate::LibraryNode;
use anyhow::{bail, ensure, Context, Result};
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use shoebox_core::ids::{Blake3Hash, EndpointId};
use shoebox_core::model::{BlobKind, PhotoId, Role};
use shoebox_core::policy::shows_directly;
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
    /// Of `photos`, those whose only other copy is the encrypted one in Shoebox Cloud (W1-09). The confirmation
    /// says so in its own words: a copy that is one account and one service away, restorable with the recovery kit.
    pub cloud_only: Vec<PhotoId>,
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

    /// Non-revoked devices with the Keeper role.
    async fn keepers(&self) -> Result<HashSet<EndpointId>> {
        Ok(self
            .store()
            .device_list()
            .await?
            .devices
            .iter()
            .filter(|d| d.role == Role::Keeper && !d.revoked)
            .map(|d| d.device)
            .collect())
    }

    /// Release the previews this device keeps next to an original it shows as it is (JPEG, PNG):
    /// the viewer shows the original, so the preview only takes room. A preview goes only once a
    /// keeper confirmed (within seven days) that it holds it, so it is never the last copy; a
    /// preview a shared album uses stays. Keepers keep theirs. Returns the released previews; the
    /// blob store collects them shortly after.
    pub async fn release_previews(&self) -> Result<Vec<Blake3Hash>> {
        if self.node().config().role == Role::Keeper {
            return Ok(vec![]);
        }
        let me = self.device();
        // A device that is the whole library has nobody to keep a preview for: its original is
        // the copy, and a preview is made again if another device joins (`restore_previews`).
        let alone = self.drops_previews() && self.is_alone().await?;
        let keepers = self.keepers().await?;
        if keepers.is_empty() && !alone {
            return Ok(vec![]);
        }
        let photos = self
            .store()
            .photos_by_time(TimeRange::ALL, Page::all())
            .await?;
        if alone {
            let mut released = vec![];
            for rec in &photos {
                let preview = rec.preview.hash;
                let blobs = self.node().blobs();
                if preview == rec.original.hash
                    || !shows_directly(&rec.original.mime)
                    || !blobs.kept(&rec.original.hash).await?
                    || !blobs.kept(&preview).await?
                    || blobs
                        .protections(&preview)
                        .await?
                        .iter()
                        .any(|p| matches!(p, Protection::Album(_)))
                {
                    continue;
                }
                released.push(preview);
            }
            return self.drop_released_previews(released).await;
        }
        // (preview, keepers that say they hold it)
        let mut candidates: Vec<(Blake3Hash, Vec<EndpointId>)> = vec![];
        for rec in &photos {
            let preview = rec.preview.hash;
            if preview == rec.original.hash || !shows_directly(&rec.original.mime) {
                continue;
            }
            let blobs = self.node().blobs();
            if !blobs.kept(&rec.original.hash).await? || !blobs.kept(&preview).await? {
                continue;
            }
            if blobs
                .protections(&preview)
                .await?
                .iter()
                .any(|p| matches!(p, Protection::Album(_)))
            {
                continue;
            }
            let held_by: Vec<EndpointId> = self
                .store()
                .holders(&preview)
                .await?
                .into_iter()
                .filter(|h| h.kind == BlobKind::Preview && h.device != me)
                .map(|h| h.device)
                .filter(|d| keepers.contains(d))
                .collect();
            if !held_by.is_empty() {
                candidates.push((preview, held_by));
            }
        }

        // Ask only about what no keeper confirmed recently.
        let now = self.now_ms();
        let mut by_holder: HashMap<EndpointId, Vec<Blake3Hash>> = HashMap::new();
        {
            let confirmations = self.inner.confirmations.lock().unwrap();
            for (preview, held_by) in &candidates {
                if !held_by.iter().any(|d| confirmations.fresh(d, preview, now)) {
                    for d in held_by {
                        by_holder.entry(*d).or_default().push(*preview);
                    }
                }
            }
        }
        for (device, hashes) in by_holder {
            let _ = self.probe_holder(&device, &hashes).await;
        }

        let now = self.now_ms();
        let released: Vec<Blake3Hash> = {
            let confirmations = self.inner.confirmations.lock().unwrap();
            candidates
                .into_iter()
                .filter(|(preview, held_by)| {
                    held_by.iter().any(|d| confirmations.fresh(d, preview, now))
                })
                .map(|(preview, _)| preview)
                .collect()
        };
        self.drop_released_previews(released).await
    }

    async fn drop_released_previews(&self, released: Vec<Blake3Hash>) -> Result<Vec<Blake3Hash>> {
        if !released.is_empty() {
            let _exclusive = self.inner.ingest.write().await;
            for hash in &released {
                self.node()
                    .blobs()
                    .unprotect(hash, Protection::Library)
                    .await?;
                self.node().blobs().unprotect(hash, Protection::Pin).await?;
            }
            drop(_exclusive);
            self.retract_holdings(released.clone()).await?;
        }
        Ok(released)
    }

    /// The originals this device could drop right now. Probes the other holders first (failures
    /// are ignored: an unreachable holder simply stops being confirmed).
    async fn freeable(&self) -> Result<Vec<(PhotoId, u64, bool)>> {
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

        let keepers = self.keepers().await?;
        let in_cloud = self.cloud_held(self.now_ms());

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
            if enough || others.iter().any(|d| keepers.contains(d)) || in_cloud.contains(&hash) {
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
            .filter_map(|(id, size, others, enough)| {
                let confirmed = |d: &&EndpointId| confirmations.fresh(d, &id, now);
                let by_devices = (enough && others.iter().any(|d| confirmed(&d)))
                    || others
                        .iter()
                        .filter(|d| keepers.contains(d))
                        .any(|d| confirmed(&d));
                if by_devices {
                    Some((id, size, false))
                } else {
                    in_cloud.contains(&id).then_some((id, size, true))
                }
            })
            .collect())
    }

    /// The videos of Live Photos that go with these originals and are safe to drop with them: held
    /// here (not a gallery file, which freeing would not remove) and held by a keeper.
    async fn freeable_videos(
        &self,
        photos: &[(PhotoId, u64, bool)],
    ) -> Result<Vec<(Blake3Hash, u64)>> {
        let me = self.device();
        let keepers = self.keepers().await?;
        let mut out = vec![];
        for (id, _, _) in photos {
            let Some(motion) = self.store().motion(id).await? else {
                continue;
            };
            let hash = motion.video.hash;
            let Some(size) = self.node().blobs().size(&hash).await? else {
                continue;
            };
            if self
                .node()
                .blobs()
                .protections(&hash)
                .await?
                .contains(&Protection::InPlace)
            {
                continue;
            }
            let on_keeper = self.store().holders(&hash).await?.iter().any(|h| {
                h.kind == BlobKind::Motion && h.device != me && keepers.contains(&h.device)
            });
            if on_keeper {
                out.push((hash, size));
            }
        }
        Ok(out)
    }

    /// Free the originals that are safe to drop. With `dry_run` nothing changes; the report is
    /// what a real run would free. A real run first fetches the preview of each photo whose
    /// preview this device let go (it shows the photo from now on; a photo whose preview cannot
    /// be fetched keeps its original), then releases this device's protection (the blob store
    /// collects the bytes shortly after) and retracts the holdings.
    pub async fn free_up_space(&self, dry_run: bool) -> Result<FreeReport> {
        let mut freeable = self.freeable().await?;
        if !dry_run {
            let mut kept = Vec::with_capacity(freeable.len());
            for (id, size, cloud) in freeable {
                let Some(rec) = self.store().photo(&id).await? else {
                    continue;
                };
                let preview = rec.preview.hash;
                let needed =
                    preview != rec.original.hash && !self.node().blobs().kept(&preview).await?;
                if needed && self.fetch_now(&id, BlobKind::Preview).await.is_err() {
                    continue;
                }
                kept.push((id, size, cloud));
            }
            freeable = kept;
        }
        // A Live Photo's video goes with its original.
        let videos = self.freeable_videos(&freeable).await?;
        let report = FreeReport {
            bytes: freeable.iter().map(|(_, s, _)| s).sum::<u64>()
                + videos.iter().map(|(_, s)| s).sum::<u64>(),
            photos: freeable.iter().map(|(id, _, _)| *id).collect(),
            cloud_only: freeable
                .iter()
                .filter(|(_, _, c)| *c)
                .map(|(id, _, _)| *id)
                .collect(),
        };
        if !dry_run && !report.photos.is_empty() {
            let _exclusive = self.inner.ingest.write().await;
            for id in &report.photos {
                self.node().blobs().remove(id).await?;
            }
            for (hash, _) in &videos {
                self.node().blobs().remove(hash).await?;
            }
            let hexes: HashSet<String> = report.photos.iter().map(|h| h.to_string()).collect();
            self.node()
                .update_config(|c| c.pinned_originals.retain(|p| !hexes.contains(p)))?;
            drop(_exclusive);
            let mut released = report.photos.clone();
            released.extend(videos.iter().map(|(hash, _)| *hash));
            self.retract_holdings(released).await?;
        }
        Ok(report)
    }
}
