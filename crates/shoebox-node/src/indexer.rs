//! The indexer (S1-11): turns photos' thumbnails into picture embeddings and publishes them as
//! shards, sharing the work with the other indexing devices without claims (epic S1 §5.2):
//!
//! 1. a device indexes its **own imports**;
//! 2. the **backlog** is shared by weighted rendezvous among always-on indexers
//!    ([`shoebox_core::policy::index_owner`]), weighted by their measured speed;
//! 3. **anyone** takes over a photo still unindexed 48 hours after this device could first have
//!    done it, so a phone that never charges or a keeper that died leaves no gap for good.
//!
//! Results wait in memory and go out as a `vec8`, a `vec1` and a `text` shard (labels, S1-13)
//! every 256 photos or 10 minutes. A crash loses at most that buffer, never publishes half a
//! shard.

use crate::library_node::LibraryNode;
use anyhow::{Context, Result};
use shoebox_core::ids::{Blake3Hash, EndpointId};
use shoebox_core::model::{IndexerStatus, ModelId, PhotoId, PhotoRecord, Role, ShardKind};
use shoebox_core::policy::{index_owner, IndexCandidate};
use shoebox_core::store::{LibraryStore, Page, TimeRange};
use shoebox_search::encoder::{ImageEncoder, Rgb};
use shoebox_search::faces::{FaceDetector, FaceEmbedder, MIN_FACE_PX};
use shoebox_search::labels::{Labels, LABELS_VERSION, MIN_PROBABILITY};
use shoebox_search::ocr::{wants_text, Ocr, OCR_ENGINE, OCR_MARK};
use shoebox_search::shard::Face;
use shoebox_search::shard::{sign_bits, Rows, Shard, TextRow, Vec8};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};
use tokio::sync::Notify;
use tracing::{debug, info, warn};

/// Whether this device may index right now (desktop: on mains power and idle; phone: charging).
pub trait IndexGate: Send + Sync + 'static {
    fn may_index(&self) -> bool;
}

/// Always (a keeper).
pub struct Always;

impl IndexGate for Always {
    fn may_index(&self) -> bool {
        true
    }
}

#[derive(Clone, Debug)]
pub struct IndexerConfig {
    /// Look for work at least this often (and whenever the library changes).
    pub tick: Duration,
    /// Photos encoded per step (the pause and the gate are checked between steps).
    pub batch: usize,
    /// Photos per shard.
    pub shard_size: usize,
    /// A smaller shard goes out once its oldest result is this old.
    pub shard_after: Duration,
    /// Anyone indexes a photo still unindexed this long after this device could have.
    pub grace: Duration,
    /// Encoder threads (keeper: up to half the cores; desktop: half).
    pub threads: usize,
    /// Shares the backlog (a keeper; a desktop set to index while plugged in).
    pub always_on: bool,
    /// Photos per minute to announce instead of the measured rate (tests).
    pub rate_override: Option<u32>,
    /// Announce a changed rate at most this often.
    pub status_every: Duration,
    /// Compact shards (an always-on indexer only) at most this often (S1-15).
    pub compact_every: Duration,
}

impl Default for IndexerConfig {
    fn default() -> Self {
        Self {
            tick: Duration::from_secs(30),
            batch: 16,
            shard_size: 256,
            shard_after: Duration::from_secs(600),
            grace: Duration::from_secs(48 * 3600),
            threads: 1,
            always_on: false,
            rate_override: None,
            status_every: Duration::from_secs(600),
            compact_every: Duration::from_secs(3600),
        }
    }
}

/// How far indexing has come on this device (S1-16).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndexProgress {
    pub model: Option<ModelId>,
    /// Photos with a picture embedding (from any device), of `total`.
    pub indexed: usize,
    pub total: usize,
    /// Photos this device is to do now.
    pub mine_left: usize,
    /// Results waiting for the next shard.
    pub buffered: usize,
    pub photos_per_min: u32,
    pub running: bool,
    pub paused: bool,
    /// Why it is not indexing (gate closed), if so.
    pub waiting: Option<String>,
}

