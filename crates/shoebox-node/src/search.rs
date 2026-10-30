//! Search on this device (S1-03): the facts index, kept current from the library's change
//! stream and persisted to `<data dir>/search/facts.idx`. Everything here works with no other
//! device online; the folder is a derived cache that is rebuilt when missing or of another
//! version.

use crate::library_node::LibraryNode;
use anyhow::Result;
use shoebox_core::ids::Timestamp;
use shoebox_core::model::{AlbumId, PhotoId};
use shoebox_core::model::{ModelId, ShardKind};
use shoebox_core::store::{Change, LibraryStore, Page, TimeRange};
use shoebox_search::encoder::TextEncoder;
use shoebox_search::facts::{self, FactsIndex, Hits, PhotoFacts};
use shoebox_search::query::{parse, NameKind, ParseContext, Query, Vocabulary};
use shoebox_search::shard::Rows;
use shoebox_search::vectors::{cut, fuse, RowIn, VectorIndex};
use shoebox_search::Zone;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tracing::{debug, warn};

/// How often a changed index is written to disk at most.
const PERSIST_EVERY: Duration = Duration::from_secs(5);
/// Meaning results within this much of the best score are shown (tuned on the eval set, S1-12:
/// the captioned photo survives 97% of the time and about 1% of the photos are shown).
pub const MEANING_MARGIN: f32 = 0.08;
/// At most this many results by meaning.
pub const MEANING_MAX: usize = 2_000;
/// The text model is let go after this long without a search by meaning (about 0.5 GB).
const TEXT_IDLE: Duration = Duration::from_secs(120);
/// Query vectors remembered.
const QUERY_CACHE: usize = 256;

/// Loads the text encoder on first use (S1-08: from the build's model files).
pub type TextLoader = Arc<dyn Fn() -> Result<Arc<dyn TextEncoder>> + Send + Sync>;

#[derive(Default)]
struct Meaning {
    loader: Option<TextLoader>,
    encoder: Option<Arc<dyn TextEncoder>>,
    last_used: Option<Instant>,
    failed: Option<String>,
    cache: HashMap<String, Arc<Vec<f32>>>,
    order: VecDeque<String>,
}

#[derive(Default)]
pub(crate) struct SearchState {
    index: Mutex<Option<FactsIndex>>,
    vocab: Mutex<Option<Arc<Vocabulary>>>,
    /// The index changed since it was last written.
    dirty: std::sync::atomic::AtomicBool,
    running: std::sync::atomic::AtomicBool,
    /// Held while the index is built, so two first searches do not build it twice.
    build: tokio::sync::Mutex<()>,
    meaning: Mutex<Meaning>,
    vectors: Mutex<Option<Arc<VectorIndex>>>,
    /// Shards changed since the vector file was built.
    vectors_stale: std::sync::atomic::AtomicBool,
    vectors_build: tokio::sync::Mutex<()>,
    /// Text shards changed since their labels and text went into the facts index.
    text_stale: std::sync::atomic::AtomicBool,
    /// Photos whose labels and text came from a shard (to clear them when the shard goes).
    text_applied: Mutex<HashSet<PhotoId>>,
    labels: Mutex<Option<Arc<shoebox_search::labels::Labels>>>,
    /// People, labels or face shards changed since names went into the facts index.
    people_stale: std::sync::atomic::AtomicBool,
    /// Photos whose people's names are in the facts index.
    people_applied: Mutex<HashSet<PhotoId>>,
}

/// What "now" and "here" mean for a query (from the device that asked).
#[derive(Clone, Copy, Debug)]
pub struct SearchOptions {
    pub now: Timestamp,
    /// Local time zone as minutes east of UTC.
    pub tz_offset_min: i32,
    /// The device is in the southern hemisphere ("last summer").
    pub southern: bool,
}

impl SearchOptions {
    pub fn utc(now: Timestamp) -> Self {
        Self {
            now,
            tz_offset_min: 0,
            southern: false,
        }
    }
}

