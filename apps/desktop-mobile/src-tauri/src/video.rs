//! Videos in the platform's own player (R1-16).
//!
//! Android's WebView re-applies byte ranges to custom-scheme responses, so a long video cannot
//! play from `shoebox://` (A-098). There the viewer hands the video to the system player instead:
//! a video in this phone's gallery is played from its file, where it already is, and one held in
//! the blob store is read through [`read_blob`] (a Media3 data source over JNI), range by range,
//! with no copy, no port and nothing listening. Desktops keep `<video>` over `shoebox://`.

use crate::core::{parse_hash, AppCore};
use crate::photos_source::VideoSource;
use crate::types::AppEvent;
use anyhow::{anyhow, Result};
use std::sync::Arc;

impl AppCore {
    /// Where the video of photo `id` is on this device: `kind` is `original` or `motion` (the
    /// video of a Live Photo), `album` a shared album the photo is in. `None` = not here yet.
    pub async fn video_source(
        self: &Arc<Self>,
        id: &str,
        kind: &str,
        album: Option<&str>,
    ) -> Result<Option<VideoSource>> {
        let (hash, mime) = match (album, kind) {
            (Some(album), _) => {
                let (hash, mime, _) = self.shared_blob_ref(album, id, kind).await?;
                (hash, mime)
            }
            (None, "motion") => {
                let photo = parse_hash(id)?;
                let m = self
                    .motion(&photo)
                    .await?
                    .ok_or_else(|| anyhow!("this photo has no video"))?;
                // Stored as a blob, or a part of the photo's original: the player reads either
                // through the handle, byte range by byte range.
                let Some(handle) = self.ln().await?.clip_source(&photo).await? else {
                    return Ok(None);
                };
                let size = self.blobs().handle_size(&handle).await?;
                return Ok(size.map(|size| VideoSource {
                    path: None,
                    blob: handle.to_string(),
                    mime: m.video.mime,
                    size,
                }));
            }
            (None, "original") => {
                let rec = self
                    .record(&parse_hash(id)?)
                    .await?
                    .ok_or_else(|| anyhow!("no such photo"))?;
                (rec.original.hash, rec.original.mime)
            }
            (None, other) => return Err(anyhow!("`{other}` is not a video")),
        };
        let Some(size) = self.blobs().size(&hash).await? else {
            return Ok(None);
        };
        let path = if album.is_none() && kind == "original" {
            self.gallery_path(&hash).await
        } else {
            None
        };
        Ok(Some(VideoSource {
            path,
            blob: hash.to_string(),
            mime,
            size,
        }))
    }

    /// The gallery file of an original this phone references in place, while it is there.
    async fn gallery_path(&self, hash: &shoebox_core::ids::Blake3Hash) -> Option<String> {
        let ln = self.ln().await.ok()?;
        if !ln.is_in_place(hash).await.unwrap_or(false) {
            return None;
        }
        let want = hash.to_string();
        let backup = self.backup.lock().await;
        backup
            .tracked
            .values()
            .filter(|t| t.hash == want)
            .find_map(|t| t.path.clone())
            .filter(|p| std::path::Path::new(p).is_file())
    }

    /// Whether this device plays videos in the platform's player rather than in the web view.
    pub fn native_video(&self) -> bool {
        self.photo_source
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|s| s.plays_video())
    }

    /// Play a video that is on this device in the platform's player. The viewer fetches it first
    /// (`fetch_now`), as it does any original.
    pub async fn play_video(
        self: &Arc<Self>,
        id: &str,
        kind: &str,
        album: Option<&str>,
    ) -> Result<()> {
        let source = self
            .photo_source
            .lock()
            .unwrap()
            .clone()
            .filter(|s| s.plays_video())
            .ok_or_else(|| anyhow!("this device plays videos in the app"))?;
        let video = self
            .video_source(id, kind, album)
            .await?
            .ok_or_else(|| anyhow!("this video is not on this device yet"))?;
        source.play_video(&video).await
    }

    /// Make the poster frame of video `id` with the platform's thumbnailer, from the original on
    /// this device, and keep it in the library for every device. `false` when it cannot be made
    /// here (no original, or no thumbnailer); the web view's frame grab is then not tried either.
    pub async fn make_poster(self: &Arc<Self>, id: &str) -> Result<bool> {
        let Some(source) = self.photo_source.lock().unwrap().clone() else {
            return Ok(false);
        };
        let Some(video) = self.video_source(id, "original", None).await? else {
            return Ok(false);
        };
        let Some(jpeg) = source.video_frame_of(&video).await? else {
            return Ok(false);
        };
        if self
            .ln()
            .await?
            .set_video_poster(&parse_hash(id)?, jpeg)
            .await?
        {
            self.emit(AppEvent::LibraryChanged);
        }
        Ok(true)
    }
}

/// The running app's core, for the player's reads (set once the app has opened its library).
#[cfg(all(feature = "tauri-app", target_os = "android"))]
static CORE: std::sync::OnceLock<Arc<AppCore>> = std::sync::OnceLock::new();

#[cfg(all(feature = "tauri-app", target_os = "android"))]
pub fn install(core: Arc<AppCore>) {
    let _ = CORE.set(core);
}

/// Bytes `offset..offset + len` of blob `hash` (fewer at its end), checked against the hash as
/// they are read.
#[cfg(all(feature = "tauri-app", target_os = "android"))]
fn read_blob(handle: &str, offset: u64, len: u64) -> Result<Vec<u8>> {
    let core = CORE.get().ok_or_else(|| anyhow!("the app is not open"))?;
    let handle: shoebox_node::BlobHandle = handle.parse()?;
    tauri::async_runtime::block_on(core.blobs().read_handle(&handle, offset..offset + len))
        .map(|b| b.to_vec())
}

/// `NativeBridge.readBlob(hash, offset, length)`: a piece of a blob for the player and the
/// thumbnailer (`BlobDataSource.kt`); `null` when it cannot be read.
#[cfg(all(feature = "tauri-app", target_os = "android"))]
#[allow(unsafe_code)] // a JNI entry point needs an unmangled name
#[no_mangle]
pub extern "system" fn Java_org_shoebox_desktop_NativeBridge_readBlob(
    mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    hash: jni::objects::JString,
    offset: jni::sys::jlong,
    length: jni::sys::jint,
) -> jni::sys::jbyteArray {
    let read = (|| -> Result<Vec<u8>> {
        let hash: String = env.get_string(&hash)?.into();
        read_blob(&hash, offset.max(0) as u64, length.max(0) as u64)
    })();
    match read {
        Ok(b) => env
            .byte_array_from_slice(&b)
            .map(|a| a.into_raw())
            .unwrap_or(std::ptr::null_mut()),
        Err(e) => {
            tracing::warn!("player read of {offset}+{length}: {e:#}");
            std::ptr::null_mut()
        }
    }
}
