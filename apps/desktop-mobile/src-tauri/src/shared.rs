//! Shared albums as the UI sees them (M5): thin glue between `SharedAlbums` (the store and
//! network side) and the library (photos to share, photos to save).

use crate::core::{now_ms, AppCore};
use crate::types::*;
use anyhow::{anyhow, bail, ensure, Result};
use shoebox_core::ids::{Blake3Hash, EndpointId};
use shoebox_core::merge::Lww;
use shoebox_core::model::{
    AlbumId, BlobKind, BlobRef, ExifSummary, MemberRole, PhotoRecord, SharedItem,
};
use shoebox_core::store::{AlbumEventKind, Cursor, LibraryStore, Page};
use shoebox_media::strip_location;
use shoebox_node::Protection;
use shoebox_store::library_link;
use shoebox_store::{item_blob, SharedAlbums, SharedSummary};
use std::sync::Arc;

/// Adds of one member closer together than this are one line in the activity feed.
const FEED_GAP_MS: u64 = 10 * 60 * 1000;

fn role_name(r: Option<MemberRole>) -> &'static str {
    match r {
        Some(MemberRole::Owner) => "owner",
        Some(MemberRole::Contributor) => "contributor",
        Some(MemberRole::Viewer) | None => "viewer",
    }
}

fn role_of(s: &str) -> MemberRole {
    match s {
        "owner" => MemberRole::Owner,
        "contributor" => MemberRole::Contributor,
        _ => MemberRole::Viewer,
    }
}

fn parse_role(s: &str) -> Result<MemberRole> {
    Ok(match s {
        "contributor" => MemberRole::Contributor,
        "viewer" => MemberRole::Viewer,
        other => bail!("you can invite people as contributor or viewer, not `{other}`"),
    })
}

fn album_id(s: &str) -> Result<AlbumId> {
    let b: [u8; 16] = hex::decode(s.trim())
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| anyhow!("`{s}` is not an album id"))?;
    Ok(AlbumId(b))
}

fn photo_id(s: &str) -> Result<Blake3Hash> {
    let b: [u8; 32] = hex::decode(s.trim())
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| anyhow!("`{s}` is not a photo id"))?;
    Ok(Blake3Hash(b))
}

fn short(id: &EndpointId) -> String {
    hex::encode(&id.0[..4])
}

fn info(s: SharedSummary) -> SharedAlbumInfo {
    SharedAlbumInfo {
        id: hex::encode(s.id.0),
        name: s.name,
        role: role_name(s.role).into(),
        item_count: s.item_count as u32,
        member_count: s.member_count as u32,
        keep_originals: s.keep_originals,
        location: s.location,
        unread: s.unread as u32,
        cover: s.cover.map(|h| h.to_string()),
    }
}

impl AppCore {
    async fn sa(&self) -> Result<SharedAlbums> {
        self.shared
            .read()
            .await
            .clone()
            .ok_or_else(|| anyhow!("set up or join a library first"))
    }

    /// The name other people see in shared albums: the person's name when set (shared by all
    /// their devices), this device's name until then.
    async fn my_name(&self) -> String {
        if let Ok(ln) = self.ln().await {
            if let Ok(Some(n)) = library_link::person_name(ln.store().as_ref()).await {
                return n;
            }
        }
        let n = self.device_name();
        if n.trim().is_empty() {
            "This device".into()
        } else {
            n
        }
    }

    /// The name shown to people you share with, and whether it was set (or is the device name).
    pub async fn profile(&self) -> Result<Profile> {
        let ln = self.ln().await?;
        let set = library_link::person_name(ln.store().as_ref()).await?;
        Ok(Profile {
            is_set: set.is_some(),
            name: self.my_name().await,
        })
    }

    /// Set the name shown to people you share with, on every device of the library.
    pub async fn set_profile_name(&self, name: &str) -> Result<()> {
        let ln = self.ln().await?;
        library_link::set_person_name(ln.store().as_ref(), name).await?;
        if let Ok(sa) = self.sa().await {
            sa.rename_me(name.trim()).await;
        }
        Ok(())
    }

