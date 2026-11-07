//! Shoebox Cloud (W1-04): a store for ciphertext that serves web links to shared albums and holds a library as
//! the cloud keeper. It never sees a photo, a name or a key: blobs and manifests arrive encrypted, and a share's
//! key lives only in the link's `#fragment`, which no browser sends.
//!
//! The bytes are kept in the data directory, or in an S3-compatible bucket (`s3`), when the service is only a
//! bookkeeper: uploads and reads then go straight to the bucket through short-lived presigned URLs.
//!
//! Two builds: the `server` feature (the default) is the `shoebox-cloud` program, keeping its records in a data
//! directory; without it the crate builds for WebAssembly and runs in a Cloudflare Durable Object
//! (`apps/cloud-worker`), which hands in its own records ([`meta::Meta`]) and HTTP client ([`s3::Http`]).

mod api;
#[cfg(feature = "server")]
mod disk;
pub mod limits;
pub mod meta;
pub mod s3;
pub mod store;

use axum::Router;
use limits::Limiter;
use meta::Meta;
use s3::{Http, S3Config, S3};
use std::path::PathBuf;
use std::sync::Arc;
use store::{Store, Target};

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
    /// Libraries admitted with their first request even while registration is closed (the operator's own).
    pub allowed_libraries: Vec<[u8; 16]>,
    /// Take the client address from the last `X-Forwarded-For` entry (behind a proxy we run).
    pub trust_proxy: bool,
    /// How far a request's signed time may be from ours.
    pub skew_ms: i64,
    /// Extra hosts the viewer page may connect to (relays, for Live links), added to the page's CSP.
    pub csp_connect: Vec<String>,
    /// Keep the bytes in this bucket instead of the data directory.
    pub s3: Option<S3Config>,
    /// The operator's token for `/admin/...` (reported shares). No token, no admin calls.
    pub admin_token: Option<String>,
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

/// Library ids written as hex, separated by spaces or commas; anything that is not one is an error.
pub fn parse_library_ids(text: &str) -> anyhow::Result<Vec<[u8; 16]>> {
    text.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .map(|s| {
            let bytes = hex::decode(s).map_err(|_| anyhow::anyhow!("{s} is not a library id"))?;
            bytes
                .try_into()
                .map_err(|_| anyhow::anyhow!("{s} is not a library id (32 hex digits)"))
        })
        .collect()
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
            allowed_libraries: vec![],
            trust_proxy: false,
            skew_ms: 5 * 60 * 1000,
            csp_connect: vec![],
            s3: None,
            admin_token: None,
            clock: system_clock(),
        }
    }
}

pub struct Service {
    pub config: Config,
    pub store: Store,
    pub(crate) limiter: Limiter,
    pub(crate) s3: Option<S3>,
}

impl Service {
    /// Open the service on its data directory, with reqwest for the bucket.
    #[cfg(feature = "server")]
    pub fn open(config: Config) -> anyhow::Result<Arc<Self>> {
        let meta: Arc<dyn Meta> = Arc::new(meta::DirMeta::new(&config.data_dir));
        let disk = config.s3.is_none().then(|| config.data_dir.clone());
        Self::open_with(config, meta, disk, Arc::new(s3::Reqwest::new()?))
    }

    /// Open the service on records kept elsewhere. `disk` holds the bytes when there is no bucket (a server only).
    pub fn open_with(
        config: Config,
        meta: Arc<dyn Meta>,
        disk: Option<PathBuf>,
        http: Arc<dyn Http>,
    ) -> anyhow::Result<Arc<Self>> {
        let s3 = config.s3.as_ref().map(|c| S3::new(c, http)).transpose()?;
        if s3.is_none() && disk.is_none() {
            anyhow::bail!("the bytes need somewhere to go: a bucket or a data directory");
        }
        let store = Store::open(meta, if s3.is_some() { None } else { disk })?;
        Ok(Arc::new(Self {
            config,
            store,
            limiter: Limiter::default(),
            s3,
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
        let stale = self.store.take_expired_uploads(now);
        for p in stale {
            if p.done {
                continue;
            }
            let lib = shoebox_core::store::LibraryId(p.library);
            self.store.release(lib, p.size);
            let Some(s3) = &self.s3 else { continue };
            let (key, kept) = match &p.target {
                Target::Blob(cid) => (
                    s3::blob_key(&p.library, cid),
                    self.store.has_blob(lib, cid).is_some(),
                ),
                Target::Object(name) => (
                    s3::object_key(&p.library, name),
                    self.store.object_len(lib, name).is_some(),
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

/// The viewer page's Content-Security-Policy. `bucket` is the bucket's origin when readers are redirected there
/// (`connect-src` applies to where a redirect leads); a Worker that fetches the bucket itself passes `None`.
pub fn page_csp(config: &Config, bucket: Option<String>) -> String {
    let connect = std::iter::once("'self'".to_string())
        .chain(bucket)
        .chain(config.csp_connect.iter().cloned())
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; img-src 'self' blob: data:; \
         media-src blob:; connect-src {connect}; worker-src 'self' blob:; base-uri 'none'; \
         frame-ancestors 'none'; form-action 'none'"
    )
}
