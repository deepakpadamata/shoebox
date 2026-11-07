//! Live web links (W1-08): the free mode. The link holds the share id, the endpoint ids of the devices that may
//! serve it and the key; the ciphertext (the same sealed blobs and manifest Shoebox Cloud would hold) sits in a
//! folder on the serving device, and a browser asks for it over iroh with `shoebox/webview/0`. Nothing is stored
//! with us, and the link works only while one of those devices is online: the sharer chose that.
//!
//! A serving device answers for the shares whose folder it holds and for nothing else: no other album, no member
//! list, no record. A folder is only served while the share is still in the library (revoking removes it) and
//! before it expires.
//!
//! Wire format, one request per bidirectional stream:
//!
//! ```text
//! request   op u8 (1 manifest, 2 blob) · share id [16] · (blob: cid [32] · offset u64 LE · length u32 LE)
//! response  status u8 (0 ok, 1 not served, 2 expired, 3 busy, 4 bad request) · (ok: file size u64 LE · bytes)
//! ```

use crate::library_node::Inner;
use crate::web_share::{WebShare, WebShareOptions};
use crate::LibraryNode;
use anyhow::{Context, Result};
use futures::future::BoxFuture;
use iroh::endpoint::Connection;
use serde::{Deserialize, Serialize};
use shoebox_core::model::AlbumId;
use shoebox_core::share::{ShareKey, ShareLink, ShareUpload};
use shoebox_core::store::LibraryStore;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

const MANIFEST: &str = "manifest";
const META: &str = "meta";
/// The most one response carries; a browser asks again for the rest.
pub const MAX_RANGE: u32 = 8 * 1024 * 1024;
/// Requests per share per minute, and streams answered at once.
const PER_MINUTE: u32 = 1500;
const AT_ONCE: usize = 16;

pub const OP_MANIFEST: u8 = 1;
pub const OP_BLOB: u8 = 2;
pub const OK: u8 = 0;
pub const NOT_SERVED: u8 = 1;
pub const EXPIRED: u8 = 2;
pub const BUSY: u8 = 3;
pub const BAD: u8 = 4;

#[derive(Serialize, Deserialize)]
struct Meta {
    album: AlbumId,
    expires_ms: Option<i64>,
}

/// Where a Live share's ciphertext sits on this device.
pub(crate) fn live_dir(data_dir: &Path, share_id: &[u8; 16]) -> PathBuf {
    data_dir.join("live").join(hex::encode(share_id))
}

/// Called by a share's build once its blobs are in `dir`: keep the manifest and what it serves, drop the rest.
pub(crate) fn store_manifest(dir: &Path, album: &AlbumId, up: &ShareUpload) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let keep: std::collections::HashSet<String> = up.cids.iter().map(hex::encode).collect();
    for e in std::fs::read_dir(dir)?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name != MANIFEST && name != META && !keep.contains(&name) {
            let _ = std::fs::remove_file(e.path());
        }
    }
    let write = |name: &str, bytes: &[u8]| -> Result<()> {
        let tmp = dir.join(format!("{name}.tmp"));
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, dir.join(name))?;
        Ok(())
    };
    write(MANIFEST, &up.manifest)?;
    write(
        META,
        &postcard::to_stdvec(&Meta {
            album: *album,
            expires_ms: up.expires_ms,
        })?,
    )
}

fn live_key(album: &AlbumId) -> String {
    format!("web-live-{}", hex::encode(album.0))
}

fn servers_key(share_id: &[u8; 16]) -> String {
    format!("web-live-at-{}", hex::encode(share_id))
}

/// What a sync of this device's Live shares did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LiveReport {
    /// Shares this device started serving (it had no folder for them).
    pub adopted: u32,
    /// Shares whose album changed and were sealed again.
    pub refreshed: u32,
    pub current: u32,
    pub failed: Vec<String>,
}

struct Gate {
    at_once: Arc<tokio::sync::Semaphore>,
    seen: Mutex<HashMap<[u8; 16], (Instant, u32)>>,
}

impl Gate {
    fn allow(&self, id: [u8; 16]) -> bool {
        let mut seen = self.seen.lock().unwrap();
        let e = seen.entry(id).or_insert((Instant::now(), 0));
        if e.0.elapsed() > Duration::from_secs(60) {
            *e = (Instant::now(), 0);
        }
        e.1 += 1;
        e.1 <= PER_MINUTE
    }
}

struct Responder<L: LibraryStore> {
    inner: Weak<Inner<L>>,
    gate: Arc<Gate>,
}