#[derive(Default)]
pub(crate) struct IndexerState {
    running: AtomicBool,
    paused: AtomicBool,
    wake: Notify,
    buffer: Mutex<Buffer>,
    progress: Mutex<IndexProgress>,
    /// This device's text reader (S1-14), if it has one.
    ocr: Mutex<Option<Arc<dyn Ocr>>>,
    /// Read text waiting for the next shard.
    ocr_rows: Mutex<ReadText>,
    /// This device's face models (S1-17), if it has them.
    faces: Mutex<Option<FaceModels>>,
    /// Faces found, waiting for the next shard.
    face_rows: Mutex<FoundFaces>,
}

/// The face detector and embedder, and the id their results are recorded under.
#[derive(Clone)]
pub struct FaceModels {
    pub detector: Arc<dyn FaceDetector>,
    pub embedder: Arc<dyn FaceEmbedder>,
    pub model: ModelId,
}

/// How often the grouping device looks at whether faces should be grouped again.
const REGROUP_EVERY: Duration = Duration::from_secs(600);

/// Library setting: people (faces) on or off for the whole library; one byte, 0 = off. Absent
/// = on (S1-17).
pub const PEOPLE_SETTING: &str = "search.people";

/// Read text waiting for a shard, and since when.
type ReadText = (Vec<(PhotoId, String)>, Option<Instant>);
/// Faces found per photo waiting for a shard, and since when.
type FoundFaces = (Vec<(PhotoId, Vec<Face>)>, Option<Instant>);
/// A photo's picture embedding and labels.
type Encoded = (PhotoId, Vec<f32>, Vec<(u16, u8)>);

#[derive(Default)]
struct Buffer {
    model: Option<ModelId>,
    dims: u16,
    /// Labels are made (a `text` shard goes out with the vectors).
    labelled: bool,
    rows: Vec<Encoded>,
    since: Option<Instant>,
}

impl<L: LibraryStore> LibraryNode<L> {
    /// Start indexing with `encoder` while `gate` allows (idempotent).
    pub fn start_indexer(
        &self,
        cfg: IndexerConfig,
        encoder: Arc<dyn ImageEncoder>,
        gate: Arc<dyn IndexGate>,
    ) {
        if self.node().config().role == Role::Viewer {
            info!("a viewer does not index (it cannot publish shards)");
            return;
        }
        if self.inner.indexer.running.swap(true, Ordering::SeqCst) {
            return;
        }
        self.inner.indexer.progress.lock().unwrap().running = true;
        let weak = Arc::downgrade(&self.inner);
        tokio::spawn(index_loop(weak, cfg, encoder, gate));
        let weak = Arc::downgrade(&self.inner);
        let mut changes = self.inner.store.subscribe();
        tokio::spawn(async move {
            while futures::StreamExt::next(&mut changes).await.is_some() {
                let Some(inner) = weak.upgrade() else { break };
                inner.indexer.wake.notify_one();
            }
        });
    }

    /// Find and describe faces with these models (S1-17), as part of indexing, while the
    /// library's people setting is on.
    pub fn set_faces(&self, models: FaceModels) {
        self.set_face_model(models.model);
        *self.inner.indexer.faces.lock().unwrap() = Some(models);
        self.inner.indexer.wake.notify_one();
    }

    /// Whether the library has people (faces) turned on.
    pub async fn people_on(&self) -> Result<bool> {
        Ok(self
            .store()
            .setting(PEOPLE_SETTING)
            .await?
            .is_none_or(|v| v.first() != Some(&0)))
    }

    /// Turn people on or off for the whole library (every device follows).
    pub async fn set_people(&self, on: bool) -> Result<()> {
        self.store()
            .put_setting(PEOPLE_SETTING, vec![u8::from(on)])
            .await?;
        Ok(())
    }

    /// Read the text in likely photos with `engine` (S1-14), as part of indexing.
    pub fn set_ocr(&self, engine: Arc<dyn Ocr>) {
        *self.inner.indexer.ocr.lock().unwrap() = Some(engine);
        self.inner.indexer.wake.notify_one();
    }

