//! Search by meaning in the app (S1-09…S1-12): the text model for every search; indexing on a
//! desktop on mains power while idle, and on a phone in a background job while charging, with
//! the image model the phone fetches from a desktop or keeper that offers it.

use crate::core::AppCore;
use anyhow::Result;
use shoebox_core::store::LibraryStore;
use shoebox_ml::files::{locate, search_dirs};
use shoebox_ml::{ClipImage, ClipText, PpOcr, SFace, YuNet};
use shoebox_node::{FaceModels, IndexGate, IndexerConfig, LibraryNode, ModelSources};
use shoebox_search::encoder::{ImageEncoder, TextEncoder};
use shoebox_search::models::{
    ModelFile, ALL, CLIP_IMAGE, CLIP_MODEL, FACES_MODEL, FACE_DETECT, FACE_EMBED, OCR_DET,
    OCR_DICT, OCR_REC,
};
use shoebox_store::Library;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// This device's search settings (S1-16), in `search.json` in the data folder.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SearchSettings {
    /// Desktop: index on mains power while idle. Phone: index while charging.
    pub index: bool,
}

impl Default for SearchSettings {
    fn default() -> Self {
        Self { index: true }
    }
}

impl SearchSettings {
    pub fn load(dir: &std::path::Path) -> Self {
        std::fs::read(dir.join("search.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, dir: &std::path::Path) -> Result<()> {
        std::fs::write(dir.join("search.json"), serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }
}

/// Where this build keeps model files: its resources, then the data folder.
pub fn model_dirs(extra: &[PathBuf], data_dir: &std::path::Path) -> Vec<PathBuf> {
    let mut dirs = extra.to_vec();
    dirs.push(data_dir.join("models"));
    search_dirs(&dirs)
}

/// Set while an indexing run (the background job's, or the user's "Index now") is going in this
/// process. WorkManager can start the job again while the native run of the first is still
/// going, since native code cannot be cancelled; a second run would delete the first one's
/// `index-stop` file and pause its indexer under it.
static INDEX_RUNNING: AtomicBool = AtomicBool::new(false);

/// How the last "Index now" run ended, for Settings.
static LAST_RUN: Mutex<Option<String>> = Mutex::new(None);

pub struct IndexRun;

impl IndexRun {
    pub fn begin() -> Option<Self> {
        (!INDEX_RUNNING.swap(true, Ordering::SeqCst)).then_some(Self)
    }
}

impl Drop for IndexRun {
    fn drop(&mut self) {
        INDEX_RUNNING.store(false, Ordering::SeqCst);
    }
}

/// The longest an "Index now" run goes on; it ends sooner when everything is done.
const MANUAL_BUDGET: Duration = Duration::from_secs(12 * 3600);

impl AppCore {
    /// Delete the model files the switched-off features would use, from this app's own `models/`
    /// folder (files that ship with the app stay): the image, face and text-reading models when
    /// this device does not index, the face models when People is off. The text model for
    /// searching is always kept. They are fetched again when the feature is turned back on.
    pub(crate) async fn remove_unused_models(&self, ln: &LibraryNode<Library>) {
        let indexing = SearchSettings::load(self.data_dir()).index;
        let people = ln.people_on().await.unwrap_or(true);
        let mut unused: Vec<&shoebox_search::models::ModelFile> = vec![];
        if !indexing {
            unused.extend([&CLIP_IMAGE, &OCR_DET, &OCR_REC, &OCR_DICT]);
        }
        if !indexing || !people {
            unused.extend([&FACE_DETECT, &FACE_EMBED]);
        }
        let dir = self.data_dir().join("models");
        for file in unused {
            let path = dir.join(file.name);
            let had = path.exists();
            if had {
                let _ = std::fs::remove_file(&path);
            }
            if let Err(e) = ln.forget_model(file).await {
                tracing::warn!("could not stop offering {}: {e:#}", file.name);
            } else if had {
                tracing::info!("removed {}: its feature is off here", file.name);
            }
        }
    }

    /// The user asked for indexing now (Settings → Search): run until this device has nothing
    /// left, with no power or idle gate, for as long as the app's process lives. Returns false
    /// when a run is already going.
    pub fn index_now(self: &Arc<Self>) -> bool {
        let Some(run) = IndexRun::begin() else {
            return false;
        };
        *LAST_RUN.lock().unwrap() = None;
        let core = self.clone();
        tokio::spawn(async move {
            let _run = run;
            let note = match core.index_burst(MANUAL_BUDGET, false).await {
                Ok(v) => v
                    .get("skipped")
                    .and_then(|s| s.as_str())
                    .map(|s| format!("Indexing did not start: {s}")),
                Err(e) => Some(format!("Indexing stopped: {e:#}")),
            };
            *LAST_RUN.lock().unwrap() = note;
        });
        true
    }

    /// Ask the run `index_now` started to stop.
    pub fn stop_index_now(&self) {
        if INDEX_RUNNING.load(Ordering::SeqCst) {
            let _ = std::fs::write(index_stop_path(self.data_dir()), b"stop");
        }
    }

    /// Set up search by meaning for `ln`: the text model, the model files offered to the other
    /// devices, and (on a desktop) the indexer.
    pub(crate) async fn start_search_models(&self, ln: &LibraryNode<Library>) {
        ln.set_search_model(CLIP_MODEL);
        self.remove_unused_models(ln).await;
        let dirs = model_dirs(&self.opts.model_dirs, self.data_dir());
        let d = dirs.clone();
        ln.set_text_encoder(Arc::new(move || {
            Ok(Arc::new(ClipText::bundled(&d)?) as Arc<dyn TextEncoder>)
        }));
        for file in ALL {
            if let Some(path) = locate(file, &dirs) {
                if let Err(e) = ln.share_model(file, &path).await {
                    tracing::warn!("could not offer {}: {e:#}", file.name);
                }
            }
        }
        if cfg!(target_os = "android") || !SearchSettings::load(self.data_dir()).index {
            return;
        }
        #[cfg(all(feature = "tauri-app", target_os = "macos"))]
        ln.set_ocr(Arc::new(crate::ocr_macos::VisionOcr));
        // Everywhere else, PP-OCRv5 (S2-03), when its files are here.
        #[cfg(not(target_os = "macos"))]
        {
            let d = dirs.clone();
            match tokio::task::spawn_blocking(move || PpOcr::locate(&d)).await {
                Ok(Ok(Some(e))) => ln.set_ocr(Arc::new(e)),
                Ok(Ok(None)) => {
                    tracing::info!("the text models are not installed: no text reading here")
                }
                Ok(Err(e)) => tracing::warn!("could not load the text models: {e:#}"),
                Err(e) => tracing::warn!("could not load the text models: {e}"),
            }
        }
        let Some(path) = locate(&CLIP_IMAGE, &dirs) else {
            tracing::info!("{} is not installed: no indexing here", CLIP_IMAGE.name);
            return;
        };
        let encoder =
            match tokio::task::spawn_blocking(move || ClipImage::load(&path, CLIP_MODEL)).await {
                Ok(Ok(e)) => e,
                Ok(Err(e)) => return tracing::warn!("could not load the image model: {e:#}"),
                Err(e) => return tracing::warn!("could not load the image model: {e}"),
            };
        if let (Some(d), Some(e)) = (locate(&FACE_DETECT, &dirs), locate(&FACE_EMBED, &dirs)) {
            match (YuNet::load(&d), SFace::load(&e)) {
                (Ok(detector), Ok(embedder)) => ln.set_faces(FaceModels {
                    detector: Arc::new(detector),
                    embedder: Arc::new(embedder),
                    model: FACES_MODEL,
                }),
                (Err(e), _) | (_, Err(e)) => {
                    tracing::warn!("could not load the face models: {e:#}")
                }
            }
        }
        let cores = std::thread::available_parallelism().map_or(2, |n| n.get());
        ln.start_indexer(
            IndexerConfig {
                always_on: true,
                threads: (cores / 2).max(1),
                ..Default::default()
            },
            Arc::new(encoder) as Arc<dyn ImageEncoder>,
            Arc::new(DesktopGate::default()),
        );
    }

    /// One bounded indexing run (the Android job that runs while charging): catch up with the
    /// other devices, get the image model (this phone's own folder, then a device that offers
    /// it, then, if `allow_download`, the release), index this device's photos until the budget
    /// runs out or nothing is left, and publish what was done.
    pub async fn index_burst(
        self: &Arc<Self>,
        budget: Duration,
        allow_download: bool,
    ) -> Result<serde_json::Value> {
        let started = Instant::now();
        let ln = self.ln().await?;
        let _ = self.sync_burst(budget / 4).await;
        if !SearchSettings::load(self.data_dir()).index {
            return Ok(serde_json::json!({ "skipped": "indexing is off on this device" }));
        }
        self.remove_unused_models(&ln).await;
        let people = ln.people_on().await.unwrap_or(true);
        let sources = self.model_sources(allow_download);
        let image = ln.model_bytes_from(&CLIP_IMAGE, &sources).await;
        let faces = match image {
            // Faces are a bonus: a download that fails here is tried again next time.
            Ok(Some(_)) if people => {
                let d = ln.model_bytes_from(&FACE_DETECT, &sources).await;
                let e = ln.model_bytes_from(&FACE_EMBED, &sources).await;
                match (d, e) {
                    (Ok(Some(d)), Ok(Some(e))) => Some((d, e)),
                    _ => None,
                }
            }
            _ => None,
        };
        // Text reading is a bonus too (S2-03): all three files, or none.
        let text = match image {
            Ok(Some(_)) => {
                let det = ln.model_bytes_from(&OCR_DET, &sources).await;
                let rec = ln.model_bytes_from(&OCR_REC, &sources).await;
                let dict = ln.model_bytes_from(&OCR_DICT, &sources).await;
                match (det, rec, dict) {
                    (Ok(Some(a)), Ok(Some(b)), Ok(Some(c))) => Some((a, b, c)),
                    _ => None,
                }
            }
            _ => None,
        };
        clear_download_progress(self.data_dir());
        let bytes = match image {
            Ok(Some(bytes)) => bytes,
            Ok(None) => {
                return Ok(serde_json::json!({
                    "skipped": if allow_download {
                        "the image model is not available"
                    } else {
                        "no device offers the image model, and a download waits for Wi-Fi"
                    }
                }));
            }
            Err(e) => {
                return Ok(serde_json::json!({
                    "skipped": format!("could not get the image model: {e:#}")
                }));
            }
        };
        let encoder =
            tokio::task::spawn_blocking(move || ClipImage::from_bytes(&bytes, CLIP_MODEL))
                .await??;
        if let Some((d, e)) = faces {
            let loaded = tokio::task::spawn_blocking(move || {
                Ok::<_, anyhow::Error>((YuNet::from_bytes(&d)?, SFace::from_bytes(&e)?))
            })
            .await?;
            if let Ok((detector, embedder)) = loaded {
                ln.set_faces(FaceModels {
                    detector: Arc::new(detector),
                    embedder: Arc::new(embedder),
                    model: FACES_MODEL,
                });
            }
        }
        if let Some((det, rec, dict)) = text {
            let loaded = tokio::task::spawn_blocking(move || {
                PpOcr::from_bytes(&det, &rec, &String::from_utf8(dict.to_vec())?)
            })
            .await?;
            match loaded {
                Ok(e) => ln.set_ocr(Arc::new(e)),
                Err(e) => tracing::warn!("could not load the text models: {e:#}"),
            }
        }
        ln.start_indexer(
            IndexerConfig {
                tick: Duration::from_secs(2),
                threads: 2,
                ..Default::default()
            },
            Arc::new(encoder) as Arc<dyn ImageEncoder>,
            Arc::new(shoebox_node::Always),
        );
        // On a core that has run before (the app's own, borrowed by the job) the indexer is
        // already running, paused since the last run.
        ln.resume_indexer();
        // Run until this device has nothing left to do (pictures, text, faces), the budget is
        // spent, or the Android side asks it to stop (too hot, unplugged): `index-stop`.
        let stop = index_stop_path(self.data_dir());
        let _ = std::fs::remove_file(&stop);
        let mut flushed = Instant::now();
        let caught_up = loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            // Publish what has been read so far, so the counts in Settings move and a run that
            // is killed keeps its work: text and face rows otherwise wait for a full shard.
            if flushed.elapsed() >= Duration::from_secs(15) {
                flushed = Instant::now();
                if let Err(e) = ln.flush_index().await {
                    tracing::warn!("could not publish index shards: {e:#}");
                }
            }
            let p = ln.index_progress();
            write_index_progress(self.data_dir(), &p, false);
            if p.caught_up {
                break true;
            }
            if started.elapsed() >= budget || stop.exists() {
                break false;
            }
        };
        let _ = std::fs::remove_file(&stop);
        ln.pause_indexer();
        let published = ln.flush_index().await?;
        // Group faces and compact shards when this device is the one the rule picks (S2-01), so a
        // phone on its own finishes the whole job in one run.
        if caught_up {
            if let Err(e) = ln.regroup_people(FACES_MODEL, false).await {
                tracing::warn!("could not group faces: {e:#}");
            }
            if let Err(e) = ln.compact_shards(false).await {
                tracing::warn!("could not compact index shards: {e:#}");
            }
        }
        let _ = self.sync_burst(Duration::from_secs(20)).await;
        let p = ln.index_progress();
        write_index_progress(self.data_dir(), &p, caught_up);
        Ok(serde_json::json!({
            "indexed": published,
            "caught_up": caught_up,
            "progress": format!("{p:?}"),
        }))
    }
}

impl AppCore {
    /// Where a model file may come from on this device (S2-02): its own folders, and (when
    /// `allow_download`) the release, into the data folder's `models/`. Progress goes to a file
    /// the app's Settings reads, since the indexing job runs in a process of its own.
    fn model_sources(&self, allow_download: bool) -> ModelSources {
        let data = self.data_dir().to_path_buf();
        let last = Arc::new(Mutex::new(0u64));
        ModelSources {
            dirs: model_dirs(&self.opts.model_dirs, &data),
            download_to: allow_download.then(|| data.join("models")),
            progress: Some(Arc::new({
                let data = data.clone();
                move |file: &ModelFile, done, total| {
                    let mut last = last.lock().unwrap();
                    // About once a megabyte, and at the end.
                    if done >= *last + (1 << 20) || done == total {
                        *last = done;
                        write_download_progress(&data, file.name, done, total);
                    }
                }
            })),
        }
    }
}

const DOWNLOAD_FILE: &str = "model-download.json";
/// A progress file older than this belongs to a job that was killed.
const DOWNLOAD_STALE: Duration = Duration::from_secs(90);

#[derive(serde::Serialize, serde::Deserialize)]
struct DownloadFile {
    name: String,
    done: u64,
    total: u64,
    at_ms: u64,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

fn write_download_progress(dir: &std::path::Path, name: &str, done: u64, total: u64) {
    let f = DownloadFile {
        name: name.to_string(),
        done,
        total,
        at_ms: now_ms(),
    };
    if let Ok(json) = serde_json::to_vec(&f) {
        let _ = std::fs::write(dir.join(DOWNLOAD_FILE), json);
    }
}

fn clear_download_progress(dir: &std::path::Path) {
    let _ = std::fs::remove_file(dir.join(DOWNLOAD_FILE));
}

/// The model being downloaded now, if a job reported progress lately.
fn download_progress(dir: &std::path::Path) -> Option<crate::types::ModelDownload> {
    let f: DownloadFile =
        serde_json::from_slice(&std::fs::read(dir.join(DOWNLOAD_FILE)).ok()?).ok()?;
    (now_ms().saturating_sub(f.at_ms) < DOWNLOAD_STALE.as_millis() as u64).then_some(
        crate::types::ModelDownload {
            name: f.name,
            done: f.done as f64,
            total: f.total as f64,
        },
    )
}

/// Where an indexing run on Android tells the notification how far it has got (it runs in its own
/// process, like the model download): `{"indexed", "total", "left", "done"}`.
pub fn index_progress_path(dir: &std::path::Path) -> PathBuf {
    dir.join("index-progress.json")
}

/// A file the Android job creates to ask a run to stop (the phone is too hot or unplugged).
pub fn index_stop_path(dir: &std::path::Path) -> PathBuf {
    dir.join("index-stop")
}

fn write_index_progress(dir: &std::path::Path, p: &shoebox_node::IndexProgress, done: bool) {
    let json = serde_json::json!({
        "indexed": p.indexed,
        "total": p.total,
        "left": p.total.saturating_sub(p.indexed),
        "done": done,
    });
    let path = index_progress_path(dir);
    let part = path.with_extension("part");
    if std::fs::write(&part, json.to_string()).is_ok() {
        let _ = std::fs::rename(&part, &path);
    }
}

impl AppCore {
    /// How much of the library search covers on this device, and who is indexing (S1-16).
    pub async fn search_status(&self) -> Result<crate::types::SearchStatus> {
        let ln = self.ln().await?;
        let s = ln.search_status().await?;
        let p = ln.index_progress();
        let names: std::collections::HashMap<_, _> = ln
            .store()
            .device_list()
            .await?
            .devices
            .into_iter()
            .map(|d| (d.device, d.name))
            .collect();
        Ok(crate::types::SearchStatus {
            total: s.total as u32,
            content: s.content as u32,
            labelled: s.labelled as u32,
            text_read: s.text_read as u32,
            faces: s.faces as u32,
            text_wanted: s.text_wanted as u32,
            text_wanted_read: s.text_wanted_read as u32,
            faces_wanted: s.faces_wanted as u32,
            people_on: s.people_on,
            meaning: s.meaning,
            indexers: s
                .indexers
                .iter()
                .map(|i| crate::types::SearchIndexer {
                    name: names.get(&i.device).cloned().unwrap_or_default(),
                    photos_per_min: i.photos_per_min,
                    this_device: i.device == ln.device(),
                    reads_text: i.reads_text,
                    finds_faces: i.finds_faces,
                    device_id: hex::encode(i.device.0),
                })
                .collect(),
            eta_secs: s.eta_secs.map(|e| e.min(u64::from(u32::MAX)) as u32),
            held_bytes: s.held_bytes as f64,
            index_here: SearchSettings::load(self.data_dir()).index,
            indexing: p.running && !p.paused,
            stage: p.stage.to_string(),
            waiting: p.waiting.clone(),
            caught_up: p.caught_up,
            skipped_pictures: p.skipped_pictures as u32,
            skipped_text: p.skipped_text as u32,
            skipped_faces: p.skipped_faces as u32,
            run_active: INDEX_RUNNING.load(Ordering::SeqCst),
            last_run: LAST_RUN.lock().unwrap().clone(),
            model_download: download_progress(self.data_dir()),
        })
    }

    /// Turn indexing on this device on or off. Off, its status says 0 photos a minute and the
    /// other devices take over its share.
    pub async fn set_search_indexing(&self, on: bool) -> Result<()> {
        SearchSettings { index: on }.save(self.data_dir())?;
        let ln = self.ln().await?;
        if on {
            ln.resume_indexer();
            if !ln.index_progress().running && self.opts.search_models {
                self.start_search_models(&ln).await;
            }
        } else {
            ln.pause_indexer();
        }
        self.remove_unused_models(&ln).await;
        Ok(())
    }

    /// Forget this device's search index and build it again (S1-16).
    pub async fn rebuild_search_index(&self) -> Result<()> {
        let ln = self.ln().await?;
        ln.rebuild_search_index().await
    }
}

/// Desktop: index on mains power while the user has been away for two minutes. Where this
/// cannot be told (no battery, or no way to ask), it counts as plugged in and idle.
#[derive(Default)]
pub struct DesktopGate {
    last: Mutex<Option<(Instant, bool)>>,
}

impl IndexGate for DesktopGate {
    fn may_index(&self) -> bool {
        let mut last = self.last.lock().unwrap();
        if let Some((at, open)) = *last {
            if at.elapsed() < Duration::from_secs(30) {
                return open;
            }
        }
        let open = on_mains() && idle_for().is_none_or(|d| d >= Duration::from_secs(120));
        *last = Some((Instant::now(), open));
        open
    }
}

pub(crate) fn on_mains() -> bool {
    #[cfg(target_os = "macos")]
    {
        if let Ok(out) = std::process::Command::new("pmset")
            .args(["-g", "batt"])
            .output()
        {
            let text = String::from_utf8_lossy(&out.stdout);
            return !text.contains("'Battery Power'");
        }
        true
    }
    #[cfg(target_os = "linux")]
    {
        let Ok(dir) = std::fs::read_dir("/sys/class/power_supply") else {
            return true;
        };
        let mut mains = None;
        for e in dir.flatten() {
            let p = e.path();
            let kind = std::fs::read_to_string(p.join("type")).unwrap_or_default();
            if kind.trim() == "Mains" {
                let online = std::fs::read_to_string(p.join("online")).unwrap_or_default();
                mains = Some(mains.unwrap_or(false) || online.trim() == "1");
            }
        }
        mains.unwrap_or(true)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        true
    }
}

/// How long since the last keyboard or mouse input, where the system says.
fn idle_for() -> Option<Duration> {
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("ioreg")
            .args(["-c", "IOHIDSystem", "-d", "4"])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let line = text.lines().find(|l| l.contains("\"HIDIdleTime\""))?;
        let ns: u64 = line.rsplit('=').next()?.trim().parse().ok()?;
        Some(Duration::from_nanos(ns))
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}