impl<L: LibraryStore> crate::ext::ConnHandler for Responder<L> {
    fn handle(&self, conn: Connection) -> BoxFuture<'static, Result<()>> {
        let inner = self.inner.upgrade();
        let gate = self.gate.clone();
        let permits = gate.at_once.clone();
        Box::pin(async move {
            let Some(inner) = inner else { return Ok(()) };
            let ln = LibraryNode { inner };
            // A browser sends many small requests over one connection.
            while let Ok((mut send, mut recv)) = conn.accept_bi().await {
                let req = match recv.read_to_end(64).await {
                    Ok(r) => r,
                    Err(_) => break,
                };
                let Ok(permit) = permits.clone().try_acquire_owned() else {
                    send.write_all(&[BUSY]).await?;
                    send.finish()?;
                    continue;
                };
                let reply = ln.answer_webview(&req, &gate).await;
                drop(permit);
                match reply {
                    Ok(Reply::Bytes(status, total, path, offset, len)) => {
                        send.write_all(&[status]).await?;
                        send.write_all(&total.to_le_bytes()).await?;
                        let mut f = tokio::fs::File::open(&path).await?;
                        f.seek(std::io::SeekFrom::Start(offset)).await?;
                        let mut left = len;
                        let mut buf = vec![0u8; 64 * 1024];
                        while left > 0 {
                            let n = f.read(&mut buf[..(left as usize).min(64 * 1024)]).await?;
                            if n == 0 {
                                break;
                            }
                            send.write_all(&buf[..n]).await?;
                            left -= n as u64;
                        }
                    }
                    Ok(Reply::Status(s)) => send.write_all(&[s]).await?,
                    Err(_) => send.write_all(&[NOT_SERVED]).await?,
                }
                send.finish()?;
            }
            let _ = tokio::time::timeout(Duration::from_secs(2), conn.closed()).await;
            Ok(())
        })
    }
}

enum Reply {
    Status(u8),
    /// status, file size, file, offset, length
    Bytes(u8, u64, PathBuf, u64, u64),
}

impl<L: LibraryStore> LibraryNode<L> {
    pub(crate) fn start_webview_support(&self) {
        let responder: Arc<dyn crate::ext::ConnHandler> = Arc::new(Responder {
            inner: Arc::downgrade(&self.inner),
            gate: Arc::new(Gate {
                at_once: Arc::new(tokio::sync::Semaphore::new(AT_ONCE)),
                seen: Mutex::new(HashMap::new()),
            }),
        });
        *self.node().webview_slot().lock().unwrap() = Some(responder);
    }

    async fn answer_webview(&self, req: &[u8], gate: &Gate) -> Result<Reply> {
        let Some((&op, rest)) = req.split_first() else {
            return Ok(Reply::Status(BAD));
        };
        let Some(share_id) = rest.get(..16).and_then(|b| <[u8; 16]>::try_from(b).ok()) else {
            return Ok(Reply::Status(BAD));
        };
        if !gate.allow(share_id) {
            return Ok(Reply::Status(BUSY));
        }
        let dir = live_dir(&self.node().config().data_dir, &share_id);
        let Some(meta) = std::fs::read(dir.join(META))
            .ok()
            .and_then(|b| postcard::from_bytes::<Meta>(&b).ok())
        else {
            return Ok(Reply::Status(NOT_SERVED));
        };
        // Revoked anywhere in the library: the record is gone, and so is the folder.
        if !self
            .web_live_shares(&meta.album)
            .await?
            .iter()
            .any(|s| s.share_id == share_id)
        {
            let _ = std::fs::remove_dir_all(&dir);
            return Ok(Reply::Status(NOT_SERVED));
        }
        if meta
            .expires_ms
            .is_some_and(|e| i64::try_from(self.now_ms()).unwrap_or(i64::MAX) >= e)
        {
            return Ok(Reply::Status(EXPIRED));
        }
        match op {
            OP_MANIFEST => {
                let path = dir.join(MANIFEST);
                let total = std::fs::metadata(&path)?.len();
                Ok(Reply::Bytes(OK, total, path, 0, total))
            }
            OP_BLOB => {
                let (Some(cid), Some(off), Some(len)) = (
                    rest.get(16..48),
                    rest.get(48..56)
                        .and_then(|b| <[u8; 8]>::try_from(b).ok())
                        .map(u64::from_le_bytes),
                    rest.get(56..60)
                        .and_then(|b| <[u8; 4]>::try_from(b).ok())
                        .map(u32::from_le_bytes),
                ) else {
                    return Ok(Reply::Status(BAD));
                };
                let path = dir.join(hex::encode(cid));
                // `cid` is hex-encoded, so it cannot name anything outside the folder.
                let Ok(total) = std::fs::metadata(&path).map(|m| m.len()) else {
                    return Ok(Reply::Status(NOT_SERVED));
                };
                let len = u64::from(len.min(MAX_RANGE)).min(total.saturating_sub(off));
                Ok(Reply::Bytes(OK, total, path, off, len))
            }
            _ => Ok(Reply::Status(BAD)),
        }
    }

    // ---- the sharer's side ----

    /// The Live links of one album.
    pub async fn web_live_shares(&self, album: &AlbumId) -> Result<Vec<WebShare>> {
        Ok(self
            .store()
            .setting(&live_key(album))
            .await?
            .and_then(|b| postcard::from_bytes(&b).ok())
            .unwrap_or_default())
    }

    async fn save_live_shares(&self, album: &AlbumId, shares: &[WebShare]) -> Result<()> {
        self.store()
            .put_setting(&live_key(album), postcard::to_stdvec(shares)?)
            .await?;
        Ok(())
    }

