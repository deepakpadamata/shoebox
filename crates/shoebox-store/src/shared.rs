//! Shared albums on a device (M5): which albums this device belongs to, keeping them in sync with
//! their members, and fetching the members' photos.
//!
//! Each album is an iroh-docs namespace ([`DocsAlbum`]). This module remembers the namespace
//! capability of every album in `shared-albums.json` (so they reopen after a restart), lets
//! the album members through the node's connection gate for blob transfer, syncs every album with
//! its members from time to time, and downloads thumbnails and previews (and originals when the
//! album is kept in full) into the blob store under an `album/<id>/<hash>` protection.

use anyhow::{anyhow, bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use shoebox_core::clock::{Clock, SystemClock};
use shoebox_core::hlc::Hlc;
use shoebox_core::ids::{Blake3Hash, EndpointId};
use shoebox_core::model::{AlbumId, BlobKind, Member, MemberRole, SharedItem};
use shoebox_core::store::{
    AlbumEvent, AlbumEventKind, AlbumStore, Expiry, InviteTicket, Page, PeerAddr, SharedAlbumMeta,
    StoreBackend, StoreError,
};
use shoebox_node::{to_endpoint_addr, to_iroh_id, ConnHandler, Node, Protection, ALBUM_ALPN};
use shoebox_store_docs::{DocsAlbum, DocsBackend};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, Notify, Semaphore};
use tracing::{debug, warn};

/// Uppercase letters, digits and `-` only, so a QR code can use its denser alphanumeric mode.
const INVITE_PREFIX: &str = "SHOEBOX-ALBUM1-";
const TICK: Duration = Duration::from_secs(5);
const RESYNC: Duration = Duration::from_secs(30);
const SYNC_TIMEOUT: Duration = Duration::from_secs(25);
const FETCH_RETRY: Duration = Duration::from_secs(20);

/// What changed in some shared album (a signal to reload it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlbumNotice(pub AlbumId);

#[derive(Serialize, Deserialize, Clone)]
struct Reg {
    album: AlbumId,
    /// Serialised iroh-docs capability: the key to the album's data.
    capability: Vec<u8>,
    /// Where the invitation came from; a first place to look for the other members.
    inviter: Option<PeerAddr>,
    /// Keep every member's original on this device (otherwise only previews).
    keep_originals: bool,
    /// What to do with GPS data when adding photos: `ask`, `keep` or `remove`.
    location: String,
    /// Events up to this time have been looked at.
    seen_ms: u64,
    /// Devices that joined as viewers (they cannot write their own membership entry).
    #[serde(default)]
    viewers: Vec<EndpointId>,
    /// After the album moved to a new space (a member was removed): who has not yet been told
    /// where it went, and whether they may write.
    #[serde(default)]
    pending_moves: Vec<(EndpointId, bool)>,
}

/// "The album moved": sent by the owner to each remaining member over [`ALBUM_ALPN`].
#[derive(Serialize, Deserialize)]
struct Moved {
    album: AlbumId,
    capability: Vec<u8>,
    owner: PeerAddr,
}

fn contributor() -> MemberRole {
    MemberRole::Contributor
}

/// An invitation this device issued: valid once, until it expires.
#[derive(Serialize, Deserialize, Clone)]
struct Invite {
    secret: [u8; 16],
    #[serde(default = "contributor")]
    role: MemberRole,
    album: AlbumId,
    expires_ms: Option<i64>,
    used: bool,
}

#[derive(Serialize, Deserialize, Default)]
struct RegFile {
    albums: Vec<Reg>,
    #[serde(default)]
    invites: Vec<Invite>,
}

/// How long a joiner that presented a valid invitation may fetch what the album needs while it
/// is still becoming a member.
const TAG_INVITE: u8 = 0;
const TAG_MOVED: u8 = 1;
const GUEST_WINDOW: Duration = Duration::from_secs(30 * 60);

struct Entry {
    album: Arc<DocsAlbum>,
    reg: Mutex<Reg>,
    last_sync: Mutex<Option<Instant>>,
    /// Blobs whose download failed recently: when to try again.
    cooling: Mutex<BTreeMap<Blake3Hash, Instant>>,
}

struct Inner {
    node: Node,
    backend: DocsBackend,
    clock: Arc<dyn Clock>,
    path: PathBuf,
    entries: Mutex<BTreeMap<AlbumId, Arc<Entry>>>,
    wake: Notify,
    notices: broadcast::Sender<AlbumNotice>,
    invites: Mutex<Vec<Invite>>,
    guests: Mutex<HashMap<iroh::EndpointId, Instant>>,
}

