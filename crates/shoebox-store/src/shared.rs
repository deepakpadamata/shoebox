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
/// The loop looks at every album at least this often (for downloads that failed); it is also
/// woken by album changes, local edits and members connecting.
const TICK: Duration = Duration::from_secs(20);
/// A safety net: compare with every member this often even when nothing announced a change.
/// Changes normally travel at once, since the member who made one syncs it to the others.
const RESYNC: Duration = Duration::from_secs(5 * 60);
const SYNC_TIMEOUT: Duration = Duration::from_secs(25);
const FETCH_RETRY: Duration = Duration::from_secs(20);
/// How long to wait to hear whether a member holds the same as we do (the first contact, so an
/// unreachable member fails here, before any sync is started).
const DIGEST_TIMEOUT: Duration = Duration::from_secs(10);
/// After a member could not be reached: wait this long, doubling on each failure up to
/// `BACKOFF_MAX`. A member connecting to us ends the wait.
const BACKOFF_MIN: Duration = Duration::from_secs(15);
const BACKOFF_MAX: Duration = Duration::from_secs(30 * 60);
/// A member connecting again within this long of our last exchange with it does not start a
/// sync (our own sync opens connections too).
const RECONNECT_QUIET: Duration = Duration::from_secs(60);
/// A viewer asks a writer to record the person's other devices at most this often.
const INTRODUCE_EVERY: Duration = Duration::from_secs(10 * 60);
/// At most this many devices in one introduction.
const MAX_INTRODUCED: usize = 16;

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
    /// Devices that presented one of our invitations, whatever role: peers to sync with even
    /// before their own membership entry has reached us.
    #[serde(default)]
    joined: Vec<EndpointId>,
    /// After the album moved to a new space (a member was removed): who has not yet been told
    /// where it went, and whether they may write.
    #[serde(default)]
    pending_moves: Vec<(EndpointId, bool)>,
    /// The person's other devices in this album (it came to this device through the library):
    /// peers to sync and fetch from even when they have no entry of their own (viewers).
    #[serde(default)]
    own: Vec<EndpointId>,
    /// When `capability` was last set (ms): the newest one wins between the person's devices.
    #[serde(default)]
    cap_ms: u64,
}

/// A shared album as the person's library remembers it, so every device of the person takes
/// part (A-097). Written by the device that created or joined it, read by the others.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AlbumRef {
    pub album: AlbumId,
    /// Serialised iroh-docs capability (changes when the album moves to a new space).
    pub capability: Vec<u8>,
    pub inviter: Option<PeerAddr>,
    pub role: MemberRole,
    /// When `capability` was set (ms): after a move the newer capability wins.
    #[serde(default)]
    pub cap_ms: u64,
    /// When the person created or joined it (ms); a later `left_ms` means they left.
    pub joined_ms: u64,
    #[serde(default)]
    pub left_ms: Option<u64>,
    /// The personal album it was shared from: photos added there are added here too.
    #[serde(default)]
    pub from_album: Option<AlbumId>,
    /// The location choice made when sharing (`keep` or `remove`), used by every device.
    #[serde(default)]
    pub location: Option<String>,
}

/// "These devices are mine too": sent by a viewer to a member who may write, so its person's
/// other devices get an entry in the album and every member lets them sync and fetch (A-152).
#[derive(Serialize, Deserialize)]
struct Introduce {
    album: AlbumId,
    /// The person's name, shown for these devices.
    name: Option<String>,
    person: Option<shoebox_core::store::LibraryId>,
    /// The sender and the person's other devices.
    devices: Vec<EndpointId>,
}

