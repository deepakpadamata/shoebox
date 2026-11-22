//! Role-driven blob fetching (M1-12): turns the policy in `shoebox_core::policy` into downloads.
//!
//! A background task keeps asking "what should this device hold that it does not?", queues it
//! thumbnails → previews → originals (newest first), picks a holder for each blob (best network
//! path first, then the most recently seen), downloads it with verification, and publishes the
//! new holding. iroh-blobs keeps partial blobs, so an interrupted download resumes with only the
//! missing ranges.

use crate::blobs::Protection;
use crate::config::NodeConfig;
use crate::library_node::LibraryNode;
use anyhow::{anyhow, bail, Result};
use shoebox_core::ids::{Blake3Hash, EndpointId, Timestamp};
use shoebox_core::model::{BlobKind, BlobRef, MotionRecord, MotionSource, PhotoId, PhotoRecord};
use shoebox_core::policy::{
    fetch_plan, motion_plan, queue_order, rank_holders, Candidate, PathQuality, PlanContext,
    QueuedFetch,
};
use shoebox_core::store::{LibraryStore, Page, TimeRange};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};
use tokio::sync::{Notify, Semaphore};
use tracing::{debug, warn};

/// How long a part-downloaded blob stays protected from GC while waiting for a retry.
const PARTIAL_HOLD: Duration = Duration::from_secs(3600);

/// Blobs fetched per batch before their holdings are published.
const CHUNK: usize = 64;

/// After a plan with nothing to fetch, look again after this many ticks (or on a relevant
/// change): a fully fetched library has nothing that becomes missing by time alone, and each
/// plan walks every photo.
const IDLE_TICKS: u32 = 60;

#[derive(Clone, Debug)]
pub struct FetcherConfig {
    /// Downloads in flight at once.
    pub concurrency: usize,
    /// How often to look for newly missing blobs even if nothing signalled a change.
    pub tick: Duration,
    /// Wait this long before retrying a blob whose download failed.
    pub retry: Duration,
}

impl Default for FetcherConfig {
    fn default() -> Self {
        Self {
            concurrency: 4,
            tick: Duration::from_secs(5),
            retry: Duration::from_secs(10),
        }
    }
}

/// Which device a blob was fetched from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FetchSource {
    pub device: EndpointId,
}

#[derive(Default)]
pub(crate) struct FetcherState {
    pub paused: AtomicBool,
    pub wake: Notify,
    pub running: AtomicBool,
    /// Blobs with a download in flight or cooling down after a failure.
    busy: Mutex<HashMap<Blake3Hash, Option<Instant>>>,
    /// Temp tags on blobs whose download failed part-way, so blob GC does not discard the
    /// progress before the retry (kept until the blob completes or [`PARTIAL_HOLD`] passes).
    partials: Mutex<HashMap<Blake3Hash, (Instant, iroh_blobs::api::TempTag)>>,
}

fn path_quality(kind: &crate::node::PathKind) -> PathQuality {
    match kind {
        crate::node::PathKind::Lan => PathQuality::Lan,
        crate::node::PathKind::Direct => PathQuality::Direct,
        crate::node::PathKind::Relay { .. } => PathQuality::Relay,
        crate::node::PathKind::Unknown => PathQuality::Unknown,
    }
}

impl<L: LibraryStore> LibraryNode<L> {
    /// Start the background fetcher (idempotent). It follows this device's role.
    pub fn start_fetcher(&self, cfg: FetcherConfig) {
        if self.inner.fetcher.running.swap(true, Ordering::SeqCst) {
            return;
        }
        let weak = Arc::downgrade(&self.inner);
        tokio::spawn(fetch_loop(weak, cfg));
        // Wake on library changes: new photos and new holdings both create work.
        let weak = Arc::downgrade(&self.inner);
        let mut changes = self.inner.store.subscribe();
        tokio::spawn(async move {
            while let Some(change) = futures::StreamExt::next(&mut changes).await {
                let Some(inner) = weak.upgrade() else { break };
                if wakes_fetcher(&change) {
                    inner.fetcher.wake.notify_one();
                }
            }
        });
    }