    /// Make a Live link: seal the album into a folder on this device, which starts serving it. The link names this
    /// device and the library's other devices (up to four), and any of them that also holds the folder can answer.
    pub async fn create_live_share(
        &self,
        album: &AlbumId,
        options: WebShareOptions,
    ) -> Result<WebShare> {
        options.check(true, self.now_ms())?;
        let mut id = [0u8; 16];
        getrandom::fill(&mut id).map_err(|e| anyhow::anyhow!("randomness: {e}"))?;
        let dir = live_dir(&self.node().config().data_dir, &id);
        let s = self.live_session(dir.clone()).await?;
        let key = ShareKey::generate();
        let built = self
            .build_share(&s, album, id, key, &options, 0, self.now_ms())
            .await;
        let share = match built {
            Ok(s) => s,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&dir);
                return Err(e);
            }
        };
        let mut servers = vec![self.device().0];
        for d in self.store().device_list().await?.devices {
            if !d.revoked && d.device != self.device() && servers.len() < 4 {
                servers.push(d.device.0);
            }
        }
        self.store()
            .put_setting(&servers_key(&id), postcard::to_stdvec(&servers)?)
            .await?;
        let mut shares = self.web_live_shares(album).await?;
        shares.push(share.clone());
        self.save_live_shares(album, &shares).await?;
        Ok(share)
    }

    /// Change what a Live link offers without changing the link: this device seals the album again under the same
    /// share id and key.
    pub async fn update_live_share(
        &self,
        album: &AlbumId,
        share_id: &[u8; 16],
        options: WebShareOptions,
    ) -> Result<WebShare> {
        options.check(true, self.now_ms())?;
        let mut shares = self.web_live_shares(album).await?;
        let i = shares
            .iter()
            .position(|x| &x.share_id == share_id)
            .context("no such link")?;
        let old = shares[i].clone();
        let dir = live_dir(&self.node().config().data_dir, share_id);
        let s = self.live_session(dir).await?;
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
        self.save_live_shares(album, &shares).await?;
        Ok(new)
    }

    /// The text to give someone for a Live share. It opens on the service's viewer page, which holds nothing.
    pub async fn live_share_link(&self, share: &WebShare) -> Result<String> {
        let base = self
            .cloud_url()
            .await?
            .context("no viewer address is set: a Live link opens on the Shoebox viewer page")?;
        let servers: Vec<[u8; 32]> = self
            .store()
            .setting(&servers_key(&share.share_id))
            .await?
            .and_then(|b| postcard::from_bytes(&b).ok())
            .unwrap_or_default();
        Ok(format!(
            "{}/a#{}",
            base.trim_end_matches('/'),
            ShareLink::live(share.share_id, &servers, ShareKey(share.key)).to_fragment()
        ))
    }

    /// Stop a Live link: the library forgets it and this device deletes its folder. Another device that served it
    /// does the same at its next request.
    pub async fn revoke_live_share(&self, album: &AlbumId, share_id: &[u8; 16]) -> Result<()> {
        let mut shares = self.web_live_shares(album).await?;
        shares.retain(|x| &x.share_id != share_id);
        self.save_live_shares(album, &shares).await?;
        let _ = std::fs::remove_dir_all(live_dir(&self.node().config().data_dir, share_id));
        Ok(())
    }

    /// Bring this device's Live shares up to date: seal again those whose album changed, and with `adopt`, start
    /// serving those that other devices made (a keeper's `album serve`).
    pub async fn sync_live_shares(&self, adopt: bool) -> Result<LiveReport> {
        let mut report = LiveReport::default();
        let data_dir = self.node().config().data_dir;
        for summary in self.store().albums().await? {
            let shares = self.web_live_shares(&summary.id).await?;
            let mut updated = shares.clone();
            for (i, share) in shares.iter().enumerate() {
                let dir = live_dir(&data_dir, &share.share_id);
                let here = dir.join(META).is_file();
                if !here && !adopt {
                    continue;
                }
                let opts = WebShareOptions {
                    full_size: share.full_size,
                    expires_ms: share.expires_ms,
                };
                let stale = self.album_fingerprint(&summary.id, &opts).await? != share.content;
                if here && !stale {
                    report.current += 1;
                    continue;
                }
                let s = self.live_session(dir.clone()).await?;
                match self
                    .build_share(
                        &s,
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
                        if here {
                            report.refreshed += 1;
                        } else {
                            report.adopted += 1;
                        }
                    }
                    Err(e) => report.failed.push(format!("{}: {e:#}", share.name)),
                }
            }
            if updated != shares {
                self.save_live_shares(&summary.id, &updated).await?;
            }
        }
        Ok(report)
    }

    /// Is a device of this library answering for this share right now? (Only this device can be checked from here.)
    pub fn serves_live_share(&self, share_id: &[u8; 16]) -> bool {
        live_dir(&self.node().config().data_dir, share_id)
            .join(META)
            .is_file()
    }
}