/// What we know about reaching one member.
#[derive(Default)]
struct Reach {
    fails: u32,
    /// Do not try before this (after failures).
    until: Option<Instant>,
    /// When the last exchange with it worked.
    last_ok: Option<Instant>,
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
const TAG_INTRODUCE: u8 = 2;
const GUEST_WINDOW: Duration = Duration::from_secs(30 * 60);

struct Entry {
    album: Arc<DocsAlbum>,
    reg: Mutex<Reg>,
    last_sync: Mutex<Option<Instant>>,
    /// Sync at the next pass (a local edit, a member came back).
    due: std::sync::atomic::AtomicBool,
    /// When this device last introduced the person's devices to a writer, successfully.
    introduced: Mutex<Option<Instant>>,
    /// Blobs whose download failed recently: when to try again.
    cooling: Mutex<BTreeMap<Blake3Hash, Instant>>,
    /// Blobs seen stored and protected for this album since it opened: `fetch_missing` skips
    /// them instead of looking each one up and re-writing its tag every pass. Cleared whenever
    /// this album's protections are dropped (`set_keep_originals(false)`).
    held: Mutex<HashSet<Blake3Hash>>,
}

impl Entry {
    fn mark_due(&self) {
        self.due.store(true, std::sync::atomic::Ordering::Relaxed);
    }
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
    /// Which person (library) this device belongs to, told to every album.
    person: Mutex<Option<shoebox_core::store::LibraryId>>,
    /// The person's name as this device last showed it in an album.
    my_name: Mutex<Option<String>>,
    /// The person's other devices (from the library's device list).
    siblings: Mutex<Vec<EndpointId>>,
    /// Per member device: failures and backoff.
    reach: Mutex<HashMap<EndpointId, Reach>>,
    /// Held while the library's list of the person's shared albums is read, changed and written
    /// back (`library_link`), so two changes on this device do not undo each other.
    list: tokio::sync::Mutex<()>,
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
    /// The newest item's id (a `PhotoId` in the album, not a blob hash).
    pub cover: Option<Blake3Hash>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberInfo {
    pub member: Member,
    pub me: bool,
    /// The person (library) this device said it belongs to.
    pub person: Option<shoebox_core::store::LibraryId>,
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

/// The invitation inside what was pasted: the share sheet sends a sentence with the code at the
/// end, and a message app may wrap the code over lines or add words after it.
pub fn decode_invite(text: &str) -> Result<InviteTicket> {
    let upper = text.to_ascii_uppercase();
    let start = upper
        .find(INVITE_PREFIX)
        .ok_or_else(|| anyhow!("this is not an album invitation"))?;
    let words: Vec<&str> = upper[start..].split_whitespace().collect();
    let mut first_error = None;
    for n in (1..=words.len()).rev() {
        match decode_invite_code(&words[..n].concat()) {
            Ok(t) => return Ok(t),
            Err(e) => {
                first_error.get_or_insert(e);
            }
        }
    }
    Err(first_error.unwrap_or_else(|| anyhow!("this is not an album invitation")))
}

/// `code` is the invitation itself, upper case, without spaces.
fn decode_invite_code(code: &str) -> Result<InviteTicket> {
    let rest = code
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
        // Shared albums have no index shards of their own, and share no videos (yet).
        BlobKind::Original | BlobKind::Index | BlobKind::Motion => {
            item.shared_hash.unwrap_or(item.original.hash)
        }
    }
}

/// The registry holds the keys to the albums (and the invitations' secrets): only this user.
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)?.write_all(bytes)?;
    Ok(())
}