    pub fn pause_fetcher(&self) {
        self.inner.fetcher.paused.store(true, Ordering::SeqCst);
    }

    pub fn resume_fetcher(&self) {
        self.inner.fetcher.paused.store(false, Ordering::SeqCst);
        self.inner.fetcher.wake.notify_one();
    }

    pub fn fetcher_paused(&self) -> bool {
        self.inner.fetcher.paused.load(Ordering::SeqCst)
    }

    // ---------- pinned originals ----------

    /// Keep the original of `photo` on this device (phones only fetch originals of pinned or
    /// own photos). Persisted in this device's `config.toml`; not replicated.
    pub async fn pin_original(&self, photo: &PhotoId) -> Result<()> {
        let hex = photo.to_string();
        self.node().update_config(|c| {
            if !c.pinned_originals.contains(&hex) {
                c.pinned_originals.push(hex.clone());
            }
            c.offloaded_originals.retain(|h| h != &hex);
        })?;
        if self.node().blobs().has(photo).await? {
            self.node().blobs().protect(photo, Protection::Pin).await?;
        }
        self.inner.fetcher.wake.notify_one();
        Ok(())
    }

    pub async fn unpin_original(&self, photo: &PhotoId) -> Result<()> {
        let hex = photo.to_string();
        self.node()
            .update_config(|c| c.pinned_originals.retain(|h| h != &hex))?;
        self.node()
            .blobs()
            .unprotect(photo, Protection::Pin)
            .await?;
        Ok(())
    }

    /// Taken within the keep-recent window of `config`.
    pub(crate) fn is_recent(&self, rec: &PhotoRecord, config: &NodeConfig) -> bool {
        const DAY_MS: i64 = 86_400_000;
        config.keep_recent_days > 0
            && rec.taken_at.0 >= self.now_ms() as i64 - i64::from(config.keep_recent_days) * DAY_MS
    }

    /// Why a Companion keeps this original on its own account, if it does: the policy that free-up
    /// leaves alone (pins are checked by the caller).
    pub(crate) fn kept_by_policy(&self, rec: &PhotoRecord, config: &NodeConfig) -> bool {
        !is_offloaded(rec, config)
            && (self.is_recent(rec, config) || (config.keep_favorites && rec.favorite.value))
    }

    /// This device's keep rules: the recent window in days (0 = off) and whether favourites are
    /// kept. A Companion's fetcher wakes to apply them.
    pub fn keep_rules(&self) -> (u32, bool) {
        let c = self.node().config();
        (c.keep_recent_days, c.keep_favorites)
    }

    pub fn set_keep_rules(&self, recent_days: u32, favorites: bool) -> Result<()> {
        self.node().update_config(|c| {
            c.keep_recent_days = recent_days;
            c.keep_favorites = favorites;
        })?;
        self.inner.fetcher.wake.notify_one();
        Ok(())
    }

    pub fn is_pinned(&self, photo: &PhotoId) -> bool {
        self.node()
            .config()
            .pinned_originals
            .contains(&photo.to_string())
    }

    // ---------- planning ----------

    /// What decides whether this device wants the original of `rec` (see [`PlanContext`]).
    pub(crate) async fn plan_context(
        &self,
        rec: &PhotoRecord,
        config: &NodeConfig,
        pinned: &HashSet<String>,
        target: usize,
        protected: &HashSet<Blake3Hash>,
    ) -> Result<PlanContext> {
        let own_below = rec.imported_by == self.device()
            && self
                .store()
                .holders(&rec.original.hash)
                .await?
                .iter()
                .filter(|h| h.kind == BlobKind::Original)
                .count()
                < target;
        // Freed by hand: the keep rules no longer want it here (a pin would have cleared this).
        let offloaded = is_offloaded(rec, config);
        Ok(PlanContext {
            pinned: pinned.contains(&rec.id.to_string()),
            own_capture_below_target: own_below,
            recent: !offloaded && self.is_recent(rec, config),
            kept_favorite: !offloaded && config.keep_favorites && rec.favorite.value,
            holds_original: protected.contains(&rec.original.hash),
        })
    }

