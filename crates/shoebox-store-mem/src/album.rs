//! In-memory `AlbumStore`.

use crate::net::MemNetwork;
use async_trait::async_trait;
use futures::channel::mpsc::{unbounded, UnboundedSender};
use futures::stream::BoxStream;
use shoebox_core::clock::Clock;
use shoebox_core::hlc::{Hlc, HlcClock};
use shoebox_core::ids::EndpointId;
use shoebox_core::model::*;
use shoebox_core::store::*;
use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub(crate) struct AlbumState {
    meta: SharedAlbumMeta,
    items: BTreeMap<PhotoId, SharedItem>,
    /// Per member the entry with the greatest HLC wins.
    members: BTreeMap<EndpointId, (Hlc, Member)>,
    events: BTreeMap<Hlc, AlbumEvent>,
    hlc: HlcClock,
    max_seen: Hlc,
    /// Invites issued by *this* replica (not replicated): secret → (role, expiry).
    invites: BTreeMap<[u8; 16], (MemberRole, Expiry)>,
}

pub(crate) struct AlbumShared {
    state: Mutex<AlbumState>,
    subs: Mutex<Vec<UnboundedSender<Change>>>,
}

/// What a joiner copies from the inviter.
pub(crate) struct AlbumSnapshot(AlbumState);

fn item_key(i: &SharedItem) -> impl Ord {
    (
        i.added_at,
        i.added_by,
        i.original.clone(),
        i.shared_hash,
        i.preview.clone(),
        i.thumb.clone(),
        i.taken_at,
    )
}

fn member_key(m: &(Hlc, Member)) -> impl Ord + '_ {
    (m.0, &m.1.display_name, m.1.role)
}

impl AlbumState {
    fn new(meta: SharedAlbumMeta, device: EndpointId) -> Self {
        Self {
            meta,
            items: BTreeMap::new(),
            members: BTreeMap::new(),
            events: BTreeMap::new(),
            hlc: HlcClock::new(device),
            max_seen: Hlc {
                physical_ms: 0,
                counter: 0,
                device,
            },
            invites: BTreeMap::new(),
        }
    }

    fn tick(&mut self, now_ms: u64) -> Hlc {
        let seen = self.max_seen;
        let h = self.hlc.observe(now_ms, &seen);
        self.max_seen = self.max_seen.max(h);
        h
    }

    fn role_of(&self, device: EndpointId) -> Option<MemberRole> {
        self.members.get(&device).map(|(_, m)| m.role)
    }

    fn put_member(&mut self, hlc: Hlc, member: Member) -> bool {
        let candidate = (hlc, member);
        match self.members.get(&candidate.1.key) {
            Some(existing) if member_key(existing) >= member_key(&candidate) => false,
            _ => {
                self.members.insert(candidate.1.key, candidate);
                true
            }
        }
    }

    fn merge_from(&mut self, other: &AlbumState) -> bool {
        let mut changed = false;
        for item in other.items.values() {
            match self.items.get(&item.id) {
                Some(existing) if item_key(existing) <= item_key(item) => {}
                _ => {
                    self.items.insert(item.id, item.clone());
                    changed = true;
                }
            }
        }
        for m in other.members.values() {
            changed |= self.put_member(m.0, m.1.clone());
        }
        for (h, e) in &other.events {
            changed |= self.events.insert(*h, e.clone()).is_none();
        }
        self.max_seen = self.max_seen.max(other.max_seen);
        changed
    }
}

impl AlbumShared {
    pub(crate) fn new(meta: SharedAlbumMeta, device: EndpointId) -> Self {
        Self {
            state: Mutex::new(AlbumState::new(meta, device)),
            subs: Mutex::new(vec![]),
        }
    }

    pub(crate) fn from_snapshot(snapshot: AlbumSnapshot, device: EndpointId) -> Self {
        let mut state = snapshot.0;
        state.hlc = HlcClock::new(device);
        state.max_seen = Hlc {
            device,
            ..state.max_seen
        };
        state.invites.clear();
        Self {
            state: Mutex::new(state),
            subs: Mutex::new(vec![]),
        }
    }

    /// Validate and consume-by-copy an invite on the inviter's replica.
    pub(crate) fn redeem(&self, ticket: &InviteTicket) -> Result<AlbumSnapshot> {
        let s = self.state.lock().unwrap();
        match s.invites.get(&ticket.secret) {
            Some((role, expiry)) if *role == ticket.role && *expiry == ticket.expires => {
                Ok(AlbumSnapshot(s.clone()))
            }
            _ => Err(StoreError::Invalid("unknown invite".into())),
        }
    }

    fn emit(&self, id: AlbumId) {
        self.subs
            .lock()
            .unwrap()
            .retain(|tx| tx.unbounded_send(Change::AlbumChanged(id)).is_ok());
    }
}

/// A shared album replica held in memory.
pub struct MemAlbum {
    shared: Arc<AlbumShared>,
    network: MemNetwork,
    id: AlbumId,
    device: EndpointId,
    clock: Arc<dyn Clock>,
}

