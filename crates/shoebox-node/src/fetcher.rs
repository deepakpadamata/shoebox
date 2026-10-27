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
use shoebox_core::ids::{Blake3Hash, EndpointId};
use shoebox_core::model::{BlobKind, BlobRef, PhotoId, PhotoRecord};
use shoebox_core::policy::{
    fetch_plan, queue_order, rank_holders, Candidate, PathQuality, PlanContext, QueuedFetch,
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
            while futures::StreamExt::next(&mut changes).await.is_some() {
                let Some(inner) = weak.upgrade() else { break };
                inner.fetcher.wake.notify_one();
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
        self.is_recent(rec, config) || (config.keep_favorites && rec.favorite.value)
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
        for rec in photos {
            let own_below = rec.imported_by == self.device()
                && self
                    .store()
                    .holders(&rec.original.hash)
                    .await?
                    .iter()
                    .filter(|h| h.kind == BlobKind::Original)
                    .count()
                    < target;
            let ctx = PlanContext {
                pinned: pinned.contains(&rec.id.to_string()),
                own_capture_below_target: own_below,
                recent: self.is_recent(&rec, &config),
                kept_favorite: config.keep_favorites && rec.favorite.value,
            };
            for item in fetch_plan(role, &rec, &ctx) {
                if !self.node().blobs().has(&item.blob.hash).await? {
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
        if self.node().blobs().has(&blob.hash).await? {
            bail!("already present");
        }
        let holders = self.ranked_holders(&blob.hash).await?;
        if holders.is_empty() {
            bail!("no known holder of {}", blob.hash);
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
            .fetch_blob_from(&ids, &blob.hash, Some(blob.size), Protection::Library)
            .await?;
        let device = report
            .main()
            .and_then(|id| by_iroh.get(&id).copied())
            .unwrap_or(holders[0]);
        Ok(FetchSource { device })
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

async fn fetch_loop<L: LibraryStore>(
    weak: Weak<crate::library_node::Inner<L>>,
    cfg: FetcherConfig,
) {
    let permits = Arc::new(Semaphore::new(cfg.concurrency.max(1)));
    loop {
        let Some(inner) = weak.upgrade() else { break };
        let ln = LibraryNode { inner };
        let wake = {
            // Wait for a signal or the next tick.
            let inner = ln.inner.clone();
            let tick = cfg.tick;
            async move {
                let _ = tokio::time::timeout(tick, inner.fetcher.wake.notified()).await;
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
                continue;
            }
        };
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
                    match ln2.fetch_from_holders(&item.item.blob).await {
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
                let _ = ln.publish_holdings(fetched).await;
                // New holdings can unblock others and change "own capture below target".
                ln.inner.fetcher.wake.notify_one();
            }
            if ln.fetcher_paused() {
                break;
            }
        }
    }
}