/// The shared albums of one device.
#[derive(Clone)]
pub struct SharedAlbums {
    inner: Arc<Inner>,
}

/// One row of the albums list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SharedSummary {
    pub id: AlbumId,
    pub name: String,
    pub role: Option<MemberRole>,
    pub item_count: usize,
    pub member_count: usize,
    pub keep_originals: bool,
    pub location: String,
    /// Photos other members added since this device last looked.
    pub unread: usize,
    pub cover: Option<Blake3Hash>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberInfo {
    pub member: Member,
    pub me: bool,
}

pub fn encode_invite(t: &InviteTicket) -> String {
    let mut body = postcard::to_stdvec(t).expect("invite encodes");
    let sum = blake3::hash(&[INVITE_PREFIX.as_bytes(), &body].concat());
    body.extend(&sum.as_bytes()[..4]);
    format!(
        "{INVITE_PREFIX}{}",
        data_encoding::BASE32_NOPAD.encode(&body)
    )
}

pub fn decode_invite(text: &str) -> Result<InviteTicket> {
    let text: String = text
        .split_whitespace()
        .collect::<String>()
        .to_ascii_uppercase();
    let rest = text
        .strip_prefix(INVITE_PREFIX)
        .ok_or_else(|| anyhow!("this is not an album invitation"))?;
    let mut bytes = data_encoding::BASE32_NOPAD
        .decode(rest.as_bytes())
        .map_err(|_| anyhow!("the invitation is garbled"))?;
    ensure!(bytes.len() > 4, "the invitation is cut short");
    let sum = bytes.split_off(bytes.len() - 4);
    let want = blake3::hash(&[INVITE_PREFIX.as_bytes(), &bytes].concat());
    ensure!(
        sum == want.as_bytes()[..4],
        "the invitation is cut short or was changed"
    );
    postcard::from_bytes(&bytes).map_err(|e| anyhow!("the invitation is malformed: {e}"))
}

/// The blob a member sees for `kind`: a stripped copy replaces the original.
pub fn item_blob(item: &SharedItem, kind: BlobKind) -> Blake3Hash {
    match kind {
        BlobKind::Thumb => item.thumb.hash,
        BlobKind::Preview => item.preview.hash,
        BlobKind::Original => item.shared_hash.unwrap_or(item.original.hash),
    }
}