    /// Stop indexing on this device. Its status then says 0 photos a minute, so the others take
    /// over its share of the backlog (S1-16).
    pub fn pause_indexer(&self) {
        self.inner.indexer.paused.store(true, Ordering::SeqCst);
        self.inner.indexer.progress.lock().unwrap().paused = true;
        self.inner.indexer.wake.notify_one();
    }

    pub fn resume_indexer(&self) {
        self.inner.indexer.paused.store(false, Ordering::SeqCst);
        self.inner.indexer.progress.lock().unwrap().paused = false;
        self.inner.indexer.wake.notify_one();
    }

    pub fn indexer_paused(&self) -> bool {
        self.inner.indexer.paused.load(Ordering::SeqCst)
    }

    pub fn index_progress(&self) -> IndexProgress {
        self.inner.indexer.progress.lock().unwrap().clone()
    }

    /// Publish whatever results are waiting as shards now (tests, shutdown).
    pub async fn flush_index(&self) -> Result<usize> {
        let (found, face_model) = {
            let mut f = self.inner.indexer.face_rows.lock().unwrap();
            f.1 = None;
            let model = self
                .inner
                .indexer
                .faces
                .lock()
                .unwrap()
                .as_ref()
                .map(|m| m.model);
            (std::mem::take(&mut f.0), model)
        };
        if let (false, Some(model)) = (found.is_empty(), face_model) {
            let ids = found.iter().map(|(id, _)| *id).collect();
            let dims = found
                .iter()
                .flat_map(|(_, f)| f.first())
                .map(|f| f.embedding.v.len() as u16)
                .next()
                .unwrap_or(128);
            let rows = Rows::Faces(found.into_iter().map(|(_, f)| f).collect());
            let shard = Shard::build(model, dims, vec![], rows, ids)?;
            self.publish_shard(&shard).await?;
        }
        let read = {
            let mut o = self.inner.indexer.ocr_rows.lock().unwrap();
            o.1 = None;
            std::mem::take(&mut o.0)
        };
        if !read.is_empty() {
            let ids = read.iter().map(|(id, _)| *id).collect();
            let rows = read
                .into_iter()
                .map(|(_, ocr)| TextRow {
                    labels: vec![],
                    ocr,
                })
                .collect();
            let shard = Shard::build(OCR_ENGINE, OCR_MARK, vec![], Rows::Text(rows), ids)?;
            self.publish_shard(&shard).await?;
        }
        let rows = {
            let mut b = self.inner.indexer.buffer.lock().unwrap();
            b.since = None;
            (b.model, b.dims, b.labelled, std::mem::take(&mut b.rows))
        };
        let (Some(model), dims, labelled, rows) = rows else {
            return Ok(0);
        };
        if rows.is_empty() {
            return Ok(0);
        }
        let n = rows.len();
        let ids: Vec<PhotoId> = rows.iter().map(|(id, _, _)| *id).collect();
        let vec8 = Rows::Vec8(rows.iter().map(|(_, v, _)| Vec8::quantize(v)).collect());
        let vec1 = Rows::Vec1(rows.iter().map(|(_, v, _)| sign_bits(v)).collect());
        let mut out = vec![(vec8, dims), (vec1, dims)];
        if labelled {
            // For text shards `dims` carries the label list's version.
            let text = rows
                .iter()
                .map(|(_, _, l)| TextRow {
                    labels: l.clone(),
                    ocr: String::new(),
                })
                .collect();
            out.push((Rows::Text(text), LABELS_VERSION));
        }
        for (r, d) in out {
            let shard = Shard::build(model, d, vec![], r, ids.clone())?;
            self.publish_shard(&shard).await?;
        }
        self.inner.indexer.progress.lock().unwrap().buffered = 0;
        debug!("published index shards for {n} photos");
        Ok(n)
    }

