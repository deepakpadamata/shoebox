//! Serving blob bytes to the webview (`shoebox://blob/<photo id>?kind=thumb|preview|original`) and
//! to the dev HTTP bridge, straight from the blob store: no base64, byte ranges for video
//! seeking, and immutable caching because a blob's content never changes.

use crate::core::AppCore;
use anyhow::{anyhow, Result};
use shoebox_core::ids::Blake3Hash;
use shoebox_core::model::BlobKind;
use std::sync::Arc;

/// Without a `Range` header at most this much is returned (as a partial response); videos and
/// large originals are streamed in pieces by the media element.
const MAX_UNRANGED: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobBytes {
    pub bytes: Vec<u8>,
    pub content_type: String,
    /// Size of the whole blob.
    pub total: u64,
    /// `Some((first, last))` (inclusive) when only part of the blob is returned.
    pub range: Option<(u64, u64)>,
    /// The blob hash: a strong validator and the cache key.
    pub etag: String,
}

pub async fn serve(
    core: &Arc<AppCore>,
    id: &str,
    kind: &str,
    range: Option<(u64, Option<u64>)>,
) -> Result<Option<BlobBytes>> {
    if let Some(rest) = kind.strip_prefix("album:") {
        // `album:<album id>:<kind>`: a photo of a shared album.
        let (album, kind) = rest
            .split_once(':')
            .ok_or_else(|| anyhow!("malformed album blob kind"))?;
        let (hash, mime, _) = core.shared_blob_ref(album, id, kind).await?;
        return read_blob(core, &hash, &mime, range).await;
    }
    let photo: Blake3Hash = {
        let b: [u8; 32] = hex::decode(id.trim())
            .ok()
            .and_then(|b| b.try_into().ok())
            .ok_or_else(|| anyhow!("not a photo id"))?;
        Blake3Hash(b)
    };
    let rec = core
        .record(&photo)
        .await?
        .ok_or_else(|| anyhow!("no such photo"))?;
    if kind == "poster" {
        // The device-local poster frame of a video (see `AppCore::set_poster`).
        return Ok(std::fs::read(core.poster_path(&photo))
            .ok()
            .map(|bytes| BlobBytes {
                total: bytes.len() as u64,
                etag: format!("poster-{}", Blake3Hash::of_bytes(&bytes)),
                bytes,
                content_type: "image/jpeg".into(),
                range: None,
            }));
    }
    let (blob, bk) = match kind {
        "thumb" => (&rec.thumb, BlobKind::Thumb),
        "preview" => (&rec.preview, BlobKind::Preview),
        "original" => (&rec.original, BlobKind::Original),
        other => return Err(anyhow!("unknown kind `{other}`")),
    };
    if core.blobs().size(&blob.hash).await?.is_none() && bk != BlobKind::Original {
        // Thumbnails and previews are small: get them for the retry.
        core.fetch_in_background(photo, bk);
    }
    match read_blob(core, &blob.hash, &blob.mime, range).await {
        Err(e) if bk == BlobKind::Original => {
            // A gallery file this phone references may be gone: sort that out, and report the
            // original as not here so the viewer fetches it from another device.
            tracing::warn!("reading the original of {photo}: {e:#}");
            core.original_unreadable(&blob.hash).await?;
            Ok(None)
        }
        other => other,
    }
}

/// The bytes (or a range) of a blob that is on this device; `None` if it is not.
async fn read_blob(
    core: &Arc<AppCore>,
    hash: &Blake3Hash,
    mime: &str,
    range: Option<(u64, Option<u64>)>,
) -> Result<Option<BlobBytes>> {
    let Some(total) = core.blobs().size(hash).await? else {
        return Ok(None);
    };
    let (start, end, partial) = match range {
        None if total <= MAX_UNRANGED => (0, total, false),
        None => (0, MAX_UNRANGED, true),
        Some((first, last)) => {
            let first = first.min(total);
            let end = last.map_or(total, |l| (l + 1).min(total)).max(first);
            (first, end, true)
        }
    };
    let bytes = if end > start {
        core.blobs().open_range(hash, start..end).await?.to_vec()
    } else {
        vec![]
    };
    Ok(Some(BlobBytes {
        bytes,
        content_type: mime.to_string(),
        total,
        range: partial.then_some((start, end.saturating_sub(1))),
        etag: hash.to_string(),
    }))
}