    /// Bring the library's list of shared albums up to date now (after creating or joining).
    async fn publish_shared(&self) {
        if let (Ok(sa), Ok(ln)) = (self.sa().await, self.ln().await) {
            let name = self.my_name().await;
            if let Err(e) = library_link::reconcile(&sa, &ln, &name).await {
                tracing::debug!("publishing shared albums to the library: {e:#}");
            }
        }
    }

    pub async fn shared_albums(&self) -> Result<Vec<SharedAlbumInfo>> {
        Ok(self
            .sa()
            .await?
            .list()
            .await?
            .into_iter()
            .map(info)
            .collect())
    }

    async fn shared_info(&self, id: &AlbumId) -> Result<SharedAlbumInfo> {
        self.sa()
            .await?
            .list()
            .await?
            .into_iter()
            .find(|s| s.id == *id)
            .map(info)
            .ok_or_else(|| anyhow!("no such shared album"))
    }

    pub async fn shared_create(&self, name: &str) -> Result<SharedAlbumInfo> {
        let sa = self.sa().await?;
        let id = sa.create(name, &self.my_name().await).await?;
        self.publish_shared().await;
        self.shared_info(&id).await
    }

    pub async fn shared_invite(&self, album: &str, role: &str, days: u32) -> Result<InviteOut> {
        ensure!((1..=90).contains(&days), "an invitation lasts 1 to 90 days");
        let sa = self.sa().await?;
        let text = sa
            .invite(&album_id(album)?, parse_role(role)?, days)
            .await?;
        let qr = qrcode::QrCode::new(text.as_bytes())
            .map_err(|e| anyhow!("the invitation is too long for a QR code: {e}"))?;
        let svg = qr
            .render::<qrcode::render::svg::Color>()
            .min_dimensions(220, 220)
            .quiet_zone(true)
            .build();
        Ok(InviteOut {
            text,
            qr_svg: svg,
            expires_at: (now_ms() + days as u64 * 86_400_000) as f64,
        })
    }

    pub async fn shared_join(&self, text: &str) -> Result<SharedAlbumInfo> {
        let sa = self.sa().await?;
        let id = sa.join(text, &self.my_name().await).await?;
        self.publish_shared().await;
        self.shared_info(&id).await
    }

    /// Leave an album on every device of the library.
    pub async fn shared_leave(&self, album: &str) -> Result<()> {
        let (sa, ln) = (self.sa().await?, self.ln().await?);
        library_link::leave_everywhere(&sa, &ln, &album_id(album)?).await
    }

    fn member_names(
        members: &[shoebox_store::MemberInfo],
    ) -> std::collections::HashMap<EndpointId, String> {
        members
            .iter()
            .map(|m| {
                let n = m.member.display_name.clone();
                // A name that is just the device id (nothing was set yet) reads badly: shorten.
                let n = if n.len() >= 32 && n.chars().all(|c| c.is_ascii_hexdigit()) {
                    format!("Device {}", short(&m.member.key))
                } else {
                    n
                };
                (m.member.key, n)
            })
            .collect()
    }

    pub async fn shared_items(
        &self,
        album: &str,
        after: Option<String>,
        limit: u32,
    ) -> Result<SharedItemsPage> {
        let sa = self.sa().await?;
        let id = album_id(album)?;
        let store = sa.album(&id)?;
        let names = Self::member_names(&sa.members(&id).await?);
        let me = shoebox_node::from_iroh_id(&self.node().id());
        let limit = limit.clamp(1, 5000) as usize;
        let cursor = match after {
            Some(a) => Some(Cursor {
                taken_at: shoebox_core::Timestamp(0),
                id: photo_id(&a)?,
            }),
            None => None,
        };
        use shoebox_core::store::AlbumStore;
        let items = store
            .items(Page {
                after: cursor,
                limit: limit + 1,
            })
            .await?;
        let next = (items.len() > limit).then(|| items[limit - 1].id.to_string());
        let mut out = vec![];
        for it in items.into_iter().take(limit) {
            out.push(SharedItemInfo {
                id: it.id.to_string(),
                taken_at: it.taken_at.0 as f64,
                added_at: it.added_at.0 as f64,
                added_by: names
                    .get(&it.added_by)
                    .cloned()
                    .unwrap_or_else(|| format!("Device {}", short(&it.added_by))),
                is_video: it.original.mime.starts_with("video/"),
                has_thumb: sa.has(&it.thumb.hash).await.unwrap_or(false),
                has_original: sa
                    .has(&item_blob(&it, BlobKind::Original))
                    .await
                    .unwrap_or(false),
                mine: it.added_by == me,
                location_removed: it.shared_hash.is_some(),
            });
        }
        Ok(SharedItemsPage {
            items: out,
            next,
            total: store.item_count().await as u32,
        })
    }

