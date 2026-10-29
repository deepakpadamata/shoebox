//! Serving blob bytes to the webview (`shoebox://blob/<photo id>?kind=thumb|preview|original`) and
//! to the dev HTTP bridge, straight from the blob store: no base64, byte ranges for video
//! seeking, and immutable caching because a blob's content never changes.

use crate::core::AppCore;
use anyhow::{anyhow, Result};
use shoebox_core::ids::Blake3Hash;
use shoebox_core::model::BlobKind;
use shoebox_core::policy::shows_directly;
use std::sync::Arc;

/// Without a `Range` header at most this much is returned (as a partial response); videos and
/// large originals are streamed in pieces by the media element.
const MAX_UNRANGED: u64 = 32 * 1024 * 1024;
/// An open-ended range (`bytes=N-`, what a media element asks for) gets at most this much: the
/// element asks for the rest piece by piece. Answering with the whole file made a large video
/// (read in full, from the gallery on Android) never start.
const MAX_OPEN_RANGE: u64 = 4 * 1024 * 1024;

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
    let (mut blob, mut bk) = match kind {
        "thumb" => (&rec.thumb, BlobKind::Thumb),
        "preview" => (&rec.preview, BlobKind::Preview),
        "original" => (&rec.original, BlobKind::Original),
        other => return Err(anyhow!("unknown kind `{other}`")),
    };
    if bk == BlobKind::Preview
        && shows_directly(&rec.original.mime)
        && core.blobs().size(&blob.hash).await?.is_none()
        && core.blobs().kept(&rec.original.hash).await?
    {
        // A device holding an original it shows as it is lets the preview go
        // (`LibraryNode::release_previews`): show the original, do not fetch the preview back.
        (blob, bk) = (&rec.original, BlobKind::Original);
    }
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

/// A `Range: bytes=first-[last]` header value (one range).
pub fn parse_range(value: &str) -> Option<(u64, Option<u64>)> {
    let spec = value.strip_prefix("bytes=")?;
    let (a, b) = spec.split_once('-')?;
    Some((
        a.parse().ok()?,
        if b.is_empty() {
            None
        } else {
            Some(b.parse().ok()?)
        },
    ))
}

/// What to send for `range` of a blob of `total` bytes: `(start, end exclusive, partial)`.
fn span(range: Option<(u64, Option<u64>)>, total: u64) -> (u64, u64, bool) {
    match range {
        None if total <= MAX_UNRANGED => (0, total, false),
        None => (0, MAX_UNRANGED, true),
        Some((first, last)) => {
            let first = first.min(total);
            let end = match last {
                Some(l) => (l + 1).min(total),
                None => first.saturating_add(MAX_OPEN_RANGE).min(total),
            };
            (first, end.max(first), true)
        }
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
    let (start, end, partial) = span(range, total);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_open_range_of_a_large_blob_is_one_piece() {
        let total = 140 * 1024 * 1024;
        assert_eq!(span(Some((0, None)), total), (0, MAX_OPEN_RANGE, true));
        let (start, end, _) = span(Some((total - 10, None)), total);
        assert_eq!(
            (start, end),
            (total - 10, total),
            "the last piece ends at the end"
        );
        assert_eq!(span(Some((10, Some(19))), total), (10, 20, true));
        assert_eq!(span(Some((total + 5, None)), total), (total, total, true));
        assert_eq!(span(None, 1000), (0, 1000, false));
    }
}