impl MemAlbum {
    pub(crate) fn new(
        shared: Arc<AlbumShared>,
        network: MemNetwork,
        id: AlbumId,
        device: EndpointId,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            shared,
            network,
            id,
            device,
            clock,
        }
    }

    pub(crate) fn install_owner(&self, now_ms: u64) {
        self.install_member(MemberRole::Owner, now_ms);
    }

    /// Register this device as a member with `role` (and log a join event for non-creators).
    pub(crate) fn install_member(&self, role: MemberRole, now_ms: u64) {
        let mut s = self.shared.state.lock().unwrap();
        let hlc = s.tick(now_ms);
        let member = Member {
            key: self.device,
            display_name: format!("{}", self.device),
            role,
        };
        s.put_member(hlc, member);
        if role != MemberRole::Owner {
            let h = s.tick(now_ms);
            s.events.insert(
                h,
                AlbumEvent {
                    hlc: h,
                    member: self.device,
                    kind: AlbumEventKind::MemberJoined,
                },
            );
        }
    }

    fn peer(&self, peer: &PeerAddr) -> Result<Arc<AlbumShared>> {
        self.network
            .0
            .lock()
            .unwrap()
            .albums
            .get(&(self.id, peer.id))
            .cloned()
            .ok_or(StoreError::Unreachable(peer.id))
    }
}

#[async_trait]
impl AlbumStore for MemAlbum {
    async fn meta(&self) -> Result<SharedAlbumMeta> {
        Ok(self.shared.state.lock().unwrap().meta.clone())
    }

    async fn add_items(&self, items: Vec<SharedItem>) -> Result<()> {
        let now = self.clock.now_ms();
        {
            let mut s = self.shared.state.lock().unwrap();
            match s.role_of(self.device) {
                Some(MemberRole::Owner | MemberRole::Contributor) => {}
                _ => return Err(StoreError::ReadOnly),
            }
            let mut new = 0u32;
            for item in items {
                match s.items.get(&item.id) {
                    Some(existing) if item_key(existing) <= item_key(&item) => {}
                    Some(_) => {
                        s.items.insert(item.id, item);
                    }
                    None => {
                        s.items.insert(item.id, item);
                        new += 1;
                    }
                }
            }
            if new == 0 {
                return Ok(());
            }
            let hlc = s.tick(now);
            s.events.insert(
                hlc,
                AlbumEvent {
                    hlc,
                    member: self.device,
                    kind: AlbumEventKind::ItemsAdded { count: new },
                },
            );
        }
        self.shared.emit(self.id);
        Ok(())
    }

    async fn items(&self, page: Page) -> Result<Vec<SharedItem>> {
        let s = self.shared.state.lock().unwrap();
        let mut items: Vec<&SharedItem> = s.items.values().collect();
        items.sort_by_key(|i| (Reverse(i.added_at), i.id));
        let skip = page
            .after
            .and_then(|c| items.iter().position(|i| i.id == c.id).map(|p| p + 1))
            .unwrap_or(0);
        Ok(items
            .into_iter()
            .skip(skip)
            .take(page.limit)
            .cloned()
            .collect())
    }

    async fn members(&self) -> Result<Vec<Member>> {
        let s = self.shared.state.lock().unwrap();
        Ok(s.members.values().map(|(_, m)| m.clone()).collect())
    }

    async fn events(&self, since: Option<Hlc>) -> Result<Vec<AlbumEvent>> {
        let s = self.shared.state.lock().unwrap();
        Ok(s.events
            .values()
            .filter(|e| since.is_none_or(|h| e.hlc > h))
            .cloned()
            .collect())
    }

    async fn invite(&self, role: MemberRole, expiry: Expiry) -> Result<InviteTicket> {
        {
            let s = self.shared.state.lock().unwrap();
            if s.role_of(self.device) != Some(MemberRole::Owner) {
                return Err(StoreError::ReadOnly);
            }
        }
        let nonce = self.network.nonce(self.device, "invite");
        let mut secret = [0u8; 16];
        secret.copy_from_slice(&nonce[..16]);
        self.shared
            .state
            .lock()
            .unwrap()
            .invites
            .insert(secret, (role, expiry));
        Ok(InviteTicket {
            album: self.id,
            role,
            expires: expiry,
            inviter: PeerAddr::id_only(self.device),
            secret,
        })
    }

    fn subscribe(&self) -> BoxStream<'static, Change> {
        let (tx, rx) = unbounded();
        self.shared.subs.lock().unwrap().push(tx);
        Box::pin(rx)
    }

    async fn sync_with(&self, peer: &PeerAddr) -> Result<()> {
        let theirs = self.peer(peer)?;
        if Arc::ptr_eq(&theirs, &self.shared) {
            return Ok(());
        }
        let remote = theirs.state.lock().unwrap().clone();
        if self.shared.state.lock().unwrap().merge_from(&remote) {
            self.shared.emit(self.id);
        }
        let mine = self.shared.state.lock().unwrap().clone();
        if theirs.state.lock().unwrap().merge_from(&mine) {
            theirs.emit(self.id);
        }
        Ok(())
    }
}