    pub async fn shared_members(&self, album: &str) -> Result<Vec<SharedMemberInfo>> {
        let sa = self.sa().await?;
        let members = sa.members(&album_id(album)?).await?;
        let names = Self::member_names(&members);
        let my_person = members.iter().find(|m| m.me).and_then(|m| m.person);
        // One row per person: their devices together (the strongest role among them).
        let mut out: Vec<(Option<shoebox_core::store::LibraryId>, SharedMemberInfo)> = vec![];
        for m in &members {
            let me = m.me || (m.person.is_some() && m.person == my_person);
            if let Some(row) = out
                .iter_mut()
                .find(|(p, _)| p.is_some() && *p == m.person)
                .map(|(_, r)| r)
            {
                row.devices += 1;
                row.me |= me;
                if m.member.role < role_of(&row.role) {
                    row.role = role_name(Some(m.member.role)).into();
                }
                continue;
            }
            out.push((
                m.person,
                SharedMemberInfo {
                    key: hex::encode(m.member.key.0),
                    name: names[&m.member.key].clone(),
                    role: role_name(Some(m.member.role)).into(),
                    me,
                    devices: 1,
                },
            ));
        }
        Ok(out.into_iter().map(|(_, r)| r).collect())
    }

    pub async fn shared_feed(&self, album: &str) -> Result<Vec<FeedEntry>> {
        let sa = self.sa().await?;
        let id = album_id(album)?;
        let names = Self::member_names(&sa.members(&id).await?);
        let name_of = |k: &EndpointId| {
            names
                .get(k)
                .cloned()
                .unwrap_or_else(|| format!("Device {}", short(k)))
        };
        let mut feed: Vec<FeedEntry> = vec![];
        for ev in sa.events(&id, None).await? {
            let (kind, count) = match ev.kind {
                AlbumEventKind::ItemsAdded { count } => ("added", count),
                AlbumEventKind::MemberJoined => ("joined", 1),
            };
            let member = name_of(&ev.member);
            let at = ev.hlc.physical_ms;
            if kind == "added" {
                if let Some(last) = feed.last_mut() {
                    if last.kind == "added"
                        && last.member == member
                        && at.saturating_sub(last.at as u64) <= FEED_GAP_MS
                    {
                        last.count += count;
                        last.at = at as f64;
                        continue;
                    }
                }
            }
            feed.push(FeedEntry {
                kind: kind.into(),
                member,
                count,
                at: at as f64,
            });
        }
        feed.reverse(); // newest first
        Ok(feed)
    }

    pub async fn shared_add_check(&self, album: &str, ids: &[String]) -> Result<AddCheck> {
        let sa = self.sa().await?;
        let (_, policy) = sa.settings(&album_id(album)?)?;
        let ln = self.ln().await?;
        let mut with_location = 0;
        for i in ids {
            if let Some(rec) = ln.store().photo(&photo_id(i)?).await? {
                if rec.exif.gps.is_some() {
                    with_location += 1;
                }
            }
        }
        Ok(AddCheck {
            total: ids.len() as u32,
            with_location,
            policy,
        })
    }