    /// Photos this device should index now, own imports first, then newest first; and how many
    /// photos already have a result.
    async fn index_work(
        &self,
        model: ModelId,
        cfg: &IndexerConfig,
        since_ms: u64,
    ) -> Result<(Vec<PhotoRecord>, usize, usize)> {
        let photos = self
            .store()
            .photos_by_time(TimeRange::ALL, Page::all())
            .await?;
        let mut done: HashSet<PhotoId> = HashSet::new();
        for (_, s) in self.live_shards_of(model, ShardKind::Vec8).await? {
            done.extend(s.ids.iter().copied());
        }
        done.extend(
            self.inner
                .indexer
                .buffer
                .lock()
                .unwrap()
                .rows
                .iter()
                .map(|(id, _, _)| *id),
        );
        let candidates: Vec<IndexCandidate> = self
            .store()
            .indexer_statuses()
            .await?
            .into_iter()
            .filter(|s| {
                s.always_on && s.models.contains(&model) && s.kinds.contains(&ShardKind::Vec8)
            })
            .map(|s| IndexCandidate {
                device: s.device,
                weight: s.photos_per_min,
            })
            .collect();
        let me = self.device();
        let now = self.inner.clock.now_ms();
        let grace = cfg.grace.as_millis() as u64;
        let total = photos.len();
        let indexed = photos.iter().filter(|p| done.contains(&p.id)).count();
        let mut own = vec![];
        let mut share = vec![];
        for p in photos {
            if done.contains(&p.id) || p.thumb.hash == p.original.hash {
                continue; // indexed, or a video still without its poster frame (A-104)
            }
            let imported_ms = p.favorite.hlc.physical_ms.min(p.deleted_at.hlc.physical_ms);
            let due = now.saturating_sub(imported_ms.max(since_ms)) >= grace;
            if p.imported_by == me {
                own.push(p);
            } else if due || index_owner(&p.id, ShardKind::Vec8, &candidates) == Some(me) {
                share.push(p);
            }
        }
        own.extend(share);
        let mut work = vec![];
        for p in own {
            if self.node().blobs().has(&p.thumb.hash).await? {
                work.push(p);
            }
        }
        Ok((work, indexed, total))
    }

    async fn publish_indexer_status(
        &self,
        model: ModelId,
        cfg: &IndexerConfig,
        rate: u32,
        labelled: bool,
    ) {
        let mut kinds = vec![ShardKind::Vec8, ShardKind::Vec1];
        let mut models = vec![model];
        let reads = self.inner.indexer.ocr.lock().unwrap().is_some();
        if labelled || reads {
            kinds.push(ShardKind::Text);
        }
        if reads {
            models.push(OCR_ENGINE);
        }
        if let Some(f) = self.inner.indexer.faces.lock().unwrap().as_ref() {
            models.push(f.model);
            kinds.push(ShardKind::Faces);
        }
        let status = IndexerStatus {
            device: self.device(),
            models,
            kinds,
            photos_per_min: cfg.rate_override.unwrap_or(rate),
            always_on: cfg.always_on,
            hlc: shoebox_core::hlc::Hlc::GENESIS,
        };
        if let Err(e) = self.store().put_indexer_status(status).await {
            warn!("could not publish the indexer status: {e}");
        }
    }

