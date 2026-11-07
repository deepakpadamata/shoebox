//! Web links to albums, from the sharer's side (W1-05): put an album's thumbnails and previews on Shoebox Cloud as
//! ciphertext, seal a manifest of them, and hand back a link whose `#fragment` is the only place the key exists.
//! After that the link works with every device off. See `planning/epics/W1-web-albums-and-cloud.md`.
//!
//! Library-level state lives in library settings, so every member device sees it (library records are encrypted
//! between devices like the photos themselves):
//!
//! * `cloud-secret`: the library's cloud secret (W1-02), set when the library is created or when the person types
//!   the recovery phrase once, since the root key itself is never kept;
//! * `cloud-url`: the service's address;
//! * `web-shares-<album>`: that album's links, each with the share key, so that any device can refresh it.

use crate::LibraryNode;
use anyhow::{anyhow, bail, ensure, Context, Result};
use reqwest::header::{AUTHORIZATION, CONTENT_LENGTH};
use serde::{Deserialize, Serialize};
use shoebox_core::cloud::{BlobKey, CloudSecret, CHUNK, TAG};
use shoebox_core::cloud_auth::{CertBundle, CloudAuth};
use shoebox_core::ids::{Blake3Hash, Timestamp};
use shoebox_core::model::{AdminCert, AlbumId, DeviceCert, PhotoRecord};
use shoebox_core::recovery::RecoveryKit;
use shoebox_core::share::{
    self, CloudBlob, Item, ItemKind, Manifest, ShareKey, ShareLink, ShareUpload,
};
use shoebox_core::store::{LibraryId, LibraryStore, Page, RootPublicKey};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;

mod cloud_keeper;
pub use cloud_keeper::{CloudBackupReport, CloudPolicy, CloudRestoreReport, CLOUD_BACKUP_KEY};

pub const CLOUD_SECRET_KEY: &str = "cloud-secret";
pub const CLOUD_URL_KEY: &str = "cloud-url";
const CIDS_FILE: &str = "cloud-cids.bin";
/// Items sealed and uploaded together, so temporary ciphertext never piles up.
const BATCH: usize = 64;
/// A video larger than this is left out of a share (it would have to be stripped of its location in memory).
const MAX_VIDEO: u64 = 256 * 1024 * 1024;
const MAX_MOTION: u64 = 32 * 1024 * 1024;

/// What the sharer chooses.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WebShareOptions {
    /// Also upload full-size photos (their location removed where the format allows; withheld where not).
    pub full_size: bool,
    /// After this (ms since the epoch) the service stops serving the link.
    pub expires_ms: Option<i64>,
}

/// One link to one album, as the library remembers it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebShare {
    pub share_id: [u8; 16],
    /// The share key: the secret in the link. Kept so any device can show the link again and refresh the share.
    pub key: [u8; 32],
    pub album: AlbumId,
    pub name: String,
    pub full_size: bool,
    pub expires_ms: Option<i64>,
    pub created_ms: u64,
    /// The manifest version the service holds.
    pub version: u32,
    /// What the album looked like when this version was made; a change means the share is stale.
    pub content: [u8; 32],
    pub items: u32,
    /// Photos left out because their previews are not on this device yet.
    pub skipped: u32,
    /// Full-size originals left out because their location could not be removed.
    pub withheld: u32,
}

impl WebShare {
    pub fn link(&self, base_url: &str) -> String {
        format!(
            "{}/a#{}",
            base_url.trim_end_matches('/'),
            ShareLink::cloud(self.share_id, ShareKey(self.key)).to_fragment()
        )
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RefreshReport {
    pub refreshed: u32,
    pub current: u32,
    pub failed: Vec<String>,
}

/// Plaintext hash to the blob it became on the service.
#[derive(Default, Serialize, Deserialize)]
struct CidCache(BTreeMap<[u8; 32], ([u8; 32], u64)>);

pub(crate) struct Session {
    base: String,
    http: reqwest::Client,
    secret: CloudSecret,
    key: shoebox_core::identity::KeyPair,
    library: LibraryId,
    root: RootPublicKey,
    admin: Option<AdminCert>,
    device: DeviceCert,
    tmp: PathBuf,
    cache: std::sync::Mutex<CidCache>,
    cache_path: PathBuf,
    /// A Live share keeps its ciphertext in this folder on the serving device instead of uploading it (W1-08).
    sink: Option<PathBuf>,
}

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
        .timeout(Duration::from_secs(900))
        .user_agent(concat!("shoebox/", env!("CARGO_PKG_VERSION")))
        .build()?)
}

fn now_i64(ms: u64) -> i64 {
    i64::try_from(ms).unwrap_or(i64::MAX)
}