    pub async fn shared_add(
        &self,
        album: &str,
        ids: &[String],
        location: Option<String>,
        remember: bool,
    ) -> Result<AddResult> {
        let sa = self.sa().await?;
        let aid = album_id(album)?;
        let ln = self.ln().await?;
        let me = shoebox_node::from_iroh_id(&self.node().id());
        let (_, policy) = sa.settings(&aid)?;
        let choice = location.unwrap_or(policy);
        ensure!(
            matches!(choice.as_str(), "ask" | "keep" | "remove"),
            "unknown location setting `{choice}`"
        );
        let now = now_ms() as i64;
        let mut recs = vec![];
        for i in ids {
            let rec = ln
                .store()
                .photo(&photo_id(i)?)
                .await?
                .ok_or_else(|| anyhow!("a selected photo is not in the library"))?;
            ensure!(!rec.is_deleted(), "a deleted photo cannot be shared");
            recs.push(rec);
        }
        ensure!(
            choice != "ask" || recs.iter().all(|r| r.exif.gps.is_none()),
            "Choose whether to keep or remove the location of these photos"
        );
        let mut out = AddResult {
            added: 0,
            location_removed: 0,
            skipped: vec![],
        };
        let mut items = vec![];
        for rec in recs {
            let is_video = rec.original.mime.starts_with("video/");
            let strip = choice == "remove" && (rec.exif.gps.is_some() || is_video);
            if !strip {
                items.push(SharedItem {
                    id: rec.id,
                    original: rec.original.clone(),
                    shared_hash: None,
                    preview: rec.preview.clone(),
                    thumb: rec.thumb.clone(),
                    added_by: me,
                    added_at: shoebox_core::Timestamp(now),
                    taken_at: rec.taken_at,
                });
                continue;
            }
            // The original has to be here to make the copy without a location.
            if !self.node().blobs().has(&rec.original.hash).await? {
                ln.fetch_now(&rec.id, BlobKind::Original).await.map_err(|e| {
                    anyhow!("the original of a photo is not on this device and could not be fetched: {e}")
                })?;
            }
            let bytes = self.node().blobs().get(&rec.original.hash).await?;
            match strip_location(&bytes) {
                Err(e) => out.skipped.push(format!("{e}")),
                Ok(s) if !s.changed => items.push(SharedItem {
                    id: rec.id,
                    original: rec.original.clone(),
                    shared_hash: None,
                    preview: rec.preview.clone(),
                    thumb: rec.thumb.clone(),
                    added_by: me,
                    added_at: shoebox_core::Timestamp(now),
                    taken_at: rec.taken_at,
                }),
                Ok(s) => {
                    let size = s.bytes.len() as u64;
                    let hash = self
                        .node()
                        .blobs()
                        .add_bytes(s.bytes, Protection::Album(aid))
                        .await?;
                    // Members only ever learn the hash of the copy, never of the original.
                    let copy = BlobRef {
                        hash,
                        size,
                        mime: rec.original.mime.clone(),
                    };
                    items.push(SharedItem {
                        id: hash,
                        original: copy,
                        shared_hash: Some(hash),
                        preview: rec.preview.clone(),
                        thumb: rec.thumb.clone(),
                        added_by: me,
                        added_at: shoebox_core::Timestamp(now),
                        taken_at: rec.taken_at,
                    });
                    out.location_removed += 1;
                }
            }
        }
        if !items.is_empty() {
            out.added = items.len() as u32;
            sa.add_items(&aid, items).await?;
        }
        if remember && matches!(choice.as_str(), "keep" | "remove") {
            sa.set_location_policy(&aid, &choice)?;
        }
        Ok(out)
    }

