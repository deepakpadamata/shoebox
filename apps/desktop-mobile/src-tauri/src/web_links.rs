//! Web links to albums as the UI sees them (W1-07): thin glue over `shoebox_node::web_share`.

use crate::core::{now_ms, AppCore};
use crate::types::*;
use anyhow::{anyhow, ensure, Result};
use shoebox_core::model::AlbumId;
use shoebox_core::store::{LibraryStore, Page};
use shoebox_node::{WebShare, WebShareOptions};

fn album_id(s: &str) -> Result<AlbumId> {
    let bytes: [u8; 16] = hex::decode(s.trim())
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| anyhow!("`{s}` is not an album id"))?;
    Ok(AlbumId(bytes))
}

fn share_id(s: &str) -> Result<[u8; 16]> {
    hex::decode(s.trim())
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| anyhow!("`{s}` is not a link id"))
}

fn qr_svg(text: &str) -> Result<String> {
    let qr = qrcode::QrCode::new(text.as_bytes())
        .map_err(|e| anyhow!("the link is too long for a QR code: {e}"))?;
    Ok(qr
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(220, 220)
        .quiet_zone(true)
        .build())
}

/// Formats whose location `strip_location` cannot remove (the original is then left out of a link).
fn unremovable(mime: &str) -> bool {
    matches!(
        mime,
        "image/heic" | "image/heif" | "image/heic-sequence" | "image/heif-sequence" | "image/avif"
    )
}

impl AppCore {
    fn web_link_info(share: &WebShare, base: &str, stale: bool) -> Result<WebLinkInfo> {
        let text = share.link(base);
        Ok(WebLinkInfo {
            share_id: hex::encode(share.share_id),
            qr_svg: qr_svg(&text)?,
            text,
            full_size: share.full_size,
            expires_at: share.expires_ms.map(|e| e as f64),
            created_at: share.created_ms as f64,
            items: share.items,
            skipped: share.skipped,
            withheld: share.withheld,
            stale,
        })
    }

    /// An album's links, and whether the cloud is ready.
    pub async fn web_links(&self, album: &str) -> Result<WebLinksInfo> {
        let ln = self.ln().await?;
        let album = album_id(album)?;
        let enabled = ln.cloud_enabled().await?;
        let url = ln.cloud_url().await?;
        let mut links = vec![];
        if let Some(base) = &url {
            for s in ln.web_shares(&album).await? {
                let stale = ln.web_share_is_stale(&s).await.unwrap_or(false);
                links.push(Self::web_link_info(&s, base, stale)?);
            }
        }
        Ok(WebLinksInfo {
            enabled,
            url,
            links,
        })
    }

    /// Set the service's address and/or, once, the recovery phrase that gives the library its cloud secret.
    pub async fn cloud_setup(&self, url: Option<String>, phrase: Option<String>) -> Result<()> {
        let ln = self.ln().await?;
        if let Some(phrase) = phrase.filter(|p| !p.trim().is_empty()) {
            ln.enable_cloud_with_phrase(&phrase)
                .await
                .map_err(|_| anyhow!("that is not this library's recovery phrase"))?;
        }
        if let Some(url) = url.filter(|u| !u.trim().is_empty()) {
            ln.set_cloud_url(&url).await?;
        }
        Ok(())
    }

    /// What making a link to this album would upload.
    pub async fn web_link_check(&self, album: &str, full_size: bool) -> Result<WebLinkCheck> {
        let ln = self.ln().await?;
        let ids = ln
            .store()
            .album_items(&album_id(album)?, Page::all())
            .await?;
        let mut c = WebLinkCheck {
            photos: 0,
            videos: 0,
            bytes: 0.0,
            with_location: 0,
            unremovable: 0,
        };
        for id in ids {
            let Some(p) = ln.store().photo(&id).await? else {
                continue;
            };
            if p.deleted_at.value.is_some() {
                continue;
            }
            let video = p.original.mime.starts_with("video/");
            if video {
                c.videos += 1;
            } else {
                c.photos += 1;
            }
            c.bytes += (p.thumb.size + p.preview.size) as f64;
            if video || full_size {
                c.bytes += p.original.size as f64;
            }
            if full_size && !video && p.exif.gps.is_some() {
                c.with_location += 1;
                if unremovable(&p.original.mime) {
                    c.unremovable += 1;
                }
            }
        }
        Ok(c)
    }

    pub async fn web_link_create(
        &self,
        album: &str,
        full_size: bool,
        days: Option<u32>,
    ) -> Result<WebLinkInfo> {
        if let Some(d) = days {
            ensure!((1..=365).contains(&d), "a link lasts 1 to 365 days");
        }
        let ln = self.ln().await?;
        let expires_ms = days.map(|d| (now_ms() + u64::from(d) * 86_400_000) as i64);
        let share = ln
            .create_web_share(
                &album_id(album)?,
                WebShareOptions {
                    full_size,
                    expires_ms,
                },
            )
            .await?;
        let base = ln
            .cloud_url()
            .await?
            .ok_or_else(|| anyhow!("no cloud address is set"))?;
        Self::web_link_info(&share, &base, false)
    }

    pub async fn web_link_revoke(&self, album: &str, share: &str) -> Result<()> {
        self.ln()
            .await?
            .revoke_web_share(&album_id(album)?, &share_id(share)?)
            .await
    }

    pub async fn web_link_rotate(&self, album: &str, share: &str) -> Result<WebLinkInfo> {
        let ln = self.ln().await?;
        let new = ln
            .rotate_web_share(&album_id(album)?, &share_id(share)?)
            .await?;
        let base = ln
            .cloud_url()
            .await?
            .ok_or_else(|| anyhow!("no cloud address is set"))?;
        Self::web_link_info(&new, &base, false)
    }

    /// Bring this device's links up to date with their albums.
    pub async fn web_links_refresh(&self) -> Result<u32> {
        let r = self.ln().await?.refresh_web_shares().await?;
        if let Some(first) = r.failed.first() {
            return Err(anyhow!("{first}"));
        }
        Ok(r.refreshed)
    }
}