fn read_registry(path: &Path) -> Result<RegFile> {
    match std::fs::read(path) {
        Ok(b) => serde_json::from_slice(&b).context("reading shared-albums.json"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(RegFile::default()),
        Err(e) => Err(e.into()),
    }
}

impl SharedAlbums {
    /// Hold while reading, changing and writing back the library's list of shared albums.
    pub(crate) async fn lock_list(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.inner.list.lock().await
    }

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
                person: Mutex::new(None),
                my_name: Mutex::new(None),
                siblings: Mutex::new(vec![]),
                reach: Mutex::new(HashMap::new()),
                list: tokio::sync::Mutex::new(()),
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
            tokio::spawn(watch_peers(
                Arc::downgrade(&this.inner),
                this.inner.node.events(),
            ));
        }
        Ok(this)
    }

    fn install(&self, album: Arc<DocsAlbum>, reg: Reg) {
        let id = reg.album;
        let entry = Arc::new(Entry {
            album: album.clone(),
            reg: Mutex::new(reg),
            last_sync: Mutex::new(None),
            due: std::sync::atomic::AtomicBool::new(true),
            introduced: Mutex::new(None),
            cooling: Mutex::new(BTreeMap::new()),
            held: Mutex::new(HashSet::new()),
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
        self.inner.save_registry()
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
        let _ = album.set_profile(my_name, self.person()).await;
        let reg = Reg {
            album: AlbumId(id),
            capability: album.capability().await?,
            inviter: None,
            keep_originals: false,
            location: "ask".into(),
            seen_ms: self.inner.clock.now_ms(),
            viewers: vec![],
            joined: vec![],
            pending_moves: vec![],
            own: vec![],
            cap_ms: self.inner.clock.now_ms(),
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
        tracing::debug!("join: invitation accepted by the inviter");
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
        tracing::debug!("join: album opened");
        let _ = album.set_profile(my_name, self.person()).await;
        self.remember_name(my_name);
        tracing::debug!("join: name published");
        let id = album.meta().await?.id;
        let reg = Reg {
            album: id,
            capability: album.capability().await?,
            inviter: Some(ticket.inviter.clone()),
            keep_originals: false,
            location: "ask".into(),
            seen_ms: self.inner.clock.now_ms(),
            viewers: vec![],
            joined: vec![],
            pending_moves: vec![],
            own: vec![],
            cap_ms: self.inner.clock.now_ms(),
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
        let members = self.members(id).await?;
        let person_of = |k: &EndpointId| {
            members
                .iter()
                .find(|m| m.member.key == *k)
                .and_then(|m| m.person)
        };
        ensure!(
            *member != me && (person_of(member).is_none() || person_of(member) != self.person()),
            "you cannot remove yourself: leave the album instead"
        );
        let only_joined = e.reg.lock().unwrap().joined.contains(member);
        ensure!(
            members.iter().any(|m| m.member.key == *member) || only_joined,
            "that person is not in this album"
        );
        // A person goes with all their devices.
        let mut gone = vec![*member];
        if let Some(p) = person_of(member) {
            gone.extend(
                members
                    .iter()
                    .filter(|m| m.person == Some(p) && m.member.key != *member)
                    .map(|m| m.member.key),
            );
        }
        let moved = Arc::new(self.inner.backend.move_album(&e.album, &gone).await?);
        let mut reg = e.reg.lock().unwrap().clone();
        reg.capability = moved.capability().await?;
        reg.cap_ms = self.inner.clock.now_ms();
        reg.viewers.retain(|v| !gone.contains(v));
        reg.joined.retain(|v| !gone.contains(v));
        reg.pending_moves = members
            .iter()
            .filter(|m| m.member.key != me && !gone.contains(&m.member.key))
            .map(|m| (m.member.key, m.member.role != MemberRole::Viewer))
            .collect();
        self.install(moved, reg);
        self.save()?;
        for g in &gone {
            if let Ok(gone) = to_iroh_id(g) {
                // Their invitation window (if still open) must not outlive their membership.
                self.inner.guests.lock().unwrap().remove(&gone);
            }
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
                member_count: e.album.people_count().await,
                keep_originals: reg.keep_originals,
                location: reg.location,
                unread,
                // The item id: the app looks the cover up as `album:<id>:thumb` by it.
                cover: first.first().map(|i| i.id),
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
            .members_with_person()
            .await
            .into_iter()
            .map(|(m, person)| MemberInfo {
                me: m.key == me,
                member: m,
                person,
            })
            .collect();
        let viewers = e.reg.lock().unwrap().viewers.clone();
        for v in viewers {
            if !out.iter().any(|m| m.member.key == v) {
                out.push(MemberInfo {
                    me: false,
                    person: None,
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

    /// Keep every original of the album on this device, or let them go: turning it off drops
    /// the album's hold on the originals (they are freed by the next cleanup unless the library
    /// or a pin needs them).
    pub async fn set_keep_originals(&self, id: &AlbumId, keep: bool) -> Result<()> {
        let e = self.entry(id)?;
        e.reg.lock().unwrap().keep_originals = keep;
        self.save()?;
        if keep {
            self.inner.wake.notify_one();
        } else {
            e.held.lock().unwrap().clear();
            for item in e.album.items(Page::all()).await? {
                let h = item_blob(&item, BlobKind::Original);
                let _ = self
                    .inner
                    .node
                    .blobs()
                    .unprotect(&h, Protection::Album(*id))
                    .await;
            }
        }
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
        self.entry(id)?.mark_due();
        self.inner.wake.notify_one();
        Ok(())
    }

    /// This device got a new name: tell the members of every album (viewers cannot).
    fn person(&self) -> Option<shoebox_core::store::LibraryId> {
        *self.inner.person.lock().unwrap()
    }

    /// Which person (library) this device belongs to; told to every album from now on.
    pub fn set_person(&self, person: shoebox_core::store::LibraryId) {
        *self.inner.person.lock().unwrap() = Some(person);
    }

    /// The person's other devices (the library's device list without this one). A viewer
    /// introduces them to the album's writers, so they may sync and fetch too (A-152).
    pub fn set_siblings(&self, devices: Vec<EndpointId>) {
        let mut s = self.inner.siblings.lock().unwrap();
        if *s != devices {
            *s = devices;
            drop(s);
            for e in self.inner.entries.lock().unwrap().values() {
                *e.introduced.lock().unwrap() = None;
            }
            self.inner.wake.notify_one();
        }
    }

    fn remember_name(&self, name: &str) {
        let name = name.trim();
        if !name.is_empty() {
            *self.inner.my_name.lock().unwrap() = Some(name.to_string());
        }
    }

    /// The albums this device is part of, for the person's library (see [`AlbumRef`]).
    pub async fn refs(&self) -> Vec<AlbumRef> {
        let entries: Vec<Arc<Entry>> = self
            .inner
            .entries
            .lock()
            .unwrap()
            .values()
            .cloned()
            .collect();
        let mut out = vec![];
        for e in entries {
            let reg = e.reg.lock().unwrap().clone();
            out.push(AlbumRef {
                album: reg.album,
                capability: reg.capability,
                inviter: reg.inviter,
                role: e.album.my_role().await.unwrap_or(MemberRole::Viewer),
                cap_ms: reg.cap_ms,
                joined_ms: 0,
                left_ms: None,
                from_album: None,
                location: Some(reg.location.clone()).filter(|l| l != "ask"),
            });
        }
        out
    }

    /// Whether this device is part of `id`.
    pub fn contains(&self, id: &AlbumId) -> bool {
        self.inner.entries.lock().unwrap().contains_key(id)
    }

    /// Take part in an album another device of this person belongs to: its record is fetched
    /// from `peers` (those devices, then the inviter), and the members are told this device is
    /// the same person. Nothing new appears in the album's activity.
    pub async fn adopt(&self, r: &AlbumRef, peers: Vec<PeerAddr>, my_name: &str) -> Result<()> {
        if let Ok(e) = self.entry(&r.album) {
            let (cap, cap_ms) = {
                let reg = e.reg.lock().unwrap();
                (reg.capability.clone(), reg.cap_ms)
            };
            if cap == r.capability || cap_ms >= r.cap_ms {
                return Ok(());
            }
            // The album moved to a new space and this device was not told (it has no entry of
            // its own): follow the person's newer capability. What was downloaded stays.
            self.inner.entries.lock().unwrap().remove(&r.album);
        }
        let mut try_peers = peers.clone();
        try_peers.extend(r.inviter.clone());
        for p in &try_peers {
            if let Ok(addr) = shoebox_node::to_endpoint_addr(p) {
                self.inner.node.add_peer_addr(addr);
            }
        }
        let album = self
            .inner
            .backend
            .adopt_album(&r.capability, r.role, &try_peers)
            .await
            .map_err(|e| match e {
                StoreError::Unreachable(_) => {
                    anyhow!("none of your devices in that album could be reached yet")
                }
                other => anyhow!("{other}"),
            })?;
        let _ = album.set_profile(my_name, self.person()).await;
        self.remember_name(my_name);
        let reg = Reg {
            album: r.album,
            capability: album.capability().await?,
            inviter: r.inviter.clone(),
            keep_originals: false,
            location: r.location.clone().unwrap_or_else(|| "ask".into()),
            seen_ms: self.inner.clock.now_ms(),
            viewers: vec![],
            joined: vec![],
            pending_moves: vec![],
            own: peers.iter().map(|p| p.id).collect(),
            cap_ms: r.cap_ms.max(1),
        };
        self.install(Arc::new(album), reg);
        self.save()?;
        self.inner.refresh_gate();
        self.inner.wake.notify_one();
        Ok(())
    }

    /// Change the name this device shows in every album (the person's name).
    pub async fn rename_me(&self, name: &str) {
        let entries: Vec<Arc<Entry>> = self
            .inner
            .entries
            .lock()
            .unwrap()
            .values()
            .cloned()
            .collect();
        let person = self.person();
        self.remember_name(name);
        for e in entries {
            let _ = e.album.set_profile(name, person).await;
            // A viewer's name is written by a writer: say it again.
            *e.introduced.lock().unwrap() = None;
        }
    }

    /// Sync one album with its members now, trying those that could not be reached lately too
    /// (a viewer first introduces the person's devices, if a writer has not recorded them).
    pub async fn sync_now(&self, id: &AlbumId) -> Result<()> {
        let e = self.entry(id)?;
        *e.introduced.lock().unwrap() = None;
        self.inner.introduce(&e, true).await;
        self.inner.sync_entry(&e, true).await;
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
    /// Everyone this album knows: members that wrote their own entry, viewers, and devices that
    /// presented an invitation (to sync with before their entry reaches us).
    fn member_ids(e: &Entry) -> Vec<EndpointId> {
        let mut ids = Self::confirmed_member_ids(e);
        let reg = e.reg.lock().unwrap();
        ids.extend(reg.joined.iter().copied());
        ids.extend(reg.own.iter().copied());
        ids
    }

    /// Members that wrote their own entry, and viewers: who may fetch blobs from us. A device that
    /// only presented an invitation gets the guest window, not lasting access.
    fn confirmed_member_ids(e: &Entry) -> Vec<EndpointId> {
        let mut ids = e.album.known_member_ids();
        ids.extend(e.reg.lock().unwrap().viewers.iter().copied());
        ids
    }

    /// Members of every album plus the inviters: the peers allowed to fetch blobs from us.
    fn gate_ids(&self) -> HashSet<iroh::EndpointId> {
        let entries: Vec<Arc<Entry>> = self.entries.lock().unwrap().values().cloned().collect();
        let mut ids = HashSet::new();
        for e in entries {
            for id in Self::confirmed_member_ids(&e) {
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
    ) -> Result<(AlbumId, MemberRole), &'static str> {
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
        if let Some(e) = self.entries.lock().unwrap().get(&album) {
            let who = shoebox_node::from_iroh_id(&from);
            let mut reg = e.reg.lock().unwrap();
            if !reg.joined.contains(&who) {
                reg.joined.push(who);
            }
            // A viewer cannot write its own membership entry: remember who it is.
            if role == MemberRole::Viewer && !reg.viewers.contains(&who) {
                reg.viewers.push(who);
            }
        }
        self.guests
            .lock()
            .unwrap()
            .insert(from, Instant::now() + GUEST_WINDOW);
        self.refresh_gate();
        Ok((album, role))
    }

    /// Write a viewer's entry into the album (it cannot), so every member's devices know it.
    async fn record_viewers(
        &self,
        album: &AlbumId,
        devices: &[EndpointId],
        name: Option<&str>,
        person: Option<shoebox_core::store::LibraryId>,
    ) -> Result<()> {
        let e = self
            .entries
            .lock()
            .unwrap()
            .get(album)
            .cloned()
            .ok_or_else(|| anyhow!("not a member of that album"))?;
        for d in devices {
            let name = match name {
                Some(n) => n.to_string(),
                None => format!("Viewer {}", hex::encode(&d.0[..4])),
            };
            e.album
                .record_viewers(std::slice::from_ref(d), &name, person)
                .await
                .map_err(|e| anyhow!("{e}"))?;
        }
        e.mark_due();
        self.refresh_gate();
        self.wake.notify_one();
        Ok(())
    }

    /// A viewer says which other devices are its person's: record them as viewers too. Only a
    /// viewer this album already knows may, and only for its own person; what it gets is no more
    /// than it could pass on itself (A-152).
    async fn accept_introduce(&self, from: iroh::EndpointId, body: &[u8]) -> Result<()> {
        let intro: Introduce =
            postcard::from_bytes(body).map_err(|e| anyhow!("malformed introduction: {e}"))?;
        ensure!(
            intro.devices.len() <= MAX_INTRODUCED,
            "too many devices in one introduction"
        );
        let e = self
            .entries
            .lock()
            .unwrap()
            .get(&intro.album)
            .cloned()
            .ok_or_else(|| anyhow!("not a member of that album"))?;
        ensure!(
            e.album.can_add().await,
            "this device cannot write to the album"
        );
        let sender = shoebox_node::from_iroh_id(&from);
        let known_viewer = e.reg.lock().unwrap().viewers.contains(&sender);
        match e.album.role_of(&sender).await {
            Some(MemberRole::Viewer) => {}
            None if known_viewer => {}
            Some(_) => bail!("members who may write add their own devices"),
            None => bail!("not a member of that album"),
        }
        // The person was said by a writer (or by the sender before): it cannot be changed here.
        if let Some(p) = e.album.person_of(&sender).await {
            ensure!(
                intro.person == Some(p),
                "those devices belong to someone else"
            );
        }
        let mut devices = intro.devices.clone();
        if !devices.contains(&sender) {
            devices.push(sender);
        }
        let name = intro
            .name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty() && n.len() <= 64);
        self.record_viewers(&intro.album, &devices, name, intro.person)
            .await
    }

    /// As a viewer: ask a member who may write to record this device and the person's other
    /// devices, if any of them has no entry yet (or the name changed). The inviter first, then
    /// the other writers.
    async fn introduce(&self, e: &Arc<Entry>, force: bool) {
        if e.album.can_add().await {
            return;
        }
        if e.introduced
            .lock()
            .unwrap()
            .is_some_and(|t| t.elapsed() < INTRODUCE_EVERY)
        {
            return;
        }
        let me = self.backend.device();
        let mut devices = vec![me];
        devices.extend(self.siblings.lock().unwrap().iter().copied());
        devices.truncate(MAX_INTRODUCED);
        let name = self.my_name.lock().unwrap().clone();
        let members = e.album.members_with_person().await;
        let person = *self.person.lock().unwrap();
        let recorded = devices.iter().all(|d| {
            members.iter().any(|(m, p)| {
                m.key == *d
                    && (name.is_none() || name.as_deref() == Some(m.display_name.as_str()))
                    && (person.is_none() || *p == person)
            })
        });
        if recorded {
            *e.introduced.lock().unwrap() = Some(Instant::now());
            return;
        }
        let album = e.reg.lock().unwrap().album;
        let msg = Introduce {
            album,
            name,
            person,
            devices,
        };
        let Ok(body) = postcard::to_stdvec(&msg) else {
            return;
        };
        let mut writers: Vec<PeerAddr> =
            e.reg.lock().unwrap().inviter.clone().into_iter().collect();
        for (m, _) in &members {
            if matches!(m.role, MemberRole::Owner | MemberRole::Contributor)
                && writers.iter().all(|w| w.id != m.key)
            {
                writers.push(PeerAddr::id_only(m.key));
            }
        }
        for w in writers {
            if !force && !self.reachable_now(&w.id) {
                continue;
            }
            match self.ask(&w, TAG_INTRODUCE, &body).await {
                Ok(reply) if reply == b"ok" => {
                    self.note_ok(w.id);
                    *e.introduced.lock().unwrap() = Some(Instant::now());
                    debug!("introduced this person's devices to {}", w.id);
                    return;
                }
                Ok(reply) => {
                    self.note_ok(w.id);
                    debug!(
                        "{} would not record this person's devices: {}",
                        w.id,
                        String::from_utf8_lossy(&reply)
                    );
                }
                Err(err) => {
                    self.note_failure(w.id);
                    debug!("could not reach {} to introduce devices: {err:#}", w.id);
                }
            }
        }
    }

    /// One request over [`ALBUM_ALPN`]: `tag`, `body`, and the reply.
    async fn ask(&self, to: &PeerAddr, tag: u8, body: &[u8]) -> Result<Vec<u8>> {
        let addr = to_endpoint_addr(to).map_err(|e| anyhow!("{e}"))?;
        let go = async {
            let conn = self.node.endpoint().connect(addr, ALBUM_ALPN).await?;
            let (mut send, mut recv) = conn.open_bi().await?;
            let mut msg = vec![tag];
            msg.extend_from_slice(body);
            send.write_all(&msg).await?;
            send.finish()?;
            let reply = recv.read_to_end(1024).await?;
            conn.close(0u32.into(), b"done");
            Ok::<Vec<u8>, anyhow::Error>(reply)
        };
        tokio::time::timeout(Duration::from_secs(15), go)
            .await
            .map_err(|_| anyhow!("timed out"))?
    }

    /// Whether `id` may be tried now (it is not waiting out a failure).
    fn reachable_now(&self, id: &EndpointId) -> bool {
        self.reach
            .lock()
            .unwrap()
            .get(id)
            .and_then(|r| r.until)
            .is_none_or(|t| t <= Instant::now())
    }

    fn note_ok(&self, id: EndpointId) {
        let mut reach = self.reach.lock().unwrap();
        let r = reach.entry(id).or_default();
        r.fails = 0;
        r.until = None;
        r.last_ok = Some(Instant::now());
    }

    fn note_failure(&self, id: EndpointId) {
        let mut reach = self.reach.lock().unwrap();
        let r = reach.entry(id).or_default();
        r.fails = r.fails.saturating_add(1);
        let wait = BACKOFF_MIN
            .saturating_mul(1u32 << (r.fails - 1).min(16))
            .min(BACKOFF_MAX);
        r.until = Some(Instant::now() + wait);
    }

    /// A device connected to us. If it is a member we could not reach, or have not talked to for
    /// a while, sync the albums it is in now.
    fn peer_connected(&self, id: EndpointId) {
        let stale = {
            let mut reach = self.reach.lock().unwrap();
            let r = reach.entry(id).or_default();
            let failed = r.fails > 0;
            r.fails = 0;
            r.until = None;
            failed || r.last_ok.is_none_or(|t| t.elapsed() >= RECONNECT_QUIET)
        };
        if !stale {
            return;
        }
        let entries: Vec<Arc<Entry>> = self.entries.lock().unwrap().values().cloned().collect();
        let mut any = false;
        for e in entries {
            if self.peers_of(&e).iter().any(|p| p.id == id) {
                e.mark_due();
                *e.introduced.lock().unwrap() = None;
                any = true;
            }
        }
        if any {
            self.wake.notify_one();
        }
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
        // Only the owner invites, so the inviter we joined through is the owner. Membership
        // entries cannot decide this: any writer can forge one that says "Owner" (A-078). The
        // owner's own device has no inviter, so nobody can move an album away from its owner.
        let sender = shoebox_node::from_iroh_id(&from);
        let owner = entry
            .reg
            .lock()
            .unwrap()
            .inviter
            .as_ref()
            .is_some_and(|inv| inv.id == sender);
        ensure!(owner, "only the owner can move an album");
        let album = Arc::new(self.backend.open_album(&moved.capability).await?);
        let mut reg = entry.reg.lock().unwrap().clone();
        reg.capability = moved.capability;
        reg.cap_ms = self.clock.now_ms();
        reg.inviter = Some(moved.owner);
        reg.pending_moves = vec![];
        reg.viewers = vec![];
        reg.joined = vec![];
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
            if !self.reachable_now(&who) {
                continue;
            }
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
            let told = tokio::time::timeout(Duration::from_secs(15), go).await;
            match &told {
                Ok(Ok(_)) => self.note_ok(who),
                _ => self.note_failure(who),
            }
            if matches!(told, Ok(Ok(true))) {
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
        write_private(
            &tmp,
            &serde_json::to_vec_pretty(&RegFile { albums, invites })?,
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

    /// Sync with every member that can be reached: first ask whether it holds the same (one
    /// small request, which also finds out whether it is there at all), then sync only if not.
    /// A member that cannot be reached is left alone for a while (`note_failure`); `force`
    /// tries it anyway (the user asked).
    async fn sync_entry(&self, e: &Arc<Entry>, force: bool) {
        e.due.store(false, std::sync::atomic::Ordering::Relaxed);
        for peer in self.peers_of(e) {
            if !force && !self.reachable_now(&peer.id) {
                continue;
            }
            let Ok(iroh_id) = to_iroh_id(&peer.id) else {
                continue;
            };
            if let Ok(addr) = to_endpoint_addr(&peer) {
                self.node.add_peer_addr(addr);
            }
            match tokio::time::timeout(DIGEST_TIMEOUT, e.album.same_as(iroh_id)).await {
                Ok(Ok(Some(true))) => {
                    self.note_ok(peer.id);
                    continue;
                }
                // Reachable, but it does not have the album (yet): nothing to exchange.
                Ok(Ok(None)) => {
                    debug!("{} does not have the album open", peer.id);
                    self.note_ok(peer.id);
                    continue;
                }
                Ok(Ok(Some(false))) => {}
                Ok(Err(err)) => {
                    self.note_failure(peer.id);
                    debug!("album member {} unreachable: {err}", peer.id);
                    continue;
                }
                Err(_) => {
                    self.note_failure(peer.id);
                    debug!("album member {} did not answer", peer.id);
                    continue;
                }
            }
            match tokio::time::timeout(SYNC_TIMEOUT, e.album.sync_with(&peer)).await {
                Ok(Ok(())) => self.note_ok(peer.id),
                Ok(Err(err)) => debug!("album sync with {} failed: {err}", peer.id),
                Err(_) => debug!("album sync with {} timed out", peer.id),
            }
        }
        // Entries may have come in through syncs the members started, so a digest can match
        // before the album has read them.
        if let Err(err) = e.album.settle().await {
            debug!("reading the album after a sync failed: {err}");
        }
        *e.last_sync.lock().unwrap() = Some(Instant::now());
    }

    /// The members to fetch from now: those not waiting out a failure.
    fn fetch_peers(&self, e: &Entry) -> Vec<iroh::EndpointId> {
        self.peers_of(e)
            .iter()
            .filter(|p| self.reachable_now(&p.id))
            .filter_map(|p| to_iroh_id(&p.id).ok())
            .collect()
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
        // Not every member holds every photo: one member at a time, each continuing from what
        // the last one sent (members are not known to hold it, so no split, A-099).
        let ids: Vec<_> = peers
            .iter()
            .filter_map(|p| to_iroh_id(&p.id).ok())
            .collect();
        self.node
            .fetch_blob_from(&ids, hash, None, Protection::Album(id))
            .await?;
        Ok(())
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
            if e.held.lock().unwrap().contains(&h) {
                continue;
            }
            if self.node.blobs().has(&h).await? {
                // Hold it for the album (a photo the library also has keeps both reasons).
                if self
                    .node
                    .blobs()
                    .protect(&h, Protection::Album(id))
                    .await
                    .is_ok()
                {
                    e.held.lock().unwrap().insert(h);
                }
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
        let peers = self.fetch_peers(e);
        if peers.is_empty() && !ignore_cooldown {
            // Nobody to ask right now; a member connecting wakes the loop.
            return Ok(0);
        }
        let permits = Arc::new(Semaphore::new(4));
        let mut joins = vec![];
        for h in missing {
            let permit = permits.clone().acquire_owned().await?;
            let (this_e, node) = (e.clone(), self.node.clone());
            let ids = peers.clone();
            joins.push(tokio::spawn(async move {
                let _p = permit;
                if !ids.is_empty()
                    && node
                        .fetch_blob_from(&ids, &h, None, Protection::Album(id))
                        .await
                        .is_ok()
                {
                    return true;
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

/// The album loop: no polling of members. An album syncs when it opens, after a local edit,
/// when a member connects that we could not reach or had not talked to for a while, and as a
/// safety net every [`RESYNC`]. Members that cannot be reached are left alone with a growing
/// backoff until they connect to us (A-153).
async fn run(weak: Weak<Inner>) {
    loop {
        let Some(inner) = weak.upgrade() else { return };
        let _ = tokio::time::timeout(TICK, inner.wake.notified()).await;
        let entries: Vec<Arc<Entry>> = inner.entries.lock().unwrap().values().cloned().collect();
        for e in entries {
            let due = e.due.load(std::sync::atomic::Ordering::Relaxed)
                || e.last_sync
                    .lock()
                    .unwrap()
                    .is_none_or(|t| t.elapsed() >= RESYNC);
            inner.push_moves(&e).await;
            inner.introduce(&e, false).await;
            if due {
                inner.sync_entry(&e, false).await;
            }
            if let Err(err) = inner.fetch_missing(&e, false).await {
                warn!("fetching album photos failed: {err:#}");
            }
        }
        drop(inner);
    }
}

/// Turn members connecting to us into syncs (see [`Inner::peer_connected`]).
async fn watch_peers(weak: Weak<Inner>, mut events: shoebox_node::NodeEvents) {
    loop {
        match events.recv().await {
            Ok(shoebox_node::NodeEvent::PeerConnected { id }) => {
                let Some(inner) = weak.upgrade() else { return };
                inner.peer_connected(shoebox_node::from_iroh_id(&id));
            }
            Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {
                if weak.strong_count() == 0 {
                    return;
                }
            }
            Err(broadcast::error::RecvError::Closed) => return,
        }
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
                        Ok((album, role)) => {
                            let _ = inner.save_registry();
                            if role == MemberRole::Viewer {
                                // Its entry, so this person's other devices know it too. The
                                // joiner's album is not open yet: no need to wait for this.
                                let (inner, who) =
                                    (inner.clone(), shoebox_node::from_iroh_id(&conn.remote_id()));
                                tokio::spawn(async move {
                                    if let Err(e) =
                                        inner.record_viewers(&album, &[who], None, None).await
                                    {
                                        debug!("could not record a new viewer: {e:#}");
                                    }
                                });
                            }
                            b"ok".to_vec()
                        }
                        Err(why) => why.as_bytes().to_vec(),
                    },
                    Err(_) => b"malformed invitation".to_vec(),
                },
                (Some(inner), Some((&TAG_INTRODUCE, body))) => {
                    match inner.accept_introduce(conn.remote_id(), body).await {
                        Ok(()) => b"ok".to_vec(),
                        Err(e) => format!("{e:#}").into_bytes(),
                    }
                }
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
            // The peer closes once it has read the reply; do not wait on it for longer than that.
            let _ = tokio::time::timeout(Duration::from_secs(5), conn.closed()).await;
            Ok(())
        })
    }
}