    /// Photos this device should read text in now (S1-14): labelled as likely to hold text (or
    /// screenshots), not read by anyone yet, and held here as a preview or original. Returns
    /// each photo with the blob to read.
    async fn ocr_work(
        &self,
        cfg: &IndexerConfig,
        since_ms: u64,
    ) -> Result<Vec<(PhotoId, Blake3Hash)>> {
        let view = self.shard_view().await?;
        let labels = self.labels();
        let mut labelled: HashMap<PhotoId, Vec<(u16, u8)>> = HashMap::new();
        let mut read: HashSet<PhotoId> = HashSet::new();
        for h in &view.live {
            let r = &view.records[h];
            if r.kind != ShardKind::Text {
                continue;
            }
            let Some(s) = self.load_shard(r).await? else {
                continue;
            };
            let Rows::Text(rows) = &s.rows else { continue };
            if s.dims == OCR_MARK {
                read.extend(s.ids.iter().copied());
            } else if s.dims == LABELS_VERSION {
                for (id, row) in s.ids.iter().zip(rows) {
                    labelled
                        .entry(*id)
                        .or_default()
                        .extend(row.labels.iter().copied());
                }
            }
        }
        read.extend(
            self.inner
                .indexer
                .ocr_rows
                .lock()
                .unwrap()
                .0
                .iter()
                .map(|(id, _)| *id),
        );
        let candidates: Vec<IndexCandidate> = self
            .store()
            .indexer_statuses()
            .await?
            .into_iter()
            .filter(|s| s.always_on && s.models.contains(&OCR_ENGINE))
            .map(|s| IndexCandidate {
                device: s.device,
                weight: s.photos_per_min,
            })
            .collect();
        let me = self.device();
        let now = self.inner.clock.now_ms();
        let grace = cfg.grace.as_millis() as u64;
        let mut out = vec![];
        for p in self
            .store()
            .photos_by_time(TimeRange::ALL, Page::all())
            .await?
        {
            if read.contains(&p.id) {
                continue;
            }
            let screenshot = shoebox_search::facts::PhotoFacts::of(&p).is_screenshot();
            let photo_labels = labelled.get(&p.id).map(Vec::as_slice).unwrap_or(&[]);
            if !wants_text(&labels, photo_labels, screenshot) {
                continue;
            }
            let imported_ms = p.favorite.hlc.physical_ms.min(p.deleted_at.hlc.physical_ms);
            let due = now.saturating_sub(imported_ms.max(since_ms)) >= grace;
            let mine = p.imported_by == me
                || due
                || index_owner(&p.id, ShardKind::Text, &candidates) == Some(me);
            if !mine {
                continue;
            }
            for blob in [p.preview.hash, p.original.hash] {
                if self.node().blobs().kept(&blob).await? {
                    out.push((p.id, blob));
                    break;
                }
            }
        }
        Ok(out)
    }

    /// Read text in up to `batch` photos; true if there was any to read.
    async fn read_some_text(
        &self,
        cfg: &IndexerConfig,
        since_ms: u64,
        engine: Arc<dyn Ocr>,
    ) -> Result<bool> {
        let work = self.ocr_work(cfg, since_ms).await?;
        if work.is_empty() {
            return Ok(false);
        }
        for (id, blob) in work.into_iter().take(cfg.batch.max(1)) {
            let bytes = self.node().blobs().get(&blob).await?;
            let e = engine.clone();
            // The text is never logged (diagnostics must not hold what is in people's photos).
            match tokio::task::spawn_blocking(move || e.read_text(&bytes)).await? {
                Ok(text) => {
                    let mut o = self.inner.indexer.ocr_rows.lock().unwrap();
                    o.1.get_or_insert_with(Instant::now);
                    o.0.push((id, text));
                }
                Err(e) => debug!("could not read text in {id}: {e}"),
            }
        }
        Ok(true)
    }

    /// Photos this device should look for faces in now (S1-17): not done by anyone yet, held
    /// here as a preview or original. Returns each photo with the blob to read.
    async fn face_work(
        &self,
        model: ModelId,
        cfg: &IndexerConfig,
        since_ms: u64,
    ) -> Result<Vec<(PhotoId, Blake3Hash)>> {
        let mut done: HashSet<PhotoId> = HashSet::new();
        for (_, s) in self.live_shards_of(model, ShardKind::Faces).await? {
            done.extend(s.ids.iter().copied());
        }
        done.extend(
            self.inner
                .indexer
                .face_rows
                .lock()
                .unwrap()
                .0
                .iter()
                .map(|(id, _)| *id),
        );
        let candidates: Vec<IndexCandidate> = self
            .store()
            .indexer_statuses()
            .await?
            .into_iter()
            .filter(|s| s.always_on && s.models.contains(&model))
            .map(|s| IndexCandidate {
                device: s.device,
                weight: s.photos_per_min,
            })
            .collect();
        let me = self.device();
        let now = self.inner.clock.now_ms();
        let grace = cfg.grace.as_millis() as u64;
        let mut out = vec![];
        for p in self
            .store()
            .photos_by_time(TimeRange::ALL, Page::all())
            .await?
        {
            if done.contains(&p.id) || p.original.mime.starts_with("video/") {
                continue;
            }
            let imported_ms = p.favorite.hlc.physical_ms.min(p.deleted_at.hlc.physical_ms);
            let due = now.saturating_sub(imported_ms.max(since_ms)) >= grace;
            let mine = p.imported_by == me
                || due
                || index_owner(&p.id, ShardKind::Faces, &candidates) == Some(me);
            if !mine {
                continue;
            }
            for blob in [p.preview.hash, p.original.hash] {
                if self.node().blobs().kept(&blob).await? {
                    out.push((p.id, blob));
                    break;
                }
            }
        }
        Ok(out)
    }