    /// Copy shared photos into this device's library (the same blobs, no second copy).
    pub async fn shared_save(&self, album: &str, ids: &[String]) -> Result<u32> {
        let sa = self.sa().await?;
        let aid = album_id(album)?;
        let store = sa.album(&aid)?;
        let ln = self.ln().await?;
        let me = shoebox_node::from_iroh_id(&self.node().id());
        let hlc = shoebox_core::Hlc {
            physical_ms: now_ms(),
            counter: 0,
            device: me,
        };
        let mut saved = 0;
        for i in ids {
            let item = store
                .item(&photo_id(i)?)
                .await
                .ok_or_else(|| anyhow!("that photo is no longer in the album"))?;
            let original = item_blob(&item, BlobKind::Original);
            for (h, _) in [
                (item.thumb.hash, BlobKind::Thumb),
                (item.preview.hash, BlobKind::Preview),
                (original, BlobKind::Original),
            ] {
                sa.fetch_now(&aid, &h).await.map_err(|e| {
                    anyhow!("could not get this photo yet ({e}); try again when someone from the album is online")
                })?;
            }
            let rec = PhotoRecord {
                id: item.id,
                original: item.original.clone(),
                preview: item.preview.clone(),
                thumb: item.thumb.clone(),
                taken_at: item.taken_at,
                exif: ExifSummary::default(),
                imported_by: me,
                favorite: Lww::new(false, hlc),
                deleted_at: Lww::new(None, hlc),
            };
            let mut held = vec![];
            for (h, k) in [
                (item.thumb.hash, BlobKind::Thumb),
                (item.preview.hash, BlobKind::Preview),
                (original, BlobKind::Original),
            ] {
                self.node().blobs().protect(&h, Protection::Library).await?;
                held.push((h, k));
            }
            ln.store().upsert_photo(rec).await?;
            ln.publish_holdings(held).await?;
            saved += 1;
        }
        Ok(saved)
    }

    pub async fn shared_settings(
        &self,
        album: &str,
        keep_originals: Option<bool>,
        location: Option<String>,
    ) -> Result<()> {
        let sa = self.sa().await?;
        let id = album_id(album)?;
        if let Some(k) = keep_originals {
            sa.set_keep_originals(&id, k).await?;
        }
        if let Some(l) = location {
            sa.set_location_policy(&id, &l)?;
        }
        Ok(())
    }

    /// Remove a member (owner only): the album moves to a new space without them.
    pub async fn shared_remove_member(&self, album: &str, member: &str) -> Result<()> {
        let key: [u8; 32] = hex::decode(member.trim())
            .ok()
            .and_then(|b| b.try_into().ok())
            .ok_or_else(|| anyhow!("`{member}` is not a member id"))?;
        self.sa()
            .await?
            .remove_member(&album_id(album)?, &EndpointId(key))
            .await
    }

    pub async fn shared_seen(&self, album: &str) -> Result<()> {
        self.sa().await?.mark_seen(&album_id(album)?)
    }

    /// Download one blob of a shared photo now (the user opened it).
    pub async fn shared_fetch(&self, album: &str, id: &str, kind: &str) -> Result<()> {
        let sa = self.sa().await?;
        let aid = album_id(album)?;
        let item = sa
            .album(&aid)?
            .item(&photo_id(id)?)
            .await
            .ok_or_else(|| anyhow!("that photo is no longer in the album"))?;
        let kind = match kind {
            "thumb" => BlobKind::Thumb,
            "preview" => BlobKind::Preview,
            "original" => BlobKind::Original,
            other => bail!("unknown blob kind `{other}`"),
        };
        sa.fetch_now(&aid, &item_blob(&item, kind)).await
    }

    /// The blob behind `shoebox://blob/<item id>?kind=album:<album id>:<kind>`.
    pub(crate) async fn shared_blob_ref(
        self: &Arc<Self>,
        album: &str,
        id: &str,
        kind: &str,
    ) -> Result<(Blake3Hash, String, bool)> {
        let sa = self.sa().await?;
        let aid = album_id(album)?;
        let item = sa
            .album(&aid)?
            .item(&photo_id(id)?)
            .await
            .ok_or_else(|| anyhow!("that photo is no longer in the album"))?;
        let (hash, mime, small) = match kind {
            "thumb" => (item.thumb.hash, item.thumb.mime.clone(), true),
            "preview" => (item.preview.hash, item.preview.mime.clone(), true),
            "original" => (
                item_blob(&item, BlobKind::Original),
                item.original.mime.clone(),
                false,
            ),
            other => bail!("unknown kind `{other}`"),
        };
        if small && !sa.has(&hash).await? {
            // Thumbnails and previews are small: get them for the retry.
            let (sa2, aid2) = (sa.clone(), aid);
            tokio::spawn(async move {
                let _ = sa2.fetch_now(&aid2, &hash).await;
            });
        }
        Ok((hash, mime, small))
    }

    // ---------- sharing a personal album ----------