impl Session {
    fn auth(&self, method: &str, path: &str, body_hash: &[u8; 32], now_ms: i64) -> String {
        format!(
            "Shoebox {}",
            CloudAuth::sign_hashed(
                &self.key,
                self.library,
                self.root,
                self.admin.clone(),
                self.device.clone(),
                method,
                path,
                now_ms,
                body_hash,
            )
            .to_header()
        )
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    async fn fail(res: reqwest::Response, what: &str) -> anyhow::Error {
        let status = res.status();
        let text = res.text().await.unwrap_or_default();
        anyhow!("{what}: the service answered {status}: {text}")
    }

    /// A small signed request (body in memory).
    async fn call(
        &self,
        method: &str,
        path: &str,
        body: Vec<u8>,
        now_ms: i64,
    ) -> Result<reqwest::Response> {
        let hash = *blake3::hash(&body).as_bytes();
        let req = self
            .http
            .request(method.parse()?, self.url(path))
            .header(AUTHORIZATION, self.auth(method, path, &hash, now_ms))
            .body(body);
        req.send().await.with_context(|| format!("{method} {path}"))
    }

    async fn has(
        &self,
        cids: &[[u8; 32]],
        now_ms: i64,
    ) -> Result<std::collections::HashSet<[u8; 32]>> {
        let mut present = std::collections::HashSet::new();
        if let Some(dir) = &self.sink {
            for c in cids {
                if dir.join(hex::encode(c)).is_file() {
                    present.insert(*c);
                }
            }
            return Ok(present);
        }
        for chunk in cids.chunks(5000) {
            let ids: Vec<String> = chunk.iter().map(hex::encode).collect();
            let res = self
                .call("POST", "/v1/blobs/has", serde_json::to_vec(&ids)?, now_ms)
                .await?;
            if !res.status().is_success() {
                return Err(Self::fail(res, "asking which blobs are stored").await);
            }
            let v: serde_json::Value = serde_json::from_slice(&res.bytes().await?)?;
            for h in v["present"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|h| h.as_str())
            {
                if let Ok(b) = hex::decode(h) {
                    if let Ok(a) = b.try_into() {
                        present.insert(a);
                    }
                }
            }
        }
        Ok(present)
    }

    /// Ask where to upload blobs (`(cid, size)`) and objects (`(name, size, hash of the bytes)`). The answer is in
    /// the same order: a blob the service holds already comes back `stored`.
    async fn begin(
        &self,
        blobs: &[([u8; 32], u64)],
        objects: &[(String, u64, [u8; 32])],
        now_ms: i64,
    ) -> Result<Vec<Slot>> {
        let body = serde_json::json!({
            "blobs": blobs.iter().map(|(c, n)| serde_json::json!({ "cid": hex::encode(c), "size": n })).collect::<Vec<_>>(),
            "objects": objects.iter().map(|(name, n, h)| serde_json::json!({ "name": name, "size": n, "hash": hex::encode(h) })).collect::<Vec<_>>(),
        });
        let res = self
            .call("POST", "/v1/uploads", serde_json::to_vec(&body)?, now_ms)
            .await?;
        if !res.status().is_success() {
            return Err(Self::fail(res, "asking where to upload").await);
        }
        let v: serde_json::Value = serde_json::from_slice(&res.bytes().await?)?;
        let slots: Vec<Slot> = v["uploads"]
            .as_array()
            .context("the service did not say where to upload")?
            .iter()
            .map(|u| Slot {
                stored: u["stored"].as_bool().unwrap_or(false),
                token: u["token"].as_str().map(String::from),
                url: u["url"].as_str().map(String::from),
            })
            .collect();
        ensure!(
            slots.len() == blobs.len() + objects.len(),
            "the service answered for {} uploads, not {}",
            slots.len(),
            blobs.len() + objects.len()
        );
        Ok(slots)
    }

    /// Put the bytes where [`Session::begin`] said: the bucket (an absolute, presigned URL, which carries its own
    /// permission) or the service itself (a path).
    async fn put_to(&self, slot: &Slot, body: reqwest::Body, len: u64) -> Result<()> {
        let url = slot
            .url
            .as_deref()
            .context("the service gave no upload URL")?;
        let url = if url.starts_with('/') {
            self.url(url)
        } else {
            url.to_string()
        };
        let res = self
            .http
            .put(url)
            .header(CONTENT_LENGTH, len)
            .body(body)
            .send()
            .await
            .context("uploading")?;
        if !res.status().is_success() {
            return Err(Self::fail(res, "uploading").await);
        }
        Ok(())
    }

    /// Tell the service these uploads are in place. Returns the tokens it refused, with why.
    async fn commit(&self, tokens: &[String], now_ms: i64) -> Result<Vec<(String, String)>> {
        if tokens.is_empty() {
            return Ok(vec![]);
        }
        let body = serde_json::json!({ "tokens": tokens });
        let res = self
            .call(
                "POST",
                "/v1/uploads/commit",
                serde_json::to_vec(&body)?,
                now_ms,
            )
            .await?;
        if !res.status().is_success() {
            return Err(Self::fail(res, "finishing uploads").await);
        }
        let v: serde_json::Value = serde_json::from_slice(&res.bytes().await?)?;
        Ok(v["results"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|r| r["ok"].as_bool() != Some(true))
            .map(|r| {
                (
                    r["token"].as_str().unwrap_or_default().to_string(),
                    r["message"].as_str().unwrap_or("refused").to_string(),
                )
            })
            .collect())
    }

    fn cached(&self, plain: &Blake3Hash) -> Option<([u8; 32], u64)> {
        self.cache.lock().unwrap().0.get(&plain.0).copied()
    }

    fn remember(&self, plain: &Blake3Hash, cid: [u8; 32], size: u64) {
        let mut c = self.cache.lock().unwrap();
        c.0.insert(plain.0, (cid, size));
    }

    fn save_cache(&self) {
        if let Ok(bytes) = postcard::to_stdvec(&*self.cache.lock().unwrap()) {
            let tmp = self.cache_path.with_extension("tmp");
            if std::fs::write(&tmp, bytes).is_ok() {
                let _ = std::fs::rename(&tmp, &self.cache_path);
            }
        }
    }
}

/// Where one upload goes, as the service answered.
struct Slot {
    stored: bool,
    token: Option<String>,
    url: Option<String>,
}

/// What [`LibraryNode::upload_all`] did: the blobs that went up, those the service had, and what stopped it.
#[derive(Default)]
struct Uploaded {
    went: std::collections::HashSet<[u8; 32]>,
    held: std::collections::HashSet<[u8; 32]>,
    error: Option<anyhow::Error>,
}

/// Blobs asked about and uploaded per round trip.
const UPLOAD_ROUND: usize = 200;

/// What gets sealed: a stored blob read piece by piece, or bytes already in memory (a copy without its location).
enum Plain {
    Stored(Blake3Hash),
    Memory(Vec<u8>),
}

/// A blob on its way to the service.
struct Staged {
    cid: [u8; 32],
    size: u64,
    key: BlobKey,
    /// Ciphertext written to disk this run (absent when the cache already knew the cid).
    file: Option<PathBuf>,
    plain: Plain,
}

impl<L: LibraryStore> LibraryNode<L> {
    // ---- the cloud secret and address ----