/// A page of results with what the query was understood as.
#[derive(Clone, Debug)]
pub struct SearchResult {
    pub query: Query,
    pub hits: Hits,
}

impl<L: LibraryStore> LibraryNode<L> {
    fn search_dir(&self) -> PathBuf {
        self.node().config().data_dir.join("search")
    }

    /// Build (or load) the search index now and keep it current from the change stream.
    /// Idempotent. Searching works without it (the index is then built on first use), but it
    /// is only kept current and saved while this runs.
    pub fn start_search(&self) {
        if self.inner.search.running.swap(true, Ordering::SeqCst) {
            return;
        }
        let mut changes = self.store().subscribe();
        let weak = Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            {
                let Some(inner) = weak.upgrade() else { return };
                let ln = LibraryNode { inner };
                if let Err(e) = ln.ensure_index().await {
                    warn!("building the search index failed: {e:#}");
                }
            }
            let mut last_write = Instant::now();
            loop {
                let next =
                    tokio::time::timeout(PERSIST_EVERY, futures::StreamExt::next(&mut changes))
                        .await;
                let Some(inner) = weak.upgrade() else { break };
                let ln = LibraryNode { inner };
                match next {
                    Ok(None) => break,
                    // A new shard record, or this device finished fetching a shard's blob (its
                    // own holdings changed): what counts may have changed.
                    Ok(Some(Change::IndexChanged)) => {
                        ln.inner.search.vectors_stale.store(true, Ordering::SeqCst);
                        ln.inner.search.text_stale.store(true, Ordering::SeqCst);
                        ln.inner.search.people_stale.store(true, Ordering::SeqCst);
                    }
                    Ok(Some(Change::PeopleChanged)) => {
                        ln.inner.search.people_stale.store(true, Ordering::SeqCst);
                    }
                    Ok(Some(Change::SettingChanged(key)))
                        if key.starts_with("people.") || key.starts_with("search.") =>
                    {
                        ln.inner.search.people_stale.store(true, Ordering::SeqCst);
                    }
                    Ok(Some(Change::HoldingsChanged { device })) if device == ln.device() => {
                        ln.inner.search.vectors_stale.store(true, Ordering::SeqCst);
                        ln.inner.search.text_stale.store(true, Ordering::SeqCst);
                        ln.inner.search.people_stale.store(true, Ordering::SeqCst);
                    }
                    Ok(Some(change)) => {
                        if let Err(e) = ln.apply_search_change(&change).await {
                            debug!("search index update failed: {e:#}");
                        }
                    }
                    Err(_) => {}
                }
                ln.release_idle_text_encoder();
                if ln.inner.search.dirty.load(Ordering::SeqCst)
                    && last_write.elapsed() >= PERSIST_EVERY
                {
                    ln.persist_search_index();
                    last_write = Instant::now();
                }
            }
        });
    }

    /// Load `facts.idx` if it is usable, then bring it up to date with the library.
    async fn ensure_index(&self) -> Result<()> {
        let _building = self.inner.search.build.lock().await;
        if self.inner.search.index.lock().unwrap().is_some() {
            return Ok(());
        }
        let path = self.search_dir().join("facts.idx");
        let mut ix = std::fs::read(&path)
            .ok()
            .and_then(|b| {
                let s = facts::decode(&b);
                if s.is_none() {
                    warn!(
                        "{} is unreadable or of another version; rebuilding",
                        path.display()
                    );
                }
                s
            })
            .map(FactsIndex::from_snapshot)
            .unwrap_or_default();
        let changed = self.reconcile_index(&mut ix).await?;
        *self.inner.search.index.lock().unwrap() = Some(ix);
        self.inner.search.text_stale.store(true, Ordering::SeqCst);
        self.inner.search.people_stale.store(true, Ordering::SeqCst);
        *self.inner.search.vocab.lock().unwrap() = None;
        if changed || !path.exists() {
            self.inner.search.dirty.store(true, Ordering::SeqCst);
            self.persist_search_index();
        }
        Ok(())
    }

    /// Make `ix` match the library: every photo (deleted ones too, so a restore is instant),
    /// every album, nothing that was purged. Returns whether anything changed.
    async fn reconcile_index(&self, ix: &mut FactsIndex) -> Result<bool> {
        let mut changed = false;
        let live = self
            .store()
            .photos_by_time(TimeRange::ALL, Page::all())
            .await?;
        let deleted = self.store().deleted_photos().await?;
        let mut present: HashSet<PhotoId> = HashSet::with_capacity(live.len() + deleted.len());
        for rec in live.iter().chain(deleted.iter()) {
            present.insert(rec.id);
            changed |= self.index_photo(ix, PhotoFacts::of(rec));
        }
        let gone: Vec<PhotoId> = ix
            .ids()
            .filter(|id| !present.contains(id))
            .copied()
            .collect();
        for id in &gone {
            changed |= ix.remove(id);
        }
        let albums = self.store().albums().await?;
        let known: HashSet<AlbumId> = albums.iter().map(|a| a.id).collect();
        let stale: Vec<AlbumId> = ix
            .album_ids()
            .filter(|a| !known.contains(a))
            .copied()
            .collect();
        for a in stale {
            ix.remove_album(&a);
            changed = true;
        }
        for a in albums {
            let items = self.store().album_items(&a.id, Page::all()).await?;
            ix.set_album(a.id, &a.name, items);
        }
        Ok(changed)
    }

    /// Add or update one photo, with what is derived from it on this device (place names).
    fn index_photo(&self, ix: &mut FactsIndex, facts: PhotoFacts) -> bool {
        let id = facts.id;
        let place = facts.gps.and_then(crate::places::place_text);
        let had_place = ix
            .extra(&id)
            .is_some_and(|e| e.iter().any(|(f, _)| *f == facts::Field::Place));
        let changed = ix.upsert(facts);
        if place.is_some() || had_place {
            ix.set_extra(&id, facts::Field::Place, place);
        }
        changed
    }

    async fn apply_search_change(&self, change: &Change) -> Result<()> {
        if self.inner.search.index.lock().unwrap().is_none() {
            return Ok(());
        }
        match change {
            Change::PhotoUpserted(id) | Change::PhotoDeleted(id) => {
                let rec = self.store().photo(id).await?;
                let mut guard = self.inner.search.index.lock().unwrap();
                let Some(ix) = guard.as_mut() else {
                    return Ok(());
                };
                match rec {
                    Some(rec) => {
                        self.index_photo(ix, PhotoFacts::of(&rec));
                    }
                    None => {
                        ix.remove(id);
                    }
                }
            }
            Change::AlbumChanged(album) => {
                let summary = self
                    .store()
                    .albums()
                    .await?
                    .into_iter()
                    .find(|a| a.id == *album);
                let items = match &summary {
                    Some(_) => self.store().album_items(album, Page::all()).await?,
                    None => vec![],
                };
                let mut guard = self.inner.search.index.lock().unwrap();
                let Some(ix) = guard.as_mut() else {
                    return Ok(());
                };
                match summary {
                    Some(a) => ix.set_album(a.id, &a.name, items),
                    None => ix.remove_album(album),
                }
            }
            _ => return Ok(()),
        }
        *self.inner.search.vocab.lock().unwrap() = None;
        self.inner.search.dirty.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// Write `facts.idx` (atomically: a temporary file, then a rename).
    fn persist_search_index(&self) {
        let bytes = {
            let guard = self.inner.search.index.lock().unwrap();
            let Some(ix) = guard.as_ref() else { return };
            facts::encode(&ix.snapshot())
        };
        let dir = self.search_dir();
        let res = std::fs::create_dir_all(&dir).and_then(|_| {
            let tmp = dir.join("facts.idx.tmp");
            std::fs::write(&tmp, &bytes)?;
            std::fs::rename(&tmp, dir.join("facts.idx"))
        });
        match res {
            Ok(()) => self.inner.search.dirty.store(false, Ordering::SeqCst),
            Err(e) => warn!("writing the search index failed: {e}"),
        }
    }

    /// Put the labels and text of live text shards into the facts index (S1-13, S1-14): one
    /// row per photo, from the shard with the lowest hash, whatever model made it.
    async fn apply_text_shards(&self) -> Result<()> {
        if !self.inner.search.text_stale.swap(false, Ordering::SeqCst) {
            return Ok(());
        }
        let view = self.shard_view().await?;
        // Label shards and read-text shards are kept apart: each photo takes its labels from
        // one and its text from the other (lowest blob hash wins within each).
        let (mut label_shards, mut ocr_shards) = (vec![], vec![]);
        for h in &view.live {
            let r = &view.records[h];
            if r.kind == ShardKind::Text {
                if let Some(s) = self.load_shard(r).await? {
                    if s.dims == shoebox_search::ocr::OCR_MARK {
                        ocr_shards.push((*h, s));
                    } else {
                        label_shards.push((*h, s));
                    }
                }
            }
        }
        let labels = self.labels();
        let mut texts: HashMap<PhotoId, (String, String)> = HashMap::new();
        for (id, (i, r)) in crate::shards::winning_rows(&label_shards) {
            let shard = &label_shards[i].1;
            let Rows::Text(t) = &shard.rows else { continue };
            // Label numbers mean something only in the list version they were made with.
            if shard.dims == shoebox_search::labels::LABELS_VERSION {
                if let Some(row) = t.get(r) {
                    texts.entry(id).or_default().0 = labels.words(&row.labels);
                }
            }
        }
        for (id, (i, r)) in crate::shards::winning_rows(&ocr_shards) {
            let Rows::Text(t) = &ocr_shards[i].1.rows else {
                continue;
            };
            if let Some(row) = t.get(r) {
                texts.entry(id).or_default().1 = row.ocr.clone();
            }
        }
        let texts: Vec<(PhotoId, String, String)> =
            texts.into_iter().map(|(id, (w, o))| (id, w, o)).collect();
        let mut guard = self.inner.search.index.lock().unwrap();
        let Some(ix) = guard.as_mut() else {
            self.inner.search.text_stale.store(true, Ordering::SeqCst);
            return Ok(());
        };
        let mut applied = self.inner.search.text_applied.lock().unwrap();
        let now: HashSet<PhotoId> = texts.iter().map(|(id, _, _)| *id).collect();
        for gone in applied.difference(&now) {
            ix.set_extra(gone, facts::Field::Label, None);
            ix.set_extra(gone, facts::Field::Ocr, None);
        }
        for (id, words, ocr) in texts {
            ix.set_extra(
                &id,
                facts::Field::Label,
                (!words.is_empty()).then_some(words),
            );
            ix.set_extra(&id, facts::Field::Ocr, (!ocr.is_empty()).then_some(ocr));
        }
        *applied = now;
        drop(applied);
        drop(guard);
        *self.inner.search.vocab.lock().unwrap() = None;
        self.inner.search.dirty.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// Put people's names into the facts index (S1-18), so "Ana" and "me" are found.
    async fn apply_people(&self) -> Result<()> {
        if !self.inner.search.people_stale.swap(false, Ordering::SeqCst) {
            return Ok(());
        }
        let names = self.people_names(self.face_model()).await?;
        let mut guard = self.inner.search.index.lock().unwrap();
        let Some(ix) = guard.as_mut() else {
            self.inner.search.people_stale.store(true, Ordering::SeqCst);
            return Ok(());
        };
        let mut applied = self.inner.search.people_applied.lock().unwrap();
        let now: HashSet<PhotoId> = names.iter().map(|(p, _)| *p).collect();
        for gone in applied.difference(&now) {
            ix.set_extra(gone, facts::Field::Person, None);
        }
        for (photo, n) in names {
            ix.set_extra(&photo, facts::Field::Person, Some(n));
        }
        *applied = now;
        drop(applied);
        drop(guard);
        *self.inner.search.vocab.lock().unwrap() = None;
        self.inner.search.dirty.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// Run `f` on the index, building it first if needed.
    pub(crate) async fn with_index<T>(&self, f: impl FnOnce(&FactsIndex) -> T) -> Result<T> {
        self.ensure_index().await?;
        if let Err(e) = self.apply_text_shards().await {
            debug!("could not apply labels and text: {e:#}");
        }
        if let Err(e) = self.apply_people().await {
            debug!("could not apply people: {e:#}");
        }
        let guard = self.inner.search.index.lock().unwrap();
        Ok(f(guard.as_ref().expect("index was just built")))
    }

    async fn vocabulary(&self) -> Result<Arc<Vocabulary>> {
        if let Some(v) = self.inner.search.vocab.lock().unwrap().clone() {
            return Ok(v);
        }
        let v = Arc::new(self.with_index(|ix| ix.vocabulary()).await?);
        *self.inner.search.vocab.lock().unwrap() = Some(v.clone());
        Ok(v)
    }

    /// Search this device's index. `offset` continues a previous page (its `hits.next`).
    pub async fn search(
        &self,
        text: &str,
        opts: &SearchOptions,
        offset: usize,
        limit: usize,
    ) -> Result<SearchResult> {
        let vocab = self.vocabulary().await?;
        let zone = Zone {
            offset_min: opts.tz_offset_min,
        };
        let cx = ParseContext {
            now: opts.now,
            zone,
            southern: opts.southern,
        };
        let query = parse(text, &vocab, &cx);
        let meaning = if query.free_text.is_empty() {
            None
        } else {
            self.query_vector(&query.free_text).await
        };
        let Some((model, qv)) = meaning else {
            let hits = self
                .with_index(|ix| ix.search(&query, zone, opts.southern, offset, limit))
                .await?;
            return Ok(SearchResult { query, hits });
        };
        let (allowed, keyword) = self
            .with_index(|ix| {
                let rows = ix.filter(&query, zone, opts.southern);
                let kw: Vec<_> = ix
                    .keyword(&rows, &query.free_text)
                    .into_iter()
                    .map(|(id, _)| id)
                    .collect();
                (ix.ids_of(&rows), kw)
            })
            .await?;
        let ids = match self.vector_index(model).await? {
            Some(vx) => {
                let allowed: HashSet<PhotoId> = allowed.into_iter().collect();
                let scored =
                    tokio::task::spawn_blocking(move || vx.score(&qv, Some(&allowed))).await??;
                let meaning = cut(&scored, MEANING_MARGIN, MEANING_MAX);
                fuse(&[&keyword, &meaning])
            }
            None => keyword,
        };
        let hits = facts::page(ids, true, offset, limit);
        Ok(SearchResult { query, hits })
    }

    /// Photos that look like `photo` (its own vector as the query), best first, itself left out.
    /// Empty when this device has no vector for it.
    pub async fn similar(&self, photo: &PhotoId, offset: usize, limit: usize) -> Result<Hits> {
        let model = match self.text_model() {
            Some(m) => m,
            None => match self.vector_models().await?.first() {
                Some(m) => *m,
                None => return Ok(Hits::default()),
            },
        };
        let Some(vx) = self.vector_index(model).await? else {
            return Ok(Hits::default());
        };
        let Some(v) = vx.vector(photo)? else {
            return Ok(Hits::default());
        };
        let allowed: HashSet<PhotoId> = self
            .with_index(|ix| {
                let rows = ix.filter(&Query::default(), Zone::UTC, false);
                ix.ids_of(&rows)
            })
            .await?
            .into_iter()
            .filter(|id| id != photo)
            .collect();
        let scored = tokio::task::spawn_blocking(move || vx.score(&v, Some(&allowed))).await??;
        Ok(facts::page(
            cut(&scored, MEANING_MARGIN * 2.0, MEANING_MAX),
            true,
            offset,
            limit,
        ))
    }

    /// The label list this device gives photos and reads text shards with (default: this
    /// build's, `Labels::shared`). Tests use their own.
    pub fn set_labels(&self, labels: Arc<shoebox_search::labels::Labels>) {
        *self.inner.search.labels.lock().unwrap() = Some(labels);
        self.inner.search.text_stale.store(true, Ordering::SeqCst);
    }

    pub fn labels(&self) -> Arc<shoebox_search::labels::Labels> {
        self.inner
            .search
            .labels
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(shoebox_search::labels::Labels::shared)
    }

    /// Where the text encoder comes from (the app and keeper pass the bundled model; tests a
    /// stand-in). Without one, search ranks by keywords only.
    pub fn set_text_encoder(&self, loader: TextLoader) {
        let mut m = self.inner.search.meaning.lock().unwrap();
        m.loader = Some(loader);
        m.encoder = None;
        m.failed = None;
        m.cache.clear();
        m.order.clear();
    }

    /// A text model is set up on this device (it may not have loaded yet).
    pub(crate) fn meaning_configured(&self) -> bool {
        let m = self.inner.search.meaning.lock().unwrap();
        m.loader.is_some() && m.failed.is_none()
    }

    fn text_model(&self) -> Option<ModelId> {
        self.inner
            .search
            .meaning
            .lock()
            .unwrap()
            .encoder
            .as_ref()
            .map(|e| e.model())
    }

    /// Whether this device can search by meaning: a text encoder and vectors of its model.
    pub async fn meaning_available(&self) -> bool {
        let has_loader = self.inner.search.meaning.lock().unwrap().loader.is_some();
        has_loader && !self.vector_models().await.unwrap_or_default().is_empty()
    }

    /// Models with live picture-vector shards in this library.
    async fn vector_models(&self) -> Result<Vec<ModelId>> {
        let view = self.shard_view().await?;
        let mut models: Vec<ModelId> = view
            .live
            .iter()
            .filter_map(|h| view.records.get(h))
            .filter(|r| r.kind == ShardKind::Vec8 || r.kind == ShardKind::Vec1)
            .map(|r| r.model)
            .collect();
        models.sort();
        models.dedup();
        Ok(models)
    }

    /// The query's vector (cached), loading the text encoder if needed. `None` when this device
    /// cannot search by meaning (no encoder, or it failed to load).
    async fn query_vector(&self, text: &str) -> Option<(ModelId, Arc<Vec<f32>>)> {
        let key = shoebox_search::text::fold(text.trim());
        let encoder = {
            let mut m = self.inner.search.meaning.lock().unwrap();
            m.last_used = Some(Instant::now());
            if let (Some(e), Some(v)) = (m.encoder.as_ref(), m.cache.get(&key)) {
                return Some((e.model(), v.clone()));
            }
            if m.failed.is_some() {
                return None;
            }
            match (&m.encoder, &m.loader) {
                (Some(e), _) => Ok(e.clone()),
                (None, Some(l)) => Err(l.clone()),
                (None, None) => return None,
            }
        };
        let encoder = match encoder {
            Ok(e) => e,
            Err(loader) => match tokio::task::spawn_blocking(move || loader()).await {
                Ok(Ok(e)) => {
                    self.inner.search.meaning.lock().unwrap().encoder = Some(e.clone());
                    e
                }
                Ok(Err(e)) => {
                    warn!("search by meaning is off on this device: {e:#}");
                    self.inner.search.meaning.lock().unwrap().failed = Some(format!("{e:#}"));
                    return None;
                }
                Err(_) => return None,
            },
        };
        let enc = encoder.clone();
        let q = key.clone();
        let v = match tokio::task::spawn_blocking(move || enc.encode_text(&q)).await {
            Ok(Ok(v)) => Arc::new(v),
            _ => return None,
        };
        let mut m = self.inner.search.meaning.lock().unwrap();
        if m.cache.insert(key.clone(), v.clone()).is_none() {
            m.order.push_back(key);
            while m.order.len() > QUERY_CACHE {
                if let Some(old) = m.order.pop_front() {
                    m.cache.remove(&old);
                }
            }
        }
        Some((encoder.model(), v))
    }

    fn release_idle_text_encoder(&self) {
        let mut m = self.inner.search.meaning.lock().unwrap();
        if m.loader.is_some()
            && m.encoder.is_some()
            && m.last_used.is_some_and(|t| t.elapsed() >= TEXT_IDLE)
        {
            m.encoder = None;
            debug!("released the idle text encoder");
        }
    }

    /// The vectors of `model` on this device, rebuilt from live shards when they changed.
    async fn vector_index(&self, model: ModelId) -> Result<Option<Arc<VectorIndex>>> {
        let current = self.inner.search.vectors.lock().unwrap().clone();
        let stale = self.inner.search.vectors_stale.load(Ordering::SeqCst);
        if let Some(v) = &current {
            if v.model == model && !stale {
                return Ok(Some(v.clone()));
            }
        }
        let _building = self.inner.search.vectors_build.lock().await;
        let path = self
            .search_dir()
            .join(format!("vec-{}.bin", hex::encode(&model.0[..8])));
        if current.is_none() && !stale {
            if let Some(v) = VectorIndex::open(&path).filter(|v| v.model == model) {
                let v = Arc::new(v);
                *self.inner.search.vectors.lock().unwrap() = Some(v.clone());
                return Ok(Some(v));
            }
        }
        self.inner
            .search
            .vectors_stale
            .store(false, Ordering::SeqCst);
        let vec8 = self.live_shards_of(model, ShardKind::Vec8).await?;
        let vec1 = self.live_shards_of(model, ShardKind::Vec1).await?;
        if vec8.is_empty() && vec1.is_empty() {
            *self.inner.search.vectors.lock().unwrap() = None;
            return Ok(None);
        }
        let dims = vec8
            .iter()
            .chain(&vec1)
            .map(|(_, s)| usize::from(s.dims))
            .next()
            .unwrap_or(0);
        let v = tokio::task::spawn_blocking(move || {
            let w8 = crate::shards::winning_rows(&vec8);
            let w1 = crate::shards::winning_rows(&vec1);
            let mut ids: Vec<&PhotoId> = w8.keys().chain(w1.keys()).collect();
            ids.sort();
            ids.dedup();
            let rows = ids.into_iter().map(|id| {
                let v8 = w8.get(id).and_then(|(i, r)| match &vec8[*i].1.rows {
                    Rows::Vec8(rows) => rows.get(*r),
                    _ => None,
                });
                let v1 = w1.get(id).and_then(|(i, r)| match &vec1[*i].1.rows {
                    Rows::Vec1(rows) => rows.get(*r).map(Vec::as_slice),
                    _ => None,
                });
                RowIn {
                    id: *id,
                    vec8: v8,
                    vec1: v1,
                }
            });
            VectorIndex::build(&path, model, dims, rows)
        })
        .await??;
        let v = Arc::new(v);
        *self.inner.search.vectors.lock().unwrap() = Some(v.clone());
        Ok(Some(v))
    }

    /// Names in the library starting with `prefix`, for suggestions while typing.
    pub async fn search_suggestions(
        &self,
        prefix: &str,
        limit: usize,
    ) -> Result<Vec<(NameKind, String)>> {
        let vocab = self.vocabulary().await?;
        Ok(vocab
            .suggest(prefix, limit)
            .into_iter()
            .map(|e| (e.kind, e.name.clone()))
            .collect())
    }

    /// Every name the parser recognises in this library.
    pub async fn search_vocabulary(&self) -> Result<Vec<(NameKind, String)>> {
        let vocab = self.vocabulary().await?;
        Ok(vocab
            .entries()
            .iter()
            .map(|e| (e.kind, e.name.clone()))
            .collect())
    }

    /// Forget the index and build it again from the library ("Rebuild the local index").
    pub async fn rebuild_search_index(&self) -> Result<()> {
        *self.inner.search.index.lock().unwrap() = None;
        *self.inner.search.vectors.lock().unwrap() = None;
        self.inner
            .search
            .vectors_stale
            .store(true, Ordering::SeqCst);
        let _ = std::fs::remove_file(self.search_dir().join("facts.idx"));
        self.ensure_index().await
    }
}
