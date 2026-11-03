//! Shoebox Cloud (W1-04): a store for ciphertext that serves web links to shared albums and holds a library as
//! the cloud keeper. It never sees a photo, a name or a key: blobs and manifests arrive encrypted, and a share's
//! key lives only in the link's `#fragment`, which no browser sends.
//!
//! The bytes are kept in the data directory, or in an S3-compatible bucket (`s3`), when the service is only a
//! bookkeeper: uploads and reads then go straight to the bucket through short-lived presigned URLs.

mod api;
pub mod limits;
pub mod s3;
pub mod store;

use axum::Router;
use limits::Limiter;
use s3::{S3Config, S3};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use store::{Cid, Store};

/// Settings. Every number is a default a deployment can change; W1-10 sets them from a subscription.
#[derive(Clone)]
pub struct Config {
    pub data_dir: PathBuf,
    /// A directory with the viewer page (`index.html`, scripts, the wasm), served at `/` and `/a`.
    pub viewer_dir: Option<PathBuf>,
    /// Bytes a library may store (blobs, objects and manifests).
    pub library_quota: u64,
    /// The largest single blob.
    pub max_blob: u64,
    /// Bytes one share may serve a day.
    pub share_daily_bytes: u64,
    pub share_requests_per_min: u32,
    pub ip_requests_per_min: u32,
    /// Let an unknown library register with its first request. Off once registration needs a subscription.
    pub open_registration: bool,
    /// Take the client address from the last `X-Forwarded-For` entry (behind a proxy we run).
    pub trust_proxy: bool,
    /// How far a request's signed time may be from ours.
    pub skew_ms: i64,
    /// Extra hosts the viewer page may connect to (relays, for Live links), added to the page's CSP.
    pub csp_connect: Vec<String>,
    /// Keep the bytes in this bucket instead of the data directory.
    pub s3: Option<S3Config>,
    /// Milliseconds since the epoch; a test clock in tests.
    pub clock: Arc<dyn Fn() -> i64 + Send + Sync>,
}

pub fn system_clock() -> Arc<dyn Fn() -> i64 + Send + Sync> {
    Arc::new(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64)
    })
}

impl Config {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            viewer_dir: None,
            library_quota: 2 * 1024 * 1024 * 1024,
            max_blob: 4 * 1024 * 1024 * 1024,
            share_daily_bytes: 1024 * 1024 * 1024,
            share_requests_per_min: 600,
            ip_requests_per_min: 1200,
            open_registration: true,
            trust_proxy: false,
            skew_ms: 5 * 60 * 1000,
            csp_connect: vec![],
            s3: None,
            clock: system_clock(),
        }
    }
}

/// What an upload is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    Blob(Cid),
    Object(String),
}

/// An upload the service handed out a URL for and has reserved quota for, until it is committed or runs out.
pub(crate) struct Pending {
    pub lib: shoebox_core::store::LibraryId,
    pub target: Target,
    pub size: u64,
    /// What the bytes hash to (the blob's id, or the object's declared hash). Checked when they pass through the
    /// service; a bucket cannot check it, and a reader checks it anyway.
    pub hash: [u8; 32],
    pub expires_ms: i64,
    /// The bytes arrived and were recorded (they went through the service).
    pub done: bool,
}

pub struct Service {
    pub config: Config,
    pub store: Store,
    pub(crate) limiter: Limiter,
    pub(crate) s3: Option<S3>,
    pub(crate) uploads: Mutex<HashMap<[u8; 16], Pending>>,
}

impl Service {
    pub fn open(config: Config) -> anyhow::Result<Arc<Self>> {
        let s3 = config.s3.as_ref().map(S3::new).transpose()?;
        let store = Store::open(&config.data_dir, s3.is_some())?;
        Ok(Arc::new(Self {
            config,
            store,
            limiter: Limiter::default(),
            s3,
            uploads: Mutex::new(HashMap::new()),
        }))
    }
    pub fn now(&self) -> i64 {
        (self.config.clock)()
    }

    /// A janitor run: delete shares that expired a week ago (and their manifests in the bucket), and give back the
    /// quota of uploads that were never finished (deleting what reached the bucket of them).
    pub async fn sweep(&self) -> usize {
        let now = self.now();
        let gone = self.store.sweep_expired(now - 7 * 24 * 3600 * 1000);
        if let Some(s3) = &self.s3 {
            for id in &gone {
                if let Err(e) = s3.delete(&s3::manifest_key(id), now).await {
                    tracing::warn!("deleting an expired share's manifest: {e:#}");
                }
            }
        }
        let stale: Vec<Pending> = {
            let mut u = self.uploads.lock().unwrap_or_else(|e| e.into_inner());
            let ids: Vec<[u8; 16]> = u
                .iter()
                .filter(|(_, p)| p.expires_ms <= now)
                .map(|(id, _)| *id)
                .collect();
            ids.iter().filter_map(|id| u.remove(id)).collect()
        };
        for p in stale {
            if p.done {
                continue;
            }
            self.store.release(p.lib, p.size);
            let Some(s3) = &self.s3 else { continue };
            let (key, kept) = match &p.target {
                Target::Blob(cid) => (
                    s3::blob_key(&p.lib.0, cid),
                    self.store.has_blob(p.lib, cid).is_some(),
                ),
                Target::Object(name) => (
                    s3::object_key(&p.lib.0, name),
                    self.store.object_len(p.lib, name).is_some(),
                ),
            };
            if !kept {
                let _ = s3.delete(&key, now).await;
            }
        }
        gone.len()
    }
}

pub fn router(service: Arc<Service>) -> Router {
    api::router(service)
}
