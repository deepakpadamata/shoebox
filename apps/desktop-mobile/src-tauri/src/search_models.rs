//! Search by meaning in the app (S1-09…S1-12): the text model for every search; indexing on a
//! desktop on mains power while idle, and on a phone in a background job while charging, with
//! the image model the phone fetches from a desktop or keeper that offers it.

use crate::core::AppCore;
use anyhow::Result;
use shoebox_core::store::LibraryStore;
use shoebox_ml::files::{locate, search_dirs};
use shoebox_ml::{ClipImage, ClipText, SFace, YuNet};
use shoebox_node::{FaceModels, IndexGate, IndexerConfig, LibraryNode, ModelSources};
use shoebox_search::encoder::{ImageEncoder, TextEncoder};
use shoebox_search::models::{
    ModelFile, ALL, CLIP_IMAGE, CLIP_MODEL, FACES_MODEL, FACE_DETECT, FACE_EMBED,
};
use shoebox_store::Library;
use std::path::PathBuf;
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

impl AppCore {
    /// Set up search by meaning for `ln`: the text model, the model files offered to the other
    /// devices, and (on a desktop) the indexer.
    pub(crate) async fn start_search_models(&self, ln: &LibraryNode<Library>) {
        ln.set_search_model(CLIP_MODEL);
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
        let sources = self.model_sources(allow_download);
        let image = ln.model_bytes_from(&CLIP_IMAGE, &sources).await;
        let faces = match image {
            // Faces are a bonus: a download that fails here is tried again next time.
            Ok(Some(_)) => {
                let d = ln.model_bytes_from(&FACE_DETECT, &sources).await;
                let e = ln.model_bytes_from(&FACE_EMBED, &sources).await;
                match (d, e) {
                    (Ok(Some(d)), Ok(Some(e))) => Some((d, e)),
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
        ln.start_indexer(
            IndexerConfig {
                tick: Duration::from_secs(2),
                threads: 2,
                ..Default::default()
            },
            Arc::new(encoder) as Arc<dyn ImageEncoder>,
            Arc::new(shoebox_node::Always),
        );
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let p = ln.index_progress();
            if p.mine_left == 0 || started.elapsed() >= budget {
                break;
            }
        }
        ln.pause_indexer();
        let published = ln.flush_index().await?;
        let _ = self.sync_burst(Duration::from_secs(20)).await;
        Ok(
            serde_json::json!({ "indexed": published, "progress": format!("{:?}", ln.index_progress()) }),
        )
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
            meaning: s.meaning,
            indexers: s
                .indexers
                .iter()
                .map(|i| crate::types::SearchIndexer {
                    name: names.get(&i.device).cloned().unwrap_or_default(),
                    photos_per_min: i.photos_per_min,
                    this_device: i.device == ln.device(),
                    reads_text: i.reads_text,
                })
                .collect(),
            eta_secs: s.eta_secs.map(|e| e.min(u64::from(u32::MAX)) as u32),
            held_bytes: s.held_bytes as f64,
            index_here: SearchSettings::load(self.data_dir()).index,
            indexing: p.running && !p.paused,
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
