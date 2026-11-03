//! Shoebox Cloud (W1-04): a store for ciphertext that serves web links to shared albums and holds a library as
//! the cloud keeper. It never sees a photo, a name or a key: blobs and manifests arrive encrypted, and a share's
//! key lives only in the link's `#fragment`, which no browser sends.

mod api;
pub mod limits;
pub mod store;

use axum::Router;
use limits::Limiter;
use std::path::PathBuf;
use std::sync::Arc;
use store::Store;

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
            clock: system_clock(),
        }
    }
}

pub struct Service {
    pub config: Config,
    pub store: Store,
    pub(crate) limiter: Limiter,
}

impl Service {
    pub fn open(config: Config) -> std::io::Result<Arc<Self>> {
        let store = Store::open(&config.data_dir)?;
        Ok(Arc::new(Self {
            config,
            store,
            limiter: Limiter::default(),
        }))
    }
    pub fn now(&self) -> i64 {
        (self.config.clock)()
    }
}

pub fn router(service: Arc<Service>) -> Router {
    api::router(service)
}
