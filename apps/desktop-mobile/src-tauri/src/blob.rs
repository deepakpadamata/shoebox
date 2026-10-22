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

pub(crate) async fn serve(
    core: &Arc<AppCore>,
    id: &str,
    kind: &str,
    range: Option<(u64, Option<u64>)>,
) -> Result<Option<BlobBytes>> {
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
    let (blob, bk) = match kind {
        "thumb" => (&rec.thumb, BlobKind::Thumb),
        "preview" => (&rec.preview, BlobKind::Preview),
        "original" => (&rec.original, BlobKind::Original),
        other => return Err(anyhow!("unknown kind `{other}`")),
    };
    let Some(total) = core.blobs().size(&blob.hash).await? else {
        if bk != BlobKind::Original {
            // Thumbnails and previews are small: get them for the retry.
            core.fetch_in_background(photo, bk);
        }
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
        core.blobs()
            .open_range(&blob.hash, start..end)
            .await?
            .to_vec()
    } else {
        vec![]
    };
    Ok(Some(BlobBytes {
        bytes,
        content_type: blob.mime.clone(),
        total,
        range: partial.then_some((start, end.saturating_sub(1))),
        etag: blob.hash.to_string(),
    }))
}