fn read_registry(path: &Path) -> Result<RegFile> {
    match std::fs::read(path) {
        Ok(b) => serde_json::from_slice(&b).context("reading shared-albums.json"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(RegFile::default()),
        Err(e) => Err(e.into()),
    }
}

impl SharedAlbums {
    /// Reopen the albums remembered in `dir` and start syncing them. `background = false` skips
    /// the sync and download loop (tests drive them by hand).
    pub async fn open(node: Node, dir: &Path, background: bool) -> Result<Self> {
        let clock: Arc<dyn Clock> = Arc::new(SystemClock);
        let backend = DocsBackend::for_node(node.clone(), SystemClock);
        let (notices, _) = broadcast::channel(256);
        let this = Self {
            inner: Arc::new(Inner {
                node,
                backend,
                clock,
                path: dir.join("shared-albums.json"),
                entries: Mutex::new(BTreeMap::new()),
                wake: Notify::new(),
                notices,
                invites: Mutex::new(vec![]),
                guests: Mutex::new(HashMap::new()),
            }),
        };
        let file = read_registry(&this.inner.path)?;
        *this.inner.invites.lock().unwrap() = file.invites;
        *this.inner.node.album_slot().lock().unwrap() =
            Some(Arc::new(InviteHandler(Arc::downgrade(&this.inner))));
        for reg in file.albums {
            match this.inner.backend.open_album(&reg.capability).await {
                Ok(album) => this.install(Arc::new(album), reg),
                Err(e) => warn!("could not reopen a shared album: {e}"),
            }
        }
        this.inner.refresh_gate();
        if background {
            tokio::spawn(run(Arc::downgrade(&this.inner)));
        }
        Ok(this)
    }

    fn install(&self, album: Arc<DocsAlbum>, reg: Reg) {
        let id = reg.album;
        let entry = Arc::new(Entry {
            album: album.clone(),
            reg: Mutex::new(reg),
            last_sync: Mutex::new(None),
            cooling: Mutex::new(BTreeMap::new()),
        });
        self.inner.entries.lock().unwrap().insert(id, entry);
        // Forward the album's changes (and wake the loop: there may be new photos to fetch).
        let weak = Arc::downgrade(&self.inner);
        let mut changes = album.subscribe();
        tokio::spawn(async move {
            while futures::StreamExt::next(&mut changes).await.is_some() {
                let Some(inner) = weak.upgrade() else { break };
                inner.refresh_gate();
                let _ = inner.notices.send(AlbumNotice(id));
                inner.wake.notify_one();
            }
        });
    }

    fn save(&self) -> Result<()> {
        let albums: Vec<Reg> = self
            .inner
            .entries
            .lock()
            .unwrap()
            .values()
            .map(|e| e.reg.lock().unwrap().clone())
            .collect();
        let tmp = self.inner.path.with_extension("json.tmp");
        let invites = self.inner.invites.lock().unwrap().clone();
        std::fs::write(
            &tmp,
            serde_json::to_vec_pretty(&RegFile { albums, invites })?,
        )?;
        std::fs::rename(tmp, &self.inner.path)?;
        Ok(())
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AlbumNotice> {
        self.inner.notices.subscribe()
    }

    fn entry(&self, id: &AlbumId) -> Result<Arc<Entry>> {
        self.inner
            .entries
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow!("this device is not part of that shared album"))
    }

    pub fn album(&self, id: &AlbumId) -> Result<Arc<DocsAlbum>> {
        Ok(self.entry(id)?.album.clone())
    }

    /// Start a new shared album; this device is its owner.
    pub async fn create(&self, name: &str, my_name: &str) -> Result<AlbumId> {
        let name = name.trim();
        ensure!(!name.is_empty(), "an album needs a name");
        let mut id = [0u8; 16];
        getrandom::fill(&mut id).expect("operating system randomness");
        let meta = SharedAlbumMeta {
            id: AlbumId(id),
            name: name.to_string(),
            cover: None,
            created_by: self.inner.backend.device(),
        };
        let album = self.inner.backend.create_album(meta).await?;
        let _ = album.set_display_name(my_name).await;
        let reg = Reg {
            album: AlbumId(id),
            capability: album.capability().await?,
            inviter: None,
            keep_originals: false,
            location: "ask".into(),
            seen_ms: self.inner.clock.now_ms(),
            viewers: vec![],
            pending_moves: vec![],
        };
        self.install(Arc::new(album), reg);
        self.save()?;
        Ok(AlbumId(id))
    }

    /// An invitation others can use to join: the text to share (also shown as a QR code).
    pub async fn invite(&self, id: &AlbumId, role: MemberRole, days: u32) -> Result<String> {
        let album = self.album(id)?;
        let expiry = Expiry::At(shoebox_core::Timestamp(
            self.inner.clock.now_ms() as i64 + days as i64 * 86_400_000,
        ));
        let mut ticket = album.invite(role, expiry).await.map_err(|e| match e {
            StoreError::ReadOnly => anyhow!("only the owner of an album can invite people"),
            other => anyhow!("{other}"),
        })?;
        // Each invitation has its own secret, so it can be used once and then refused.
        getrandom::fill(&mut ticket.secret).expect("operating system randomness");
        {
            let mut invites = self.inner.invites.lock().unwrap();
            let now = self.inner.clock.now_ms() as i64;
            invites.retain(|i| !i.used && i.expires_ms.is_none_or(|t| t > now));
            invites.push(Invite {
                secret: ticket.secret,
                role,
                album: *id,
                expires_ms: match expiry {
                    Expiry::At(t) => Some(t.0),
                    Expiry::Never => None,
                },
                used: false,
            });
        }
        self.save()?;
        Ok(encode_invite(&ticket))
    }

    /// Join the album an invitation points at.
    pub async fn join(&self, text: &str, my_name: &str) -> Result<AlbumId> {
        let ticket = decode_invite(text)?;
        ensure!(
            !self
                .inner
                .entries
                .lock()
                .unwrap()
                .contains_key(&ticket.album),
            "you are already in this album"
        );
        // The inviter may not be in this library's device list: let it through for the join.
        self.inner.node.add_peer_addr(
            shoebox_node::to_endpoint_addr(&ticket.inviter).map_err(|e| anyhow!("{e}"))?,
        );
        let mut gate = self.inner.gate_ids();
        gate.insert(to_iroh_id(&ticket.inviter.id).map_err(|e| anyhow!("{e}"))?);
        self.inner.node.set_album_peers(gate);
        if let Expiry::At(t) = ticket.expires {
            ensure!(
                self.inner.clock.now_ms() as i64 <= t.0,
                "this invitation has expired: ask for a new one"
            );
        }
        self.inner.present_invitation(&ticket).await?;
        let album = self
            .inner
            .backend
            .join_album(&ticket)
            .await
            .map_err(|e| match e {
                StoreError::Expired => anyhow!("this invitation has expired: ask for a new one"),
                StoreError::Unreachable(_) => anyhow!(
                    "could not reach the person who invited you: both of you need to be online"
                ),
                other => anyhow!("{other}"),
            })?;
        let _ = album.set_display_name(my_name).await;
        let id = album.meta().await?.id;
        let reg = Reg {
            album: id,
            capability: album.capability().await?,
            inviter: Some(ticket.inviter.clone()),
            keep_originals: false,
            location: "ask".into(),
            seen_ms: self.inner.clock.now_ms(),
            viewers: vec![],
            pending_moves: vec![],
        };
        self.install(Arc::new(album), reg);
        self.save()?;
        self.inner.wake.notify_one();
        Ok(id)
    }

    /// Remove a member (owner only): the album moves to a new space without them and everyone who
    /// stays is told where it went. The removed member keeps what they already downloaded.
    pub async fn remove_member(&self, id: &AlbumId, member: &EndpointId) -> Result<()> {
        let e = self.entry(id)?;
        let me = self.inner.backend.device();
        ensure!(
            e.album.my_role().await == Some(MemberRole::Owner),
            "only the owner can remove people"
        );
        ensure!(
            *member != me,
            "you cannot remove yourself: leave the album instead"
        );
        let members = self.members(id).await?;
        ensure!(
            members.iter().any(|m| m.member.key == *member),
            "that person is not in this album"
        );
        let moved = Arc::new(self.inner.backend.move_album(&e.album, &[*member]).await?);
        let mut reg = e.reg.lock().unwrap().clone();
        reg.capability = moved.capability().await?;
        reg.viewers.retain(|v| v != member);
        reg.pending_moves = members
            .iter()
            .filter(|m| m.member.key != me && m.member.key != *member)
            .map(|m| (m.member.key, m.member.role != MemberRole::Viewer))
            .collect();
        self.install(moved, reg);
        self.save()?;
        if let Ok(gone) = to_iroh_id(member) {
            // Their invitation window (if still open) must not outlive their membership.
            self.inner.guests.lock().unwrap().remove(&gone);
        }
        self.inner.refresh_gate();
        self.inner.wake.notify_one();
        let _ = self.inner.notices.send(AlbumNotice(*id));
        Ok(())
    }

    /// Stop being part of an album on this device (what was downloaded may be freed later).
    pub async fn leave(&self, id: &AlbumId) -> Result<()> {
        let entry = self.entry(id)?;
        let items = entry.album.items(Page::all()).await?;
        self.inner.entries.lock().unwrap().remove(id);
        self.save()?;
        for item in items {
            for h in [
                item.thumb.hash,
                item.preview.hash,
                item_blob(&item, BlobKind::Original),
            ] {
                let _ = self
                    .inner
                    .node
                    .blobs()
                    .unprotect(&h, Protection::Album(*id))
                    .await;
            }
        }
        self.inner.refresh_gate();
        Ok(())
    }

    pub async fn list(&self) -> Result<Vec<SharedSummary>> {
        let entries: Vec<Arc<Entry>> = self
            .inner
            .entries
            .lock()
            .unwrap()
            .values()
            .cloned()
            .collect();
        let me = self.inner.backend.device();
        let mut out = vec![];
        for e in entries {
            let reg = e.reg.lock().unwrap().clone();
            let meta = e.album.meta().await?;
            let events = e.album.events(None).await?;
            let unread = events
                .iter()
                .filter(|ev| ev.member != me && ev.hlc.physical_ms > reg.seen_ms)
                .map(|ev| match ev.kind {
                    AlbumEventKind::ItemsAdded { count } => count as usize,
                    _ => 0,
                })
                .sum();
            let first = e
                .album
                .items(Page {
                    after: None,
                    limit: 1,
                })
                .await?;
            out.push(SharedSummary {
                id: meta.id,
                name: meta.name,
                role: e.album.my_role().await,
                item_count: e.album.item_count().await,
                member_count: e.album.members().await?.len(),
                keep_originals: reg.keep_originals,
                location: reg.location,
                unread,
                cover: first.first().map(|i| i.thumb.hash),
            });
        }
        out.sort_by_key(|a| a.name.to_lowercase());
        Ok(out)
    }

    pub async fn members(&self, id: &AlbumId) -> Result<Vec<MemberInfo>> {
        let me = self.inner.backend.device();
        let e = self.entry(id)?;
        let mut out: Vec<MemberInfo> = e
            .album
            .members()
            .await?
            .into_iter()
            .map(|m| MemberInfo {
                me: m.key == me,
                member: m,
            })
            .collect();
        let viewers = e.reg.lock().unwrap().viewers.clone();
        for v in viewers {
            if !out.iter().any(|m| m.member.key == v) {
                out.push(MemberInfo {
                    me: false,
                    member: Member {
                        key: v,
                        display_name: format!("Viewer {}", hex::encode(&v.0[..4])),
                        role: MemberRole::Viewer,
                    },
                });
            }
        }
        Ok(out)
    }

    pub async fn events(&self, id: &AlbumId, since: Option<Hlc>) -> Result<Vec<AlbumEvent>> {
        Ok(self.album(id)?.events(since).await?)
    }

    /// Mark everything in the album as seen.
    pub fn mark_seen(&self, id: &AlbumId) -> Result<()> {
        let e = self.entry(id)?;
        e.reg.lock().unwrap().seen_ms = self.inner.clock.now_ms();
        self.save()
    }

    pub fn settings(&self, id: &AlbumId) -> Result<(bool, String)> {
        let e = self.entry(id)?;
        let r = e.reg.lock().unwrap();
        Ok((r.keep_originals, r.location.clone()))
    }

    pub fn set_keep_originals(&self, id: &AlbumId, keep: bool) -> Result<()> {
        self.entry(id)?.reg.lock().unwrap().keep_originals = keep;
        self.save()?;
        self.inner.wake.notify_one();
        Ok(())
    }

    pub fn set_location_policy(&self, id: &AlbumId, policy: &str) -> Result<()> {
        ensure!(
            matches!(policy, "ask" | "keep" | "remove"),
            "unknown location setting `{policy}`"
        );
        self.entry(id)?.reg.lock().unwrap().location = policy.to_string();
        self.save()
    }

    /// Add photos. The blobs they refer to must already be in this device's blob store; they are
    /// held for the album from now on.
    pub async fn add_items(&self, id: &AlbumId, items: Vec<SharedItem>) -> Result<()> {
        let album = self.album(id)?;
        for item in &items {
            for h in [
                item.thumb.hash,
                item.preview.hash,
                item_blob(item, BlobKind::Original),
            ] {
                if self.inner.node.blobs().has(&h).await? {
                    self.inner
                        .node
                        .blobs()
                        .protect(&h, Protection::Album(*id))
                        .await?;
                }
            }
        }
        album.add_items(items).await.map_err(|e| match e {
            StoreError::ReadOnly => anyhow!("you can only look at this album, not add to it"),
            other => anyhow!("{other}"),
        })?;
        self.inner.wake.notify_one();
        Ok(())
    }

    /// Sync one album with its members now.
    pub async fn sync_now(&self, id: &AlbumId) -> Result<()> {
        let e = self.entry(id)?;
        self.inner.sync_entry(&e).await;
        Ok(())
    }

    /// Download one blob of an album item right now (the user opened it).
    pub async fn fetch_now(&self, id: &AlbumId, hash: &Blake3Hash) -> Result<()> {
        let e = self.entry(id)?;
        self.inner.fetch_from_members(&e, hash).await
    }

    /// Whether the blob is on this device.
    pub async fn has(&self, hash: &Blake3Hash) -> Result<bool> {
        self.inner.node.blobs().has(hash).await
    }

    /// Wait until every wanted blob of the album is local, or the time is up (for tests and for
    /// "finish downloading").
    pub async fn fetch_all(&self, id: &AlbumId) -> Result<usize> {
        let e = self.entry(id)?;
        self.inner.fetch_missing(&e, true).await
    }
}

impl Inner {
    /// Everyone this album knows: members that wrote their own entry, and viewers.
    fn member_ids(e: &Entry) -> Vec<EndpointId> {
        let mut ids = e.album.known_member_ids();
        ids.extend(e.reg.lock().unwrap().viewers.iter().copied());
        ids
    }

    /// Members of every album plus the inviters: the peers allowed to fetch blobs from us.
    fn gate_ids(&self) -> HashSet<iroh::EndpointId> {
        let entries: Vec<Arc<Entry>> = self.entries.lock().unwrap().values().cloned().collect();
        let mut ids = HashSet::new();
        for e in entries {
            for id in Self::member_ids(&e) {
                if let Ok(i) = to_iroh_id(&id) {
                    ids.insert(i);
                }
            }
            if let Some(inv) = &e.reg.lock().unwrap().inviter {
                if let Ok(i) = to_iroh_id(&inv.id) {
                    ids.insert(i);
                }
            }
        }
        {
            let mut guests = self.guests.lock().unwrap();
            guests.retain(|_, until| *until > Instant::now());
            ids.extend(guests.keys().copied());
        }
        ids.remove(&self.node.id());
        ids
    }

    /// Show the inviter our invitation so it lets us fetch the album's data (it does not know
    /// us yet). Fails with the reason the inviter gives: expired, already used, unknown.
    async fn present_invitation(&self, t: &InviteTicket) -> Result<()> {
        let addr = to_endpoint_addr(&t.inviter).map_err(|e| anyhow!("{e}"))?;
        let go = async {
            let conn = self.node.endpoint().connect(addr, ALBUM_ALPN).await?;
            let (mut send, mut recv) = conn.open_bi().await?;
            send.write_all(&[&[TAG_INVITE][..], &t.secret].concat())
                .await?;
            send.finish()?;
            let reply = recv.read_to_end(256).await?;
            conn.close(0u32.into(), b"done");
            Ok::<Vec<u8>, anyhow::Error>(reply)
        };
        let reply = tokio::time::timeout(Duration::from_secs(20), go)
            .await
            .map_err(|_| ())
            .and_then(|r| r.map_err(|_| ()))
            .map_err(|_| {
                anyhow!("could not reach the person who invited you: both of you need to be online")
            })?;
        match reply.as_slice() {
            b"ok" => Ok(()),
            other => bail!("{}", String::from_utf8_lossy(other)),
        }
    }

    /// The inviter's side: check the invitation and let the sender through.
    fn accept_invitation(
        &self,
        from: iroh::EndpointId,
        secret: [u8; 16],
    ) -> Result<(), &'static str> {
        let now = self.clock.now_ms() as i64;
        let (album, role) = {
            let mut invites = self.invites.lock().unwrap();
            let Some(inv) = invites.iter_mut().find(|i| i.secret == secret) else {
                return Err(
                    "this invitation is not known here (it may have been used or withdrawn)",
                );
            };
            if inv.used {
                return Err("this invitation has already been used: ask for a new one");
            }
            if inv.expires_ms.is_some_and(|t| t <= now) {
                return Err("this invitation has expired: ask for a new one");
            }
            if !self.entries.lock().unwrap().contains_key(&inv.album) {
                return Err("this album no longer exists here");
            }
            inv.used = true;
            (inv.album, inv.role)
        };
        if role == MemberRole::Viewer {
            // A viewer cannot write its own membership entry: remember who it is.
            if let Some(e) = self.entries.lock().unwrap().get(&album) {
                let who = shoebox_node::from_iroh_id(&from);
                let mut reg = e.reg.lock().unwrap();
                if !reg.viewers.contains(&who) {
                    reg.viewers.push(who);
                }
            }
        }
        self.guests
            .lock()
            .unwrap()
            .insert(from, Instant::now() + GUEST_WINDOW);
        self.refresh_gate();
        Ok(())
    }

    /// The owner tells us the album moved to a new space: follow it (only the owner may say so).
    async fn accept_moved(self: &Arc<Self>, from: iroh::EndpointId, body: &[u8]) -> Result<()> {
        let moved: Moved =
            postcard::from_bytes(body).map_err(|e| anyhow!("malformed notice: {e}"))?;
        let entry = self
            .entries
            .lock()
            .unwrap()
            .get(&moved.album)
            .cloned()
            .ok_or_else(|| anyhow!("not a member of that album"))?;
        let sender = shoebox_node::from_iroh_id(&from);
        let owner = entry
            .album
            .members()
            .await?
            .iter()
            .any(|m| m.key == sender && m.role == MemberRole::Owner);
        ensure!(owner, "only the owner can move an album");
        let album = Arc::new(self.backend.open_album(&moved.capability).await?);
        let mut reg = entry.reg.lock().unwrap().clone();
        reg.capability = moved.capability;
        reg.inviter = Some(moved.owner);
        reg.pending_moves = vec![];
        reg.viewers = vec![];
        let shared = SharedAlbums {
            inner: self.clone(),
        };
        shared.install(album, reg);
        shared.save()?;
        self.refresh_gate();
        let _ = self.notices.send(AlbumNotice(moved.album));
        self.wake.notify_one();
        Ok(())
    }

    /// Tell the members who have not heard yet where the album went.
    async fn push_moves(&self, e: &Arc<Entry>) {
        let (album, pending) = {
            let reg = e.reg.lock().unwrap();
            if reg.pending_moves.is_empty() {
                return;
            }
            (reg.album, reg.pending_moves.clone())
        };
        let (Ok(write), Ok(read)) = (
            e.album.capability_for(true).await,
            e.album.capability_for(false).await,
        ) else {
            return;
        };
        let owner = shoebox_node::from_endpoint_addr(&self.node.addr());
        for (who, writer) in pending {
            let notice = Moved {
                album,
                capability: if writer { write.clone() } else { read.clone() },
                owner: owner.clone(),
            };
            let Ok(addr) = to_endpoint_addr(&PeerAddr::id_only(who)) else {
                continue;
            };
            let go = async {
                let conn = self.node.endpoint().connect(addr, ALBUM_ALPN).await?;
                let (mut send, mut recv) = conn.open_bi().await?;
                let mut msg = vec![TAG_MOVED];
                msg.extend(postcard::to_stdvec(&notice)?);
                send.write_all(&msg).await?;
                send.finish()?;
                let reply = recv.read_to_end(1024).await?;
                conn.close(0u32.into(), b"done");
                Ok::<bool, anyhow::Error>(reply == b"ok")
            };
            if matches!(
                tokio::time::timeout(Duration::from_secs(15), go).await,
                Ok(Ok(true))
            ) {
                e.reg
                    .lock()
                    .unwrap()
                    .pending_moves
                    .retain(|(w, _)| *w != who);
                let _ = self.save_registry(); // saves the registry as a whole
            }
        }
    }

    fn save_registry(&self) -> Result<()> {
        let albums: Vec<Reg> = self
            .entries
            .lock()
            .unwrap()
            .values()
            .map(|e| e.reg.lock().unwrap().clone())
            .collect();
        let invites = self.invites.lock().unwrap().clone();
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(
            &tmp,
            serde_json::to_vec_pretty(&RegFile { albums, invites })?,
        )?;
        std::fs::rename(tmp, &self.path)?;
        Ok(())
    }

    fn refresh_gate(&self) {
        self.node.set_album_peers(self.gate_ids());
    }

    fn peers_of(&self, e: &Entry) -> Vec<PeerAddr> {
        let me = self.backend.device();
        let mut seen: HashSet<EndpointId> = HashSet::from([me]);
        let mut out = vec![];
        if let Some(inv) = e.reg.lock().unwrap().inviter.clone() {
            if seen.insert(inv.id) {
                out.push(inv);
            }
        }
        for id in Self::member_ids(e) {
            if seen.insert(id) {
                out.push(PeerAddr::id_only(id));
            }
        }
        out
    }

    async fn sync_entry(&self, e: &Arc<Entry>) {
        for peer in self.peers_of(e) {
            match tokio::time::timeout(SYNC_TIMEOUT, e.album.sync_with(&peer)).await {
                Ok(Ok(())) => {}
                Ok(Err(err)) => debug!("album sync with {} failed: {err}", peer.id),
                Err(_) => debug!("album sync with {} timed out", peer.id),
            }
        }
        *e.last_sync.lock().unwrap() = Some(Instant::now());
    }

    async fn fetch_from_members(&self, e: &Arc<Entry>, hash: &Blake3Hash) -> Result<()> {
        let id = e.reg.lock().unwrap().album;
        if self.node.blobs().has(hash).await? {
            self.node
                .blobs()
                .protect(hash, Protection::Album(id))
                .await?;
            return Ok(());
        }
        let peers = self.peers_of(e);
        ensure!(
            !peers.is_empty(),
            "no other member of this album is known yet"
        );
        let mut last = anyhow!("no member could be reached");
        for p in peers {
            let iroh = to_iroh_id(&p.id).map_err(|e| anyhow!("{e}"))?;
            match self
                .node
                .fetch_blob(iroh, hash, Protection::Album(id))
                .await
            {
                Ok(()) => return Ok(()),
                Err(err) => last = err,
            }
        }
        Err(last)
    }

    /// Fetch what this device should hold. Returns how many blobs arrived.
    async fn fetch_missing(&self, e: &Arc<Entry>, ignore_cooldown: bool) -> Result<usize> {
        let (id, keep) = {
            let r = e.reg.lock().unwrap();
            (r.album, r.keep_originals)
        };
        let items = e.album.items(Page::all()).await?;
        let mut want: Vec<Blake3Hash> = vec![];
        for it in &items {
            want.push(it.thumb.hash);
            want.push(it.preview.hash);
            if keep {
                want.push(item_blob(it, BlobKind::Original));
            }
        }
        let now = Instant::now();
        let mut missing = vec![];
        for h in want {
            if self.node.blobs().has(&h).await? {
                // Hold it for the album (a photo the library also has keeps both reasons).
                let _ = self.node.blobs().protect(&h, Protection::Album(id)).await;
                continue;
            }
            let cooling = e.cooling.lock().unwrap().get(&h).is_some_and(|t| *t > now);
            if ignore_cooldown || !cooling {
                missing.push(h);
            }
        }
        if missing.is_empty() {
            return Ok(0);
        }
        let permits = Arc::new(Semaphore::new(4));
        let mut joins = vec![];
        for h in missing {
            let permit = permits.clone().acquire_owned().await?;
            let (this_e, node) = (e.clone(), self.node.clone());
            let peers = self.peers_of(e);
            joins.push(tokio::spawn(async move {
                let _p = permit;
                for p in peers {
                    let Ok(iroh) = to_iroh_id(&p.id) else {
                        continue;
                    };
                    if node
                        .fetch_blob(iroh, &h, Protection::Album(id))
                        .await
                        .is_ok()
                    {
                        return true;
                    }
                }
                this_e
                    .cooling
                    .lock()
                    .unwrap()
                    .insert(h, Instant::now() + FETCH_RETRY);
                false
            }));
        }
        let mut got = 0;
        for j in joins {
            if j.await.unwrap_or(false) {
                got += 1;
            }
        }
        if got > 0 {
            let _ = self.notices.send(AlbumNotice(id));
        }
        Ok(got)
    }
}