    /// Give the library its cloud secret by typing the recovery phrase once (the root key is not kept on any device).
    /// Every device then has the secret through the library. A library made after this version has it from the start.
    pub async fn enable_cloud_with_phrase(&self, phrase: &str) -> Result<()> {
        let cap = self.capability()?;
        let root = RecoveryKit::key_from_phrase(phrase, &cap.root)?;
        self.set_cloud_secret(&CloudSecret::from_root(&root)).await
    }

    pub(crate) async fn set_cloud_secret(&self, secret: &CloudSecret) -> Result<()> {
        self.store()
            .put_setting(CLOUD_SECRET_KEY, secret.as_bytes().to_vec())
            .await?;
        Ok(())
    }

    pub async fn cloud_enabled(&self) -> Result<bool> {
        Ok(self.store().setting(CLOUD_SECRET_KEY).await?.is_some())
    }

    pub async fn set_cloud_url(&self, url: &str) -> Result<()> {
        let url = url.trim().trim_end_matches('/');
        ensure!(
            url.starts_with("https://")
                || url.starts_with("http://127.0.0.1")
                || url.starts_with("http://localhost"),
            "the cloud address must start with https://"
        );
        self.store()
            .put_setting(CLOUD_URL_KEY, url.as_bytes().to_vec())
            .await?;
        Ok(())
    }

    pub async fn cloud_url(&self) -> Result<Option<String>> {
        Ok(self
            .store()
            .setting(CLOUD_URL_KEY)
            .await?
            .and_then(|b| String::from_utf8(b).ok()))
    }

    async fn session(&self) -> Result<Session> {
        let base = self
            .cloud_url()
            .await?
            .context("no cloud address is set for this library")?;
        self.session_for(base, None).await
    }

    /// A session that seals into `dir` on this device and talks to no service (Live shares).
    pub(crate) async fn live_session(&self, dir: PathBuf) -> Result<Session> {
        self.session_for(String::new(), Some(dir)).await
    }

