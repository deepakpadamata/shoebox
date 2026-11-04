//! The Mac's Photos library as a [`PhotoSource`] (P1-04): a thin client of the ShoeboxPhotos Swift
//! package (`native/ShoeboxPhotos`, P1-03), linked by `build.rs`. Every call blocks in PhotoKit,
//! so each runs on a blocking thread.
//!
//! Originals are copied into the store (D4), as every desktop import is; one in iCloud only is
//! downloaded on mains power (D3), and a Live Photo's video comes with its still (P1-08).

// The C entry points of the Swift bridge.
#![allow(unsafe_code)]

use crate::photos_source::{Asset, Conditions, Permission, PhotoSource, Scan};
use anyhow::{anyhow, Context, Result};
use futures::future::BoxFuture;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::ffi::{c_char, CStr, CString};
use std::path::PathBuf;

extern "C" {
    fn sbp_permission() -> *mut c_char;
    fn sbp_request_permission() -> *mut c_char;
    fn sbp_scan(token: *const c_char) -> *mut c_char;
    fn sbp_read(id: *const c_char, dir: *const c_char, network: bool) -> *mut c_char;
    fn sbp_frame(id: *const c_char) -> *mut c_char;
    fn sbp_free(p: *mut c_char);
}

/// The bridge's reply: `{"ok": …}` or `{"error": "…"}`.
#[derive(Deserialize)]
#[serde(untagged)]
enum Reply<T> {
    Ok { ok: T },
    Error { error: String },
}

/// Take ownership of a reply string from the bridge and decode it.
fn decode<T: DeserializeOwned>(raw: *mut c_char) -> Result<T> {
    if raw.is_null() {
        return Err(anyhow!("no reply from Photos"));
    }
    // SAFETY: the bridge returns a NUL-terminated string from `strdup`, freed here once.
    let text = unsafe { CStr::from_ptr(raw) }
        .to_string_lossy()
        .into_owned();
    unsafe { sbp_free(raw) };
    match serde_json::from_str::<Reply<T>>(&text).context("reply from Photos")? {
        Reply::Ok { ok } => Ok(ok),
        Reply::Error { error } => Err(anyhow!(error)),
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f).await?
}

#[derive(Deserialize)]
struct ScanReply {
    assets: Vec<Asset>,
    removed: Vec<String>,
    token: String,
}

#[derive(Deserialize)]
struct ReadReply {
    path: String,
}

fn permission(state: &str) -> Permission {
    match state {
        "granted" => Permission::Granted,
        "limited" => Permission::Limited,
        // Never asked counts as not allowed: turning backup on asks.
        _ => Permission::Denied,
    }
}

/// The system Photos library on this Mac. `scratch` holds originals between reading and import.
pub struct MacPhotoSource {
    scratch: PathBuf,
}

impl MacPhotoSource {
    pub fn new(scratch: impl Into<PathBuf>) -> Self {
        Self {
            scratch: scratch.into(),
        }
    }

    fn read(&self, id: &str, network: bool) -> BoxFuture<'_, Result<PathBuf>> {
        let id = id.to_string();
        let scratch = self.scratch.clone();
        Box::pin(blocking(move || {
            std::fs::create_dir_all(&scratch)?;
            // A directory of its own: the caller deletes it with the Live Photo video beside.
            let mut nonce = [0u8; 8];
            getrandom::fill(&mut nonce).map_err(|e| anyhow!("{e}"))?;
            let dir = scratch.join(format!("photos-{}", hex::encode(nonce)));
            std::fs::create_dir(&dir)?;
            let cid = CString::new(id)?;
            let cdir = CString::new(dir.to_string_lossy().as_bytes())?;
            // SAFETY: both strings outlive the call; the reply is freed by `decode`.
            let r: Result<ReadReply> =
                decode(unsafe { sbp_read(cid.as_ptr(), cdir.as_ptr(), network) });
            match r {
                Ok(r) => Ok(PathBuf::from(r.path)),
                Err(e) => {
                    let _ = std::fs::remove_dir_all(&dir);
                    Err(e)
                }
            }
        }))
    }
}

impl PhotoSource for MacPhotoSource {
    fn permission(&self) -> BoxFuture<'_, Result<Permission>> {
        // SAFETY: no arguments; the reply is freed by `decode`.
        Box::pin(blocking(|| {
            decode::<String>(unsafe { sbp_permission() }).map(|s| permission(&s))
        }))
    }

    fn request_permission(&self) -> BoxFuture<'_, Result<Permission>> {
        // SAFETY: as above.
        Box::pin(blocking(|| {
            decode::<String>(unsafe { sbp_request_permission() }).map(|s| permission(&s))
        }))
    }

    fn scan(&self, token: Option<String>) -> BoxFuture<'_, Result<Scan>> {
        Box::pin(blocking(move || {
            let t = CString::new(token.unwrap_or_default())?;
            // SAFETY: `t` outlives the call; the reply is freed by `decode`.
            let r: ScanReply = decode(unsafe { sbp_scan(t.as_ptr()) })?;
            Ok(Scan {
                assets: r.assets,
                removed: r.removed,
                token: r.token,
            })
        }))
    }

    /// A Mac is taken to have a network; mains power is what downloads from iCloud wait for.
    fn conditions(&self) -> BoxFuture<'_, Result<Conditions>> {
        Box::pin(async {
            Ok(Conditions {
                on_wifi: true,
                charging: crate::search_models::on_mains(),
                battery_pct: 100,
            })
        })
    }

    fn read_original(&self, id: &str) -> BoxFuture<'_, Result<PathBuf>> {
        self.read(id, false)
    }

    fn download_original(&self, id: &str) -> BoxFuture<'_, Result<PathBuf>> {
        self.read(id, true)
    }

    fn video_frame(&self, id: &str, _path: Option<&str>) -> BoxFuture<'_, Result<Option<Vec<u8>>>> {
        let id = id.to_string();
        Box::pin(blocking(move || {
            let cid = CString::new(id)?;
            // SAFETY: `cid` outlives the call; the reply is freed by `decode`.
            let b64: String = decode(unsafe { sbp_frame(cid.as_ptr()) })?;
            crate::photos_source::decode_frame(&b64)
        }))
    }

    fn kind(&self) -> &'static str {
        "photos_library"
    }
}