    /// Find faces in up to `batch` photos; true if there were any to look at.
    async fn find_some_faces(
        &self,
        cfg: &IndexerConfig,
        since_ms: u64,
        models: FaceModels,
    ) -> Result<bool> {
        if !self.people_on().await? {
            return Ok(false);
        }
        let work = self.face_work(models.model, cfg, since_ms).await?;
        if work.is_empty() {
            return Ok(false);
        }
        for (id, blob) in work.into_iter().take(cfg.batch.max(1)) {
            let bytes = self.node().blobs().get(&blob).await?;
            let m = models.clone();
            let found = tokio::task::spawn_blocking(move || describe_faces(&bytes, &m)).await?;
            match found {
                Ok(faces) => {
                    let mut f = self.inner.indexer.face_rows.lock().unwrap();
                    f.1.get_or_insert_with(Instant::now);
                    f.0.push((id, faces));
                }
                Err(e) => debug!("could not look for faces in {id}: {e:#}"),
            }
        }
        Ok(true)
    }

    /// When this device first started indexing (the 48-hour grace counts from it at the earliest,
    /// so a device joining an old library does not take over everyone's share at once).
    fn indexer_since(&self) -> u64 {
        let path = self
            .node()
            .config()
            .data_dir
            .join("search")
            .join("indexer-since");
        if let Some(ms) = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| s.trim().parse().ok())
        {
            return ms;
        }
        let now = self.inner.clock.now_ms();
        let _ = std::fs::create_dir_all(path.parent().expect("parent"));
        let _ = std::fs::write(&path, now.to_string());
        now
    }
}