    async fn session_for(&self, base: String, sink: Option<PathBuf>) -> Result<Session> {
        let secret: [u8; 32] = self
            .store()
            .setting(CLOUD_SECRET_KEY)
            .await?
            .and_then(|b| b.try_into().ok())
            .context("the cloud is not enabled for this library: type the recovery phrase once to enable it")?;
        let cap = self.capability()?;
        let list = self.store().device_list().await?;
        let me = self.device();
        let device = list
            .devices
            .iter()
            .find(|d| d.device == me && !d.revoked)
            .cloned()
            .context("this device is not a member of the library")?;
        let admin = if device.added_by.0 == cap.root.0 {
            None
        } else {
            list.admins
                .iter()
                .find(|a| a.admin == device.added_by)
                .cloned()
        };
        let dir = self.node().config().data_dir;
        let tmp = dir.join("cloud-tmp");
        std::fs::create_dir_all(&tmp)?;
        // Left over from a run that stopped halfway.
        for e in std::fs::read_dir(&tmp)?.flatten() {
            let _ = std::fs::remove_file(e.path());
        }
        let cache_path = dir.join(CIDS_FILE);
        let cache = std::fs::read(&cache_path)
            .ok()
            .and_then(|b| postcard::from_bytes(&b).ok())
            .unwrap_or_default();
        Ok(Session {
            base,
            http: client()?,
            secret: CloudSecret::from_bytes(secret),
            key: self.node().device_key(),
            library: cap.identity_id(),
            root: cap.root,
            admin,
            device,
            tmp,
            cache: std::sync::Mutex::new(cache),
            cache_path,
            sink,
        })
    }

    /// Tell the service about this library's certificates, revocations above all, so a revoked device stops being
    /// served. Called after a device is revoked; harmless to call again.
    pub async fn push_cloud_certs(&self) -> Result<()> {
        if !self.cloud_enabled().await? || self.cloud_url().await?.is_none() {
            return Ok(());
        }
        let s = self.session().await?;
        let list = self.store().device_list().await?;
        let body = postcard::to_stdvec(&CertBundle {
            admins: list.admins,
            devices: list.devices,
        })?;
        let res = s
            .call("POST", "/v1/library/certs", body, now_i64(self.now_ms()))
            .await?;
        ensure!(
            res.status().is_success(),
            "sending certificates: {}",
            res.status()
        );
        Ok(())
    }

    /// [`LibraryNode::push_cloud_certs`] without waiting for it: a revocation must not hang on an offline service.
    /// Refreshing shares pushes again, so a push that failed is retried.
    pub(crate) fn push_cloud_certs_in_background(&self) {
        let me = self.clone();
        tokio::spawn(async move {
            if let Err(e) = me.push_cloud_certs().await {
                tracing::warn!("could not tell the cloud about a revocation: {e:#}");
            }
        });
    }

    // ---- shares ----

    fn shares_key(album: &AlbumId) -> String {
        format!("web-shares-{}", hex::encode(album.0))
    }

    /// The links of one album.
    pub async fn web_shares(&self, album: &AlbumId) -> Result<Vec<WebShare>> {
        Ok(self
            .store()
            .setting(&Self::shares_key(album))
            .await?
            .and_then(|b| postcard::from_bytes(&b).ok())
            .unwrap_or_default())
    }

    async fn save_web_shares(&self, album: &AlbumId, shares: &[WebShare]) -> Result<()> {
        self.store()
            .put_setting(&Self::shares_key(album), postcard::to_stdvec(shares)?)
            .await?;
        Ok(())
    }

    /// The text to give someone for `share`.
    pub async fn web_share_link(&self, share: &WebShare) -> Result<String> {
        Ok(share.link(&self.cloud_url().await?.context("no cloud address is set")?))
    }

    /// Put an album on the web: upload what is missing, seal a manifest, and return the link's record.
    pub async fn create_web_share(
        &self,
        album: &AlbumId,
        options: WebShareOptions,
    ) -> Result<WebShare> {
        let s = self.session().await?;
        let mut id = [0u8; 16];
        getrandom::fill(&mut id).map_err(|e| anyhow!("randomness: {e}"))?;
        let share = self
            .build_share(
                &s,
                album,
                id,
                ShareKey::generate(),
                &options,
                0,
                self.now_ms(),
            )
            .await?;
        let mut shares = self.web_shares(album).await?;
        shares.push(share.clone());
        self.save_web_shares(album, &shares).await?;
        Ok(share)
    }

    /// Change what an existing link offers (full-size downloads, its end date) without changing the link: the same
    /// address and key keep working, with a new manifest version.
    pub async fn update_web_share(
        &self,
        album: &AlbumId,
        share_id: &[u8; 16],
        options: WebShareOptions,
    ) -> Result<WebShare> {
        let mut shares = self.web_shares(album).await?;
        let i = shares
            .iter()
            .position(|x| &x.share_id == share_id)
            .context("no such link")?;
        let old = shares[i].clone();
        let s = self.session().await?;
        let new = self
            .build_share(
                &s,
                album,
                old.share_id,
                ShareKey(old.key),
                &options,
                old.version + 1,
                old.created_ms,
            )
            .await?;
        shares[i] = new.clone();
        self.save_web_shares(album, &shares).await?;
        Ok(new)
    }

    /// Stop a link now: the service deletes it, and the library forgets it.
    pub async fn revoke_web_share(&self, album: &AlbumId, share_id: &[u8; 16]) -> Result<()> {
        let s = self.session().await?;
        let path = format!("/v1/shares/{}", hex::encode(share_id));
        let res = s
            .call("DELETE", &path, vec![], now_i64(self.now_ms()))
            .await?;
        // Already gone is fine.
        ensure!(
            res.status().is_success() || res.status() == reqwest::StatusCode::NOT_FOUND,
            "revoking: the service answered {}",
            res.status()
        );
        let mut shares = self.web_shares(album).await?;
        shares.retain(|x| &x.share_id != share_id);
        self.save_web_shares(album, &shares).await
    }