    /// Everything this device should hold according to its role but does not, in fetch order.
    pub async fn missing_blobs(&self) -> Result<Vec<QueuedFetch>> {
        let config = self.node().config();
        let role = config.role;
        let pinned: HashSet<String> = config.pinned_originals.iter().cloned().collect();
        let target = self.redundancy_target().await? as usize;
        let photos = self
            .store()
            .photos_by_time(TimeRange::ALL, Page::all())
            .await?;
        let mut queue = vec![];
        for item in self.shard_fetch_plan(photos.len()).await? {
            // Not a photo's blob: ordered by kind alone (newest-first does not apply).
            queue.push(QueuedFetch {
                photo: item.blob.hash,
                taken_at: Timestamp(0),
                item,
            });
        }
        let motions: std::collections::HashMap<PhotoId, MotionRecord> = self
            .store()
            .motions()
            .await?
            .into_iter()
            .map(|m| (m.photo, m))
            .collect();
        // One listing of the tags for the whole pass: asking `kept` per photo lists every album
        // tag each time, which grew with photos × shared-album items on every 5 s replan.
        // A tag also answers "held": only a complete blob is ever tagged. Asking the store
        // instead reads a small blob's whole inlined content to report its size, and the
        // database keeps every page it read cached (hundreds of MB once every thumbnail is asked).
        let protected = self.node().blobs().protected().await?;
        let holds = |hash: &Blake3Hash| {
            let tagged = protected.contains(hash);
            let hash = *hash;
            async move {
                if tagged {
                    return Ok::<_, anyhow::Error>(true);
                }
                self.node().blobs().has(&hash).await
            }
        };
        for rec in photos {
            let ctx = self
                .plan_context(&rec, &config, &pinned, target, &protected)
                .await?;
            // An embedded clip is part of the original: wanted with it, never fetched on its own.
            let video = motions
                .get(&rec.id)
                .filter(|m| m.source != MotionSource::Embedded)
                .map(|m| motion_plan(role, m, &ctx))
                .unwrap_or_default();
            for item in fetch_plan(role, &rec, &ctx).into_iter().chain(video) {
                if !holds(&item.blob.hash).await? {
                    queue.push(QueuedFetch {
                        photo: rec.id,
                        taken_at: rec.taken_at,
                        item,
                    });
                }
            }
        }
        queue_order(&mut queue);
        Ok(queue)
    }

    // ---------- fetching ----------

    /// Keep a temp tag on `hash` so a part-downloaded blob survives GC until the retry.
    async fn hold_partial(&self, hash: Blake3Hash) {
        let hf = iroh_blobs::HashAndFormat {
            hash: crate::blobs::iroh_hash(&hash),
            format: iroh_blobs::BlobFormat::Raw,
        };
        if let Ok(tag) = self.node().blobs().raw().tags().temp_tag(hf).await {
            let mut partials = self.inner.fetcher.partials.lock().unwrap();
            partials.insert(hash, (Instant::now(), tag));
            partials.retain(|_, (since, _)| since.elapsed() < PARTIAL_HOLD);
        }
    }

    /// Devices (other than this one) that hold `blob`, best first.
    async fn ranked_holders(&self, blob: &Blake3Hash) -> Result<Vec<EndpointId>> {
        let mut candidates = vec![];
        for holding in self.store().holders(blob).await? {
            if holding.device == self.device() {
                continue;
            }
            let (path, last_seen_ago_ms) = match crate::addr::to_iroh_id(&holding.device) {
                Ok(id) => match self.node().peer_info(id).await {
                    Some(info) => (
                        path_quality(&info.path),
                        info.last_seen.map(|t| t.elapsed().as_millis() as u64),
                    ),
                    None => (PathQuality::Unknown, None),
                },
                Err(_) => continue,
            };
            candidates.push(Candidate {
                device: holding.device,
                path,
                last_seen_ago_ms,
            });
        }
        rank_holders(&mut candidates);
        Ok(candidates.into_iter().map(|c| c.device).collect())
    }