/// The faces in a photo's preview (or original), as stored in a faces shard.
fn describe_faces(bytes: &[u8], m: &FaceModels) -> Result<Vec<Face>> {
    let img = image::load_from_memory(bytes).context("decoding a preview")?;
    // Previews are at most 2048 px and already upright (`render.rs`); bring originals down too.
    let img = if img.width().max(img.height()) > 2048 {
        img.resize(2048, 2048, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let rgb = img.to_rgb8();
    let (w, h) = (rgb.width() as f32, rgb.height() as f32);
    let image = Rgb {
        width: rgb.width(),
        height: rgb.height(),
        data: rgb.into_raw(),
    };
    let norm = |v: f32, of: f32| ((v / of).clamp(0.0, 1.0) * 65_535.0).round() as u16;
    let mut out = vec![];
    for d in m.detector.detect(&image)? {
        let (bw, bh) = (d.bbox[2] - d.bbox[0], d.bbox[3] - d.bbox[1]);
        if bw.min(bh) < MIN_FACE_PX {
            continue;
        }
        let v = m.embedder.embed(&image, &d.landmarks)?;
        let mut landmarks = [0u16; 10];
        for (i, l) in d.landmarks.iter().enumerate() {
            landmarks[2 * i] = norm(l[0], w);
            landmarks[2 * i + 1] = norm(l[1], h);
        }
        out.push(Face {
            bbox: [
                norm(d.bbox[0], w),
                norm(d.bbox[1], h),
                norm(d.bbox[2], w),
                norm(d.bbox[3], h),
            ],
            landmarks,
            quality: (d.score.clamp(0.0, 1.0) * 255.0).round() as u8,
            embedding: Vec8::quantize(&v),
        });
    }
    Ok(out)
}

fn decode_thumb(bytes: &[u8]) -> Result<Rgb> {
    let img = image::load_from_memory(bytes).context("decoding a thumbnail")?;
    let rgb = img.to_rgb8();
    Ok(Rgb {
        width: rgb.width(),
        height: rgb.height(),
        data: rgb.into_raw(),
    })
}

async fn index_loop<L: LibraryStore>(
    weak: Weak<crate::library_node::Inner<L>>,
    cfg: IndexerConfig,
    encoder: Arc<dyn ImageEncoder>,
    gate: Arc<dyn IndexGate>,
) {
    let model = encoder.model();
    let dims = encoder.dims() as u16;
    let threads = Arc::new(tokio::sync::Semaphore::new(cfg.threads.max(1)));
    let mut announced: Option<(u32, Instant)> = None;
    let mut announced_reads = 0u8;
    let mut announced_paused = false;
    let mut regrouped: Option<Instant> = None;
    // The first round waits a full interval: at start other devices' shards are still arriving.
    let mut compacted: Option<Instant> = Some(Instant::now());
    let mut rate: u32 = cfg.rate_override.unwrap_or(30);
    // Labels only when the list's vectors were made with this encoder's model.
    let (labels, since) = {
        let Some(inner) = weak.upgrade() else { return };
        let ln = LibraryNode { inner };
        let labels: Option<Arc<Labels>> = Some(ln.labels()).filter(|l| l.model() == Some(model));
        (labels, ln.indexer_since())
    };
    loop {
        let Some(inner) = weak.upgrade() else { break };
        let ln = LibraryNode { inner };
        let st = &ln.inner.indexer;
        {
            let mut b = st.buffer.lock().unwrap();
            b.model = Some(model);
            b.dims = dims;
            b.labelled = labels.is_some();
        }
        // Announce on start, when a text reader was added, and when the measured rate moved by
        // a fifth (at most every 10 min).
        let reads =
            st.ocr.lock().unwrap().is_some() as u8 + 2 * st.faces.lock().unwrap().is_some() as u8;
        let paused = ln.indexer_paused();
        let stale = announced_reads != reads
            || announced_paused != paused
            || announced.is_none_or(|(r, at)| {
                at.elapsed() >= cfg.status_every && (r.abs_diff(rate) * 5 > r.max(1))
            });
        if stale {
            let shown = if paused { 0 } else { rate };
            let mut c = cfg.clone();
            if paused {
                c.rate_override = Some(0);
            }
            ln.publish_indexer_status(model, &c, shown, labels.is_some())
                .await;
            announced = Some((rate, Instant::now()));
            announced_reads = reads;
            announced_paused = paused;
        }
        let old = st
            .buffer
            .lock()
            .unwrap()
            .since
            .is_some_and(|t| t.elapsed() >= cfg.shard_after)
            || st
                .ocr_rows
                .lock()
                .unwrap()
                .1
                .is_some_and(|t| t.elapsed() >= cfg.shard_after)
            || st
                .face_rows
                .lock()
                .unwrap()
                .1
                .is_some_and(|t| t.elapsed() >= cfg.shard_after);
        if old {
            if let Err(e) = ln.flush_index().await {
                warn!("could not publish index shards: {e:#}");
            }
        }
        let open = gate.may_index();
        st.progress.lock().unwrap().waiting = (!open).then(|| "waiting for power and idle".into());
        if !ln.indexer_paused() && open {
            match ln.index_work(model, &cfg, since).await {
                Ok((work, indexed, total)) => {
                    {
                        let mut p = st.progress.lock().unwrap();
                        p.model = Some(model);
                        p.indexed = indexed;
                        p.total = total;
                        p.mine_left = work.len();
                        p.photos_per_min = rate;
                    }
                    for chunk in work.chunks(cfg.batch.max(1)) {
                        if ln.indexer_paused() || !gate.may_index() {
                            break;
                        }
                        let started = Instant::now();
                        let mut jobs = vec![];
                        for p in chunk {
                            let bytes = match ln.node().blobs().get(&p.thumb.hash).await {
                                Ok(b) => b,
                                Err(_) => continue,
                            };
                            let enc = encoder.clone();
                            let labels = labels.clone();
                            let permit = threads.clone().acquire_owned().await.expect("open");
                            let id = p.id;
                            jobs.push(tokio::task::spawn_blocking(move || {
                                let _permit = permit;
                                let v = decode_thumb(&bytes)
                                    .and_then(|img| enc.encode_image(&img).map_err(Into::into))
                                    .map(|v| {
                                        let l = labels
                                            .map(|l| l.assign(&v, MIN_PROBABILITY))
                                            .unwrap_or_default();
                                        (v, l)
                                    });
                                (id, v)
                            }));
                        }
                        let mut done = 0u32;
                        for j in jobs {
                            match j.await {
                                Ok((id, Ok((v, l)))) => {
                                    let mut b = st.buffer.lock().unwrap();
                                    b.since.get_or_insert_with(Instant::now);
                                    b.rows.push((id, v, l));
                                    done += 1;
                                }
                                Ok((id, Err(e))) => debug!("could not index {id}: {e:#}"),
                                Err(e) => warn!("an encoder task failed: {e}"),
                            }
                        }
                        let secs = started.elapsed().as_secs_f64().max(1e-3);
                        if done > 0 {
                            let measured = (f64::from(done) * 60.0 / secs).round() as u32;
                            rate = ((rate * 3 + measured.max(1)) / 4).max(1);
                        }
                        let full = {
                            let b = st.buffer.lock().unwrap();
                            let mut p = st.progress.lock().unwrap();
                            p.mine_left = p.mine_left.saturating_sub(chunk.len());
                            p.indexed += done as usize;
                            p.buffered = b.rows.len();
                            p.photos_per_min = cfg.rate_override.unwrap_or(rate);
                            b.rows.len() >= cfg.shard_size
                        };
                        if full {
                            if let Err(e) = ln.flush_index().await {
                                warn!("could not publish index shards: {e:#}");
                            }
                            st.progress.lock().unwrap().buffered = 0;
                        }
                    }
                }
                Err(e) => warn!("could not plan indexing: {e:#}"),
            }
            let engine = st.ocr.lock().unwrap().clone();
            if let Some(engine) = engine {
                if !ln.indexer_paused() && gate.may_index() {
                    match ln.read_some_text(&cfg, since, engine).await {
                        Ok(true) => st.wake.notify_one(),
                        Ok(false) => {}
                        Err(e) => warn!("could not read text in photos: {e:#}"),
                    }
                    let full = st.ocr_rows.lock().unwrap().0.len() >= cfg.shard_size;
                    if full {
                        if let Err(e) = ln.flush_index().await {
                            warn!("could not publish index shards: {e:#}");
                        }
                    }
                }
            }
            let faces = st.faces.lock().unwrap().clone();
            if let Some(faces) = faces {
                if cfg.always_on && regrouped.is_none_or(|t| t.elapsed() >= REGROUP_EVERY) {
                    regrouped = Some(Instant::now());
                    if let Err(e) = ln.regroup_people(faces.model, false).await {
                        warn!("could not group faces: {e:#}");
                    }
                }
                if !ln.indexer_paused() && gate.may_index() {
                    match ln.find_some_faces(&cfg, since, faces).await {
                        Ok(true) => st.wake.notify_one(),
                        Ok(false) => {}
                        Err(e) => warn!("could not look for faces: {e:#}"),
                    }
                    let full = st.face_rows.lock().unwrap().0.len() >= cfg.shard_size;
                    if full {
                        if let Err(e) = ln.flush_index().await {
                            warn!("could not publish index shards: {e:#}");
                        }
                    }
                }
            }
        }
        if cfg.always_on && compacted.is_none_or(|t| t.elapsed() >= cfg.compact_every) {
            compacted = Some(Instant::now());
            if let Err(e) = ln.compact_shards(false).await {
                warn!("could not compact index shards: {e:#}");
            }
        }
        let wait = {
            let inner = ln.inner.clone();
            let tick = cfg.tick;
            async move {
                let _ = tokio::time::timeout(tick, inner.indexer.wake.notified()).await;
            }
        };
        drop(ln);
        wait.await;
    }
}

/// For tests and tools: the devices that would index `photo` now.
pub fn owners_of(
    photo: &PhotoId,
    statuses: &[IndexerStatus],
    model: ModelId,
) -> Option<EndpointId> {
    let c: Vec<IndexCandidate> = statuses
        .iter()
        .filter(|s| s.always_on && s.models.contains(&model))
        .map(|s| IndexCandidate {
            device: s.device,
            weight: s.photos_per_min,
        })
        .collect();
    index_owner(photo, ShardKind::Vec8, &c)
}