    /// A new link in place of the old one (new key and id); the old link stops.
    pub async fn rotate_web_share(&self, album: &AlbumId, share_id: &[u8; 16]) -> Result<WebShare> {
        let old = self
            .web_shares(album)
            .await?
            .into_iter()
            .find(|x| &x.share_id == share_id)
            .context("no such link")?;
        let new = self
            .create_web_share(
                album,
                WebShareOptions {
                    full_size: old.full_size,
                    expires_ms: old.expires_ms,
                },
            )
            .await?;
        self.revoke_web_share(album, share_id).await?;
        Ok(new)
    }

    /// Bring every link up to date with its album: those whose album changed get new blobs and a new manifest
    /// version. Any member device can run this; a keeper does it by itself.
    pub async fn refresh_web_shares(&self) -> Result<RefreshReport> {
        let mut report = RefreshReport::default();
        if !self.cloud_enabled().await? || self.cloud_url().await?.is_none() {
            return Ok(report);
        }
        // The service must know who has been revoked before it serves their next request.
        if let Err(e) = self.push_cloud_certs().await {
            report.failed.push(format!("certificates: {e:#}"));
        }
        let mut session: Option<Session> = None;
        for summary in self.store().albums().await? {
            let shares = self.web_shares(&summary.id).await?;
            if shares.is_empty() {
                continue;
            }
            let mut updated = shares.clone();
            for (i, share) in shares.iter().enumerate() {
                let opts = WebShareOptions {
                    full_size: share.full_size,
                    expires_ms: share.expires_ms,
                };
                if self.album_fingerprint(&summary.id, &opts).await? == share.content {
                    report.current += 1;
                    continue;
                }
                if session.is_none() {
                    session = Some(self.session().await?);
                }
                let s = session.as_ref().expect("just set");
                match self
                    .build_share(
                        s,
                        &summary.id,
                        share.share_id,
                        ShareKey(share.key),
                        &opts,
                        share.version + 1,
                        share.created_ms,
                    )
                    .await
                {
                    Ok(new) => {
                        updated[i] = new;
                        report.refreshed += 1;
                    }
                    Err(e) => report.failed.push(format!("{}: {e:#}", share.name)),
                }
            }
            if updated != shares {
                self.save_web_shares(&summary.id, &updated).await?;
            }
        }
        Ok(report)
    }

    /// Links the service switched off because someone reported them (W1-10): share id and the reporter's words.
    /// Empty when the service cannot be reached; the sheet then simply shows nothing.
    pub async fn reported_web_shares(&self) -> Vec<([u8; 16], String)> {
        let Ok(s) = self.session().await else {
            return vec![];
        };
        let Ok(res) = s
            .call("GET", "/v1/shares", vec![], now_i64(self.now_ms()))
            .await
        else {
            return vec![];
        };
        let Ok(bytes) = res.bytes().await else {
            return vec![];
        };
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
        v["shares"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|x| {
                let id: [u8; 16] = hex::decode(x["shareId"].as_str()?).ok()?.try_into().ok()?;
                let why = x["reported"]["reason"].as_str()?.to_string();
                Some((id, why))
            })
            .collect()
    }

    /// Has the album changed since this version of the share was made?
    pub async fn web_share_is_stale(&self, share: &WebShare) -> Result<bool> {
        let opts = WebShareOptions {
            full_size: share.full_size,
            expires_ms: share.expires_ms,
        };
        Ok(self.album_fingerprint(&share.album, &opts).await? != share.content)
    }