    /// Download `blob` from its holders (several at once when it is large, A-099). Does not
    /// publish the holding.
    async fn fetch_from_holders(&self, blob: &BlobRef) -> Result<FetchSource> {
        self.fetch_from_holders_as(blob, Protection::Library).await
    }

    /// Fetch search data (a model file) from whoever holds it, kept as `idx/<hash>`.
    pub(crate) async fn fetch_blob_as_index(&self, blob: &BlobRef) -> Result<FetchSource> {
        self.fetch_from_holders_as(blob, Protection::Index).await
    }

    async fn fetch_from_holders_as(&self, blob: &BlobRef, why: Protection) -> Result<FetchSource> {
        if self.node().blobs().has(&blob.hash).await? {
            bail!("already present");
        }
        let mut holders = self.ranked_holders(&blob.hash).await?;
        if holders.is_empty() {
            bail!("no known holder of {}", blob.hash);
        }
        // A holder that said it is not sending originals (a phone on mobile data) is not asked
        // for one: it would refuse, and asking costs its data.
        if blob.size > crate::PAUSED_SERVE_MAX {
            holders.retain(|d| !self.holds_back_large(d));
            if holders.is_empty() {
                bail!(
                    "the devices holding {} are not sending originals right now (waiting for Wi-Fi)",
                    blob.hash
                );
            }
        }
        let mut by_iroh = HashMap::new();
        for device in &holders {
            let id = crate::addr::to_iroh_id(device).map_err(|e| anyhow!("{e}"))?;
            by_iroh.insert(id, *device);
        }
        let ids: Vec<_> = holders
            .iter()
            .filter_map(|d| crate::addr::to_iroh_id(d).ok())
            .collect();
        let report = self
            .node()
            .fetch_blob_from(&ids, &blob.hash, Some(blob.size), why)
            .await?;
        let device = report
            .main()
            .and_then(|id| by_iroh.get(&id).copied())
            .unwrap_or(holders[0]);
        Ok(FetchSource { device })
    }

    /// Bytes `range` of `blob`, which is not here, read from the devices that hold it (best
    /// first) without fetching the rest: a video plays while it streams (see
    /// [`Node::read_remote_range`](crate::Node::read_remote_range)). A device holding back large
    /// blobs (a phone on mobile data) is not asked. Pieces already here are read with no device
    /// online.
    pub async fn read_range_elsewhere(
        &self,
        blob: &BlobRef,
        range: std::ops::Range<u64>,
    ) -> Result<bytes::Bytes> {
        let mut holders = self.ranked_holders(&blob.hash).await?;
        if blob.size > crate::PAUSED_SERVE_MAX {
            holders.retain(|d| !self.holds_back_large(d));
        }
        let ids: Vec<_> = holders
            .iter()
            .filter_map(|d| crate::addr::to_iroh_id(d).ok())
            .collect();
        self.node()
            .read_remote_range(&ids, &blob.hash, blob.size, range)
            .await
    }