async fn run(weak: Weak<Inner>) {
    loop {
        let Some(inner) = weak.upgrade() else { return };
        let _ = tokio::time::timeout(TICK, inner.wake.notified()).await;
        let entries: Vec<Arc<Entry>> = inner.entries.lock().unwrap().values().cloned().collect();
        for e in entries {
            let due = e
                .last_sync
                .lock()
                .unwrap()
                .is_none_or(|t| t.elapsed() >= RESYNC);
            inner.push_moves(&e).await;
            if due {
                inner.sync_entry(&e).await;
            }
            if let Err(err) = inner.fetch_missing(&e, false).await {
                warn!("fetching album photos failed: {err:#}");
            }
        }
        drop(inner);
    }
}

struct InviteHandler(Weak<Inner>);

impl ConnHandler for InviteHandler {
    fn handle(
        &self,
        conn: iroh::endpoint::Connection,
    ) -> futures::future::BoxFuture<'static, Result<()>> {
        let weak = self.0.clone();
        Box::pin(async move {
            let (mut send, mut recv) = conn.accept_bi().await?;
            let bytes = recv.read_to_end(64 * 1024).await?;
            let reply: Vec<u8> = match (weak.upgrade(), bytes.split_first()) {
                (Some(inner), Some((&TAG_INVITE, secret))) => match <[u8; 16]>::try_from(secret) {
                    Ok(secret) => match inner.accept_invitation(conn.remote_id(), secret) {
                        Ok(()) => {
                            let _ = inner.save_registry();
                            b"ok".to_vec()
                        }
                        Err(why) => why.as_bytes().to_vec(),
                    },
                    Err(_) => b"malformed invitation".to_vec(),
                },
                (Some(inner), Some((&TAG_MOVED, body))) => {
                    match inner.accept_moved(conn.remote_id(), body).await {
                        Ok(()) => b"ok".to_vec(),
                        Err(e) => format!("{e:#}").into_bytes(),
                    }
                }
                _ => b"this device cannot take invitations right now".to_vec(),
            };
            send.write_all(&reply).await?;
            send.finish()?;
            conn.closed().await;
            Ok(())
        })
    }
}