    async fn personal_album(&self, album: &str) -> Result<(AlbumId, String, Vec<String>)> {
        let ln = self.ln().await?;
        let id = album_id(album)?;
        let name = ln
            .store()
            .albums()
            .await?
            .into_iter()
            .find(|a| a.id == id)
            .map(|a| a.name)
            .ok_or_else(|| anyhow!("no such album"))?;
        let mut ids = vec![];
        for p in ln.store().album_items(&id, Page::all()).await? {
            if let Some(rec) = ln.store().photo(&p).await? {
                if !rec.is_deleted() {
                    ids.push(p.to_string());
                }
            }
        }
        Ok((id, name, ids))
    }

    /// Before sharing an album: how many of its photos carry a location.
    pub async fn album_share_check(&self, album: &str) -> Result<AddCheck> {
        let (_, _, ids) = self.personal_album(album).await?;
        let ln = self.ln().await?;
        let mut with_location = 0;
        for i in &ids {
            if let Some(rec) = ln.store().photo(&photo_id(i)?).await? {
                if rec.exif.gps.is_some() {
                    with_location += 1;
                }
            }
        }
        Ok(AddCheck {
            total: ids.len() as u32,
            with_location,
            policy: "ask".into(),
        })
    }

    /// Share one of your albums: a shared album with its name and photos, kept linked so photos
    /// added to the album later are shared too (from whichever of your devices can). `location`
    /// is `keep` or `remove`, asked once and used for every later photo.
    pub async fn album_share(
        &self,
        album: &str,
        location: Option<String>,
    ) -> Result<SharedAlbumInfo> {
        let (pid, name, ids) = self.personal_album(album).await?;
        let (sa, ln) = (self.sa().await?, self.ln().await?);
        ensure!(
            library_link::shared_from(ln.store().as_ref(), &pid)
                .await?
                .is_none(),
            "this album is already shared"
        );
        if let Some(l) = &location {
            ensure!(
                matches!(l.as_str(), "keep" | "remove"),
                "choose whether to keep or remove the location"
            );
        }
        let id = sa.create(&name, &self.my_name().await).await?;
        let hex_id = hex::encode(id.0);
        if let Some(l) = &location {
            sa.set_location_policy(&id, l)?;
        }
        if !ids.is_empty() {
            self.shared_add(&hex_id, &ids, location.clone(), true)
                .await?;
        }
        library_link::link(&sa, &ln, &id, &pid, location).await?;
        self.publish_shared().await;
        self.shared_info(&id).await
    }

    /// Share what was added to shared personal albums since (on any device of the library).
    /// Photos already in the shared album, including copies without location, are skipped.
    pub async fn sync_linked_albums(&self) -> Result<u32> {
        let (sa, ln) = (self.sa().await?, self.ln().await?);
        let mut added = 0;
        for (personal, shared, location) in library_link::shared_links(ln.store().as_ref()).await? {
            let Ok(album) = sa.album(&shared) else {
                continue;
            };
            if !album.can_add().await {
                continue;
            }
            use shoebox_core::store::AlbumStore;
            let items = album.items(Page::all()).await?;
            let mut missing = vec![];
            for p in ln.store().album_items(&personal, Page::all()).await? {
                let Some(rec) = ln.store().photo(&p).await? else {
                    continue;
                };
                if rec.is_deleted()
                    || items
                        .iter()
                        .any(|i| i.id == rec.id || i.preview.hash == rec.preview.hash)
                {
                    continue;
                }
                // Without a location choice, only photos without a location can go.
                if location.is_none() && rec.exif.gps.is_some() {
                    continue;
                }
                missing.push(p.to_string());
            }
            if !missing.is_empty() {
                let r = self
                    .shared_add(&hex::encode(shared.0), &missing, location.clone(), false)
                    .await?;
                added += r.added;
            }
        }
        Ok(added)
    }

    /// Personal album id → the shared album it is shared as.
    pub(crate) async fn shared_links(&self) -> Vec<(String, String)> {
        let Ok(ln) = self.ln().await else {
            return vec![];
        };
        library_link::shared_links(ln.store().as_ref())
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|(p, s, _)| (hex::encode(p.0), hex::encode(s.0)))
            .collect()
    }
}