    /// Fetch one blob of `photo` right now (the user opened it), bypassing the queue and a paused
    /// fetcher. Returns the device it came from; if the blob is already local, that is this device.
    pub async fn fetch_now(&self, photo: &PhotoId, kind: BlobKind) -> Result<FetchSource> {
        let rec = self
            .store()
            .photo(photo)
            .await?
            .ok_or_else(|| anyhow!("unknown photo {photo}"))?;
        let blob_ref = match kind {
            BlobKind::Original => rec.original.clone(),
            BlobKind::Preview => rec.preview.clone(),
            BlobKind::Thumb => rec.thumb.clone(),
            BlobKind::Motion => {
                let m = self
                    .store()
                    .motion(photo)
                    .await?
                    .ok_or_else(|| anyhow!("photo {photo} has no video"))?;
                if m.source == MotionSource::Embedded {
                    // The clip is in the original: readable here, or came with the original.
                    if self.clip_source(photo).await?.is_some() {
                        return Ok(FetchSource {
                            device: self.device(),
                        });
                    }
                    // A device that still stores the clip from before serves it as a blob (a
                    // few MB); otherwise the original is fetched and the clip read from it.
                    let me = self.device();
                    let stored_elsewhere = self
                        .store()
                        .holders(&m.video.hash)
                        .await?
                        .iter()
                        .any(|h| h.kind == BlobKind::Motion && h.device != me);
                    if !stored_elsewhere {
                        let source = Box::pin(self.fetch_now(photo, BlobKind::Original)).await?;
                        anyhow::ensure!(
                            self.clip_source(photo).await?.is_some(),
                            "the video is not in the original of {photo}"
                        );
                        return Ok(source);
                    }
                }
                m.video
            }
            BlobKind::Index => bail!("index shards are not blobs of a photo"),
        };
        let blob = blob_ref.hash;
        if self.node().blobs().kept(&blob).await? {
            return Ok(FetchSource {
                device: self.device(),
            });
        }
        // Released and waiting for the store's collection (a gallery file this device stopped
        // referencing): the store would answer from it, so wait for it to go.
        let mut waited = 0;
        while self.node().blobs().has(&blob).await? {
            anyhow::ensure!(
                waited < 90,
                "this device is still letting go of its copy of that file; try again in a minute"
            );
            waited += 1;
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
        let source = self.fetch_from_holders(&blob_ref).await?;
        self.publish_holdings(vec![(blob, kind)]).await?;
        if kind == BlobKind::Original && self.is_pinned(photo) {
            self.node().blobs().protect(&blob, Protection::Pin).await?;
        }
        Ok(source)
    }
}

/// Whether a library change can make a blob missing here or fetchable: photos, videos, shards,
/// holdings (copies counted, sources), settings (the redundancy target) and devices. Albums,
/// people and sync progress cannot.
fn wakes_fetcher(change: &shoebox_core::store::Change) -> bool {
    use shoebox_core::store::Change;
    match change {
        Change::PhotoUpserted(_)
        | Change::PhotoDeleted(_)
        | Change::MotionChanged(_)
        | Change::IndexChanged
        | Change::HoldingsChanged { .. }
        | Change::SettingChanged(_)
        | Change::DeviceListChanged => true,
        Change::AlbumChanged(_) | Change::PeopleChanged | Change::RemoteSyncProgress { .. } => {
            false
        }
    }
}

async fn fetch_loop<L: LibraryStore>(
    weak: Weak<crate::library_node::Inner<L>>,
    cfg: FetcherConfig,
) {
    let permits = Arc::new(Semaphore::new(cfg.concurrency.max(1)));
    // How long to wait before the next plan when nothing signals a change.
    let mut wait = cfg.tick;
    // Plans in a row that fetched nothing although something was missing.
    let mut stuck = 0u32;
    loop {
        let Some(inner) = weak.upgrade() else { break };
        let ln = LibraryNode { inner };
        let wake = {
            // Wait for a signal or the next tick.
            let inner = ln.inner.clone();
            async move {
                let _ = tokio::time::timeout(wait, inner.fetcher.wake.notified()).await;
            }
        };
        wake.await;
        if ln.fetcher_paused() {
            continue;
        }
        let queue = match ln.missing_blobs().await {
            Ok(q) => q,
            Err(e) => {
                warn!("could not plan fetches: {e:#}");
                wait = cfg.tick;
                continue;
            }
        };
        let mut progressed = false;
        {
            // Cooling entries past their time say nothing an absent entry does not.
            let now = Instant::now();
            ln.inner
                .fetcher
                .busy
                .lock()
                .unwrap()
                .retain(|_, until| until.is_none_or(|t| t > now));
        }
        // Work through the queue in chunks so holdings are published as we go, not at the very end.
        for chunk in queue.chunks(CHUNK) {
            let mut handles = vec![];
            for item in chunk.iter().cloned() {
                let hash = item.item.blob.hash;
                {
                    let mut busy = ln.inner.fetcher.busy.lock().unwrap();
                    match busy.get(&hash) {
                        Some(None) => continue,                                   // in flight
                        Some(Some(until)) if *until > Instant::now() => continue, // cooling down
                        _ => {}
                    }
                    busy.insert(hash, None);
                }
                let Ok(permit) = permits.clone().acquire_owned().await else {
                    return;
                };
                if ln.fetcher_paused() {
                    ln.inner.fetcher.busy.lock().unwrap().remove(&hash);
                    break;
                }
                let ln2 = ln.clone();
                let retry = cfg.retry;
                handles.push(tokio::spawn(async move {
                    let _permit = permit;
                    let why = match item.item.kind {
                        BlobKind::Index => Protection::Index,
                        _ => Protection::Library,
                    };
                    match ln2.fetch_from_holders_as(&item.item.blob, why).await {
                        Ok(_) => {
                            ln2.inner.fetcher.busy.lock().unwrap().remove(&hash);
                            ln2.inner.fetcher.partials.lock().unwrap().remove(&hash);
                            let pinned = ln2.node().config().pinned_originals;
                            if item.item.kind == BlobKind::Original
                                && pinned.contains(&item.photo.to_string())
                            {
                                let _ = ln2.node().blobs().protect(&hash, Protection::Pin).await;
                            }
                            Some((hash, item.item.kind))
                        }
                        Err(e) => {
                            debug!("fetch of {hash} failed: {e:#}");
                            ln2.hold_partial(hash).await;
                            ln2.inner
                                .fetcher
                                .busy
                                .lock()
                                .unwrap()
                                .insert(hash, Some(Instant::now() + retry));
                            None
                        }
                    }
                }));
            }
            let mut fetched = vec![];
            for h in handles {
                if let Ok(Some(done)) = h.await {
                    fetched.push(done);
                }
            }
            if !fetched.is_empty() {
                progressed = true;
                let _ = ln.publish_holdings(fetched).await;
                // New holdings can unblock others and change "own capture below target".
                ln.inner.fetcher.wake.notify_one();
            }
            if ln.fetcher_paused() {
                break;
            }
        }
        stuck = if queue.is_empty() || progressed {
            0
        } else {
            stuck.saturating_add(1)
        };
        wait = next_wait(cfg.tick, queue.is_empty(), stuck);
    }
}

/// How long the fetcher waits for a change before planning again. Nothing missing waits the
/// longest. Blobs that stay missing (their holders are offline, or nobody holds them) back off
/// from one tick to that same ceiling, doubling each time: every plan walks every photo.
fn next_wait(tick: Duration, nothing_missing: bool, stuck: u32) -> Duration {
    let longest = tick * IDLE_TICKS;
    if nothing_missing {
        return longest;
    }
    tick.saturating_mul(1u32 << stuck.min(16)).min(longest)
}

/// The person freed this original from this device by hand (see `offloaded_originals`).
pub(crate) fn is_offloaded(rec: &PhotoRecord, config: &NodeConfig) -> bool {
    !config.offloaded_originals.is_empty()
        && config.offloaded_originals.contains(&rec.id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans_back_off_while_nothing_can_be_fetched() {
        let tick = Duration::from_secs(5);
        let longest = tick * IDLE_TICKS;
        assert_eq!(next_wait(tick, true, 0), longest);
        assert_eq!(next_wait(tick, false, 0), tick, "progress keeps the pace");
        assert_eq!(next_wait(tick, false, 1), tick * 2);
        assert_eq!(next_wait(tick, false, 3), tick * 8);
        assert_eq!(next_wait(tick, false, 6), longest);
        assert_eq!(next_wait(tick, false, u32::MAX), longest);
    }
}
