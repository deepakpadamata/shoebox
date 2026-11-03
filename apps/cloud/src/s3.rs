//! Bytes in an S3-compatible bucket. The service keeps its bookkeeping and hands out short-lived presigned URLs:
//! clients upload straight to the bucket and readers are redirected to it, so no photo's bytes pass through here.
//! The few requests the service makes itself (a manifest, a size check, a delete) are presigned too.

use anyhow::{anyhow, bail, Context, Result};
use jiff::Timestamp;
use reqwest::header::CONTENT_LENGTH;
use rusty_s3::actions::{DeleteObject, GetObject, HeadObject, PutObject};
use rusty_s3::{Bucket, Credentials, S3Action, UrlStyle};
use std::sync::Arc;
use std::time::Duration;
use url::Url;

/// Where the bucket is and how to sign for it.
#[derive(Clone, Debug)]
pub struct S3Config {
    /// The S3 endpoint, e.g. `https://<account>.r2.cloudflarestorage.com` or `https://s3.eu-central-003.backblazeb2.com`.
    /// Browsers are redirected to it, so it must be reachable from the internet.
    pub endpoint: Url,
    pub bucket: String,
    /// `auto` for R2; the bucket's region elsewhere.
    pub region: String,
    pub access_key: String,
    pub secret_key: String,
    /// `https://endpoint/bucket/key` rather than `https://bucket.endpoint/key`.
    pub path_style: bool,
}

pub struct S3 {
    bucket: Bucket,
    creds: Credentials,
    http: reqwest::Client,
}

/// How long a reader's link to a blob lasts. A revoked or expired share stops handing out new ones at once; one
/// already handed out works until it runs out.
pub const READ_TTL: Duration = Duration::from_secs(600);
/// How long an upload link lasts (a large original on a slow line).
pub const WRITE_TTL: Duration = Duration::from_secs(3600);
const OWN_TTL: Duration = Duration::from_secs(60);

fn client() -> Result<reqwest::Client> {
    let roots = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.into(),
    };
    let tls = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()?
    .with_root_certificates(roots)
    .with_no_client_auth();
    Ok(reqwest::Client::builder()
        .tls_backend_preconfigured(tls)
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(120))
        .redirect(reqwest::redirect::Policy::none())
        .build()?)
}

fn at(now_ms: i64) -> Timestamp {
    Timestamp::from_millisecond(now_ms).unwrap_or(Timestamp::UNIX_EPOCH)
}

impl S3 {
    pub fn new(c: &S3Config) -> Result<Self> {
        let style = if c.path_style {
            UrlStyle::Path
        } else {
            UrlStyle::VirtualHost
        };
        let bucket = Bucket::new(
            c.endpoint.clone(),
            style,
            c.bucket.clone(),
            c.region.clone(),
        )
        .map_err(|e| anyhow!("the S3 bucket: {e}"))?;
        Ok(Self {
            bucket,
            creds: Credentials::new(c.access_key.clone(), c.secret_key.clone()),
            http: client()?,
        })
    }

    /// The bucket's origin (`https://host[:port]`), for the viewer page's `connect-src`.
    pub fn origin(&self) -> String {
        self.bucket.base_url().origin().ascii_serialization()
    }

    pub fn get_url(&self, key: &str, now_ms: i64) -> Url {
        GetObject::new(&self.bucket, Some(&self.creds), key).sign_with_time(READ_TTL, &at(now_ms))
    }

    /// A URL to upload exactly `len` bytes to `key`: the length is signed, so the bucket refuses any other.
    pub fn put_url(&self, key: &str, len: u64, now_ms: i64) -> Url {
        let mut a = PutObject::new(&self.bucket, Some(&self.creds), key);
        a.headers_mut().insert("content-length", len.to_string());
        a.sign_with_time(WRITE_TTL, &at(now_ms))
    }

    /// The size of what is stored under `key`, or `None` when nothing is.
    pub async fn head(&self, key: &str, now_ms: i64) -> Result<Option<u64>> {
        let url = HeadObject::new(&self.bucket, Some(&self.creds), key)
            .sign_with_time(OWN_TTL, &at(now_ms));
        let res = self
            .http
            .head(url)
            .send()
            .await
            .context("asking the bucket")?;
        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !res.status().is_success() {
            bail!("the bucket answered {}", res.status());
        }
        res.headers()
            .get(CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
            .map(Some)
            .context("the bucket did not say how large it is")
    }

    pub async fn put(&self, key: &str, bytes: Vec<u8>, now_ms: i64) -> Result<()> {
        let len = bytes.len() as u64;
        let url = self.put_url(key, len, now_ms);
        let res = self
            .http
            .put(url)
            .header(CONTENT_LENGTH, len)
            .body(bytes)
            .send()
            .await
            .context("writing to the bucket")?;
        if !res.status().is_success() {
            bail!("the bucket answered {}", res.status());
        }
        Ok(())
    }

    /// Delete `key`; deleting what is not there is not an error.
    pub async fn delete(&self, key: &str, now_ms: i64) -> Result<()> {
        let url = DeleteObject::new(&self.bucket, Some(&self.creds), key)
            .sign_with_time(OWN_TTL, &at(now_ms));
        let res = self
            .http
            .delete(url)
            .send()
            .await
            .context("deleting from the bucket")?;
        if !res.status().is_success() && res.status() != reqwest::StatusCode::NOT_FOUND {
            bail!("the bucket answered {}", res.status());
        }
        Ok(())
    }
}

/// Where a library's blob, a library's object and a share's manifest live in the bucket (the same layout as on disk).
pub fn blob_key(lib: &[u8; 16], cid: &[u8; 32]) -> String {
    format!("libs/{}/blobs/{}", hex::encode(lib), hex::encode(cid))
}
pub fn object_key(lib: &[u8; 16], name: &str) -> String {
    format!("libs/{}/objects/{name}", hex::encode(lib))
}
pub fn manifest_key(share: &[u8; 16]) -> String {
    format!("shares/{}/manifest", hex::encode(share))
}