    /// What an album holds, as far as a share cares: its photos and which of them are deleted, and the options.
    pub(crate) async fn album_fingerprint(
        &self,
        album: &AlbumId,
        opts: &WebShareOptions,
    ) -> Result<[u8; 32]> {
        let mut ids = self.store().album_items(album, Page::all()).await?;
        ids.sort();
        let mut h = blake3::Hasher::new();
        h.update(&[u8::from(opts.full_size)]);
        for id in ids {
            let deleted = self
                .store()
                .photo(&id)
                .await?
                .is_none_or(|p| p.deleted_at.value.is_some());
            h.update(&id.0);
            h.update(&[u8::from(deleted)]);
        }
        Ok(*h.finalize().as_bytes())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn build_share(
        &self,
        s: &Session,
        album: &AlbumId,
        share_id: [u8; 16],
        key: ShareKey,
        opts: &WebShareOptions,
        version: u32,
        created_ms: u64,
    ) -> Result<WebShare> {
        let name = self
            .store()
            .albums()
            .await?
            .into_iter()
            .find(|a| &a.id == album)
            .map(|a| a.name)
            .context("no such album")?;
        let content = self.album_fingerprint(album, opts).await?;
        let mut ids = self.store().album_items(album, Page::all()).await?;
        ids.sort();

        let mut items = Vec::new();
        let mut cids: Vec<[u8; 32]> = Vec::new();
        let (mut skipped, mut withheld) = (0u32, 0u32);
        let mut records = Vec::new();
        for id in ids {
            if let Some(p) = self.store().photo(&id).await? {
                if p.deleted_at.value.is_none() {
                    records.push(p);
                }
            }
        }
        records.sort_by_key(|p| (p.taken_at, p.id));
        for batch in records.chunks(BATCH) {
            let (built, sk, wh, batch_cids) = self.stage_batch(s, batch, opts).await?;
            items.extend(built);
            skipped += sk;
            withheld += wh;
            cids.extend(batch_cids);
        }
        cids.sort();
        cids.dedup();

        let manifest = Manifest {
            album_name: name.clone(),
            items,
            expires: opts.expires_ms.map(Timestamp),
            inbox_key: None,
        };
        let sealed = share::seal(&manifest, share_id, version, &key, &key.signer());
        let upload = postcard::to_stdvec(&ShareUpload {
            manifest: sealed,
            cids,
            expires_ms: opts.expires_ms,
        })?;
        if let Some(dir) = &s.sink {
            let up: ShareUpload = postcard::from_bytes(&upload)?;
            crate::web_live::store_manifest(dir, album, &up)?;
        } else {
            let path = format!("/v1/shares/{}", hex::encode(share_id));
            let res = s.call("PUT", &path, upload, now_i64(self.now_ms())).await?;
            if !res.status().is_success() {
                return Err(Session::fail(res, "putting the share on the service").await);
            }
        }
        s.save_cache();
        Ok(WebShare {
            share_id,
            key: key.0,
            album: *album,
            name,
            full_size: opts.full_size,
            expires_ms: opts.expires_ms,
            created_ms,
            version,
            content,
            items: u32::try_from(manifest.items.len()).unwrap_or(u32::MAX),
            skipped,
            withheld,
        })
    }

    /// Seal and upload the blobs of up to [`BATCH`] photos, returning their manifest items.
    async fn stage_batch(
        &self,
        s: &Session,
        batch: &[PhotoRecord],
        opts: &WebShareOptions,
    ) -> Result<(Vec<Item>, u32, u32, Vec<[u8; 32]>)> {
        struct Pending {
            record: PhotoRecord,
            thumb: Staged,
            preview: Staged,
            original: Option<Staged>,
            motion: Option<Staged>,
        }
        let blobs = self.node().blobs();
        let mut pending = Vec::new();
        let (mut skipped, mut withheld) = (0u32, 0u32);
        for p in batch {
            let is_video = p.original.mime.starts_with("video/");
            // A device that keeps no preview next to its original makes it again for the link.
            if !is_video && !blobs.has(&p.preview.hash).await? {
                let _ = self.ensure_preview(p).await;
            }
            // Thumbnails and previews are on every device once it has synced; one that is not yet is skipped,
            // and the link says how many.
            if !(blobs.has(&p.thumb.hash).await? && blobs.has(&p.preview.hash).await?) && !is_video
            {
                skipped += 1;
                continue;
            }
            let thumb = match self
                .stage(s, Plain::Stored(p.thumb.hash), p.exif.gps.is_some())
                .await
            {
                Ok(t) => t,
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            };
            let preview = match self
                .stage(s, Plain::Stored(p.preview.hash), p.exif.gps.is_some())
                .await
            {
                Ok(t) => t,
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            };
            // A video has to be played from its original; a photo's original only when the sharer allowed it.
            let want_original = opts.full_size || is_video;
            let mut original = None;
            if want_original {
                if is_video && p.original.size > MAX_VIDEO {
                    withheld += 1;
                } else if !blobs.has(&p.original.hash).await? {
                    // Not on this device; a keeper's refresh adds it.
                } else {
                    match self.stage_original(s, p).await? {
                        Some(o) => original = Some(o),
                        None => withheld += 1,
                    }
                }
            }
            let motion = match self.store().motion(&p.id).await? {
                Some(m) if m.video.size <= MAX_MOTION && self.clip_available(&p.id).await? => {
                    // A motion photo's clip is stripped like any original.
                    self.stage_motion(s, p, &m.video.hash).await?
                }
                _ => None,
            };
            pending.push(Pending {
                record: p.clone(),
                thumb,
                preview,
                original,
                motion,
            });
        }
        // Which of these does the service already hold?
        let all: Vec<&Staged> = pending
            .iter()
            .flat_map(|p| {
                [
                    Some(&p.thumb),
                    Some(&p.preview),
                    p.original.as_ref(),
                    p.motion.as_ref(),
                ]
                .into_iter()
                .flatten()
            })
            .collect();
        let asked: Vec<[u8; 32]> = all.iter().map(|b| b.cid).collect();
        let up = self.upload_all(s, &all).await;
        if let Some(e) = up.error {
            return Err(e);
        }
        for b in &all {
            if let Some(f) = &b.file {
                let _ = std::fs::remove_file(f);
            }
        }
        let blob = |b: &Staged| CloudBlob {
            cid: Blake3Hash(b.cid),
            key: b.key.0,
            size: b.size,
        };
        let items = pending
            .iter()
            .map(|p| Item {
                kind: if p.record.original.mime.starts_with("video/") {
                    ItemKind::Video
                } else {
                    ItemKind::Photo
                },
                taken: p.record.taken_at,
                width: p.record.exif.width,
                height: p.record.exif.height,
                orientation: 1,
                thumb: blob(&p.thumb),
                preview: blob(&p.preview),
                original: p.original.as_ref().map(blob),
                motion: p.motion.as_ref().map(blob),
            })
            .collect();
        Ok((items, skipped, withheld, asked))
    }

    /// An original, without its location. `None` when that cannot be done for this kind of file (it is withheld).
    async fn stage_original(&self, s: &Session, p: &PhotoRecord) -> Result<Option<Staged>> {
        self.stage_stripped(
            s,
            &p.original.hash,
            p.exif.gps.is_some() || p.original.mime.starts_with("video/"),
        )
        .await
    }

    async fn stage_motion(
        &self,
        s: &Session,
        p: &PhotoRecord,
        hash: &Blake3Hash,
    ) -> Result<Option<Staged>> {
        // A clip read from the photo's original is staged from memory.
        if !self.node().blobs().has(hash).await? {
            let Some(bytes) = self.clip_bytes(&p.id).await? else {
                return Ok(None);
            };
            return match shoebox_media::strip_location(&bytes) {
                Ok(stripped) => Ok(Some(
                    self.stage(s, Plain::Memory(stripped.bytes), false).await?,
                )),
                Err(_) => Ok(None),
            };
        }
        self.stage_stripped(s, hash, true).await
    }

    async fn stage_stripped(
        &self,
        s: &Session,
        hash: &Blake3Hash,
        may_have_location: bool,
    ) -> Result<Option<Staged>> {
        if !may_have_location {
            return Ok(Some(self.stage(s, Plain::Stored(*hash), false).await?));
        }
        let bytes = self.node().blobs().get(hash).await?;
        match shoebox_media::strip_location(&bytes) {
            Ok(stripped) if !stripped.changed => {
                Ok(Some(self.stage(s, Plain::Stored(*hash), false).await?))
            }
            Ok(stripped) => Ok(Some(
                self.stage(s, Plain::Memory(stripped.bytes), false).await?,
            )),
            Err(_) => Ok(None),
        }
    }

    /// Work out a blob's id and key, sealing it to a temporary file unless the cache knows the id already.
    async fn stage(&self, s: &Session, plain: Plain, check_metadata: bool) -> Result<Staged> {
        let plain_hash = match &plain {
            Plain::Stored(h) => *h,
            Plain::Memory(b) => Blake3Hash::of_bytes(b),
        };
        // A stored thumbnail or preview of a photo with a place must carry none (they are re-encoded, so they do
        // not; this makes sure of it for the one thing that must not leak).
        let plain = if check_metadata {
            if let Plain::Stored(h) = &plain {
                let bytes = self.node().blobs().get(h).await?;
                match shoebox_media::strip_location(&bytes) {
                    Ok(st) if st.changed => Plain::Memory(st.bytes),
                    Ok(_) => plain,
                    Err(_) => bail!("cannot check a derived image for a location"),
                }
            } else {
                plain
            }
        } else {
            plain
        };
        let plain_hash = match &plain {
            Plain::Memory(b) => Blake3Hash::of_bytes(b),
            Plain::Stored(_) => plain_hash,
        };
        let key = s.secret.blob_key(&plain_hash);
        if let Some((cid, size)) = s.cached(&plain_hash) {
            return Ok(Staged {
                cid,
                size,
                key,
                file: None,
                plain,
            });
        }
        let (cid, size, file) = self.seal_to_file(s, &key, &plain).await?;
        s.remember(&plain_hash, cid, size);
        Ok(Staged {
            cid,
            size,
            key,
            file: Some(file),
            plain,
        })
    }

    async fn seal_to_file(
        &self,
        s: &Session,
        key: &BlobKey,
        plain: &Plain,
    ) -> Result<([u8; 32], u64, PathBuf)> {
        let mut r = [0u8; 8];
        getrandom::fill(&mut r).map_err(|e| anyhow!("randomness: {e}"))?;
        let path = s.tmp.join(hex::encode(r));
        let mut file = tokio::fs::File::create(&path).await?;
        let mut hasher = blake3::Hasher::new();
        let mut written = 0u64;
        let result: Result<()> = async {
            let total = match plain {
                Plain::Stored(h) => self
                    .node()
                    .blobs()
                    .size(h)
                    .await?
                    .context("blob is not stored")?,
                Plain::Memory(b) => b.len() as u64,
            };
            let pieces = total.div_ceil(CHUNK as u64).max(1);
            for i in 0..pieces {
                let (a, b) = (i * CHUNK as u64, ((i + 1) * CHUNK as u64).min(total));
                let piece: Vec<u8> = match plain {
                    Plain::Stored(h) => self.node().blobs().open_range(h, a..b).await?.to_vec(),
                    Plain::Memory(m) => m[a as usize..b as usize].to_vec(),
                };
                ensure!(
                    piece.len() as u64 == b - a,
                    "a blob was shorter than its size"
                );
                let sealed = key.seal_piece(i, i + 1 == pieces, &piece);
                debug_assert_eq!(sealed.len(), piece.len() + TAG);
                hasher.update(&sealed);
                file.write_all(&sealed).await?;
                written += sealed.len() as u64;
            }
            file.flush().await?;
            Ok(())
        }
        .await;
        if let Err(e) = result {
            let _ = std::fs::remove_file(&path);
            return Err(e);
        }
        Ok((*hasher.finalize().as_bytes(), written, path))
    }

    /// Upload the blobs the service (or a Live share's folder) does not hold. Whatever went up before a failure is
    /// committed and reported as gone up.
    async fn upload_all(&self, s: &Session, items: &[&Staged]) -> Uploaded {
        let mut out = Uploaded::default();
        if let Some(dir) = &s.sink {
            let asked: Vec<[u8; 32]> = items.iter().map(|b| b.cid).collect();
            let result = async {
                let present = s.has(&asked, 0).await?;
                std::fs::create_dir_all(dir)?;
                for b in items {
                    if present.contains(&b.cid) {
                        out.held.insert(b.cid);
                        continue;
                    }
                    let (path, temp) = self.sealed_file(s, b).await?;
                    let moved = std::fs::rename(&path, dir.join(hex::encode(b.cid)));
                    if temp {
                        let _ = std::fs::remove_file(&path);
                    }
                    moved?;
                    out.went.insert(b.cid);
                }
                Ok(())
            }
            .await;
            out.error = result.err();
            return out;
        }
        for round in items.chunks(UPLOAD_ROUND) {
            let wanted: Vec<([u8; 32], u64)> = round.iter().map(|b| (b.cid, b.size)).collect();
            let slots = match s.begin(&wanted, &[], now_i64(self.now_ms())).await {
                Ok(slots) => slots,
                Err(e) => {
                    out.error = Some(e);
                    return out;
                }
            };
            let mut put: Vec<(String, [u8; 32])> = vec![];
            let mut failed = None;
            for (b, slot) in round.iter().zip(&slots) {
                if slot.stored {
                    out.held.insert(b.cid);
                    continue;
                }
                let sent = async {
                    let (path, temp) = self.sealed_file(s, b).await?;
                    let file = tokio::fs::File::open(&path).await;
                    let result = match file {
                        Ok(f) => {
                            let body = reqwest::Body::wrap_stream(
                                tokio_util::io::ReaderStream::with_capacity(f, 256 * 1024),
                            );
                            s.put_to(slot, body, b.size).await
                        }
                        Err(e) => Err(e.into()),
                    };
                    if temp {
                        let _ = std::fs::remove_file(&path);
                    }
                    result
                }
                .await;
                match sent {
                    Ok(()) => put.push((slot.token.clone().unwrap_or_default(), b.cid)),
                    Err(e) => {
                        failed = Some(e);
                        break;
                    }
                }
            }
            let tokens: Vec<String> = put.iter().map(|(t, _)| t.clone()).collect();
            match s.commit(&tokens, now_i64(self.now_ms())).await {
                Ok(refused) => {
                    for (t, c) in &put {
                        if !refused.iter().any(|(r, _)| r == t) {
                            out.went.insert(*c);
                        }
                    }
                    if let Some((_, why)) = refused.first() {
                        failed
                            .get_or_insert_with(|| anyhow!("the service refused an upload: {why}"));
                    }
                }
                Err(e) => {
                    failed.get_or_insert(e);
                }
            }
            if let Some(e) = failed {
                out.error = Some(e);
                return out;
            }
        }
        out
    }

    /// The sealed file of a staged blob, sealing it again when the cache knew its id but the file is gone. The flag
    /// says the file was made here, for the caller to delete.
    async fn sealed_file(&self, s: &Session, b: &Staged) -> Result<(PathBuf, bool)> {
        match &b.file {
            Some(f) => Ok((f.clone(), false)),
            None => {
                let (cid, _, f) = self.seal_to_file(s, &b.key, &b.plain).await?;
                ensure!(cid == b.cid, "a blob sealed differently than before");
                Ok((f, true))
            }
        }
    }
}
