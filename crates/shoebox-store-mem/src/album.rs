//! In-memory `AlbumStore`: a thin wrapper over the shared [`AlbumState`].

use crate::net::MemNetwork;
use async_trait::async_trait;
use futures::channel::mpsc::{unbounded, UnboundedSender};
use futures::stream::BoxStream;
use shoebox_core::album_state::AlbumState;
use shoebox_core::clock::Clock;
use shoebox_core::hlc::Hlc;
use shoebox_core::ids::EndpointId;
use shoebox_core::model::*;
use shoebox_core::store::*;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

pub(crate) struct AlbumShared {
    state: Mutex<AlbumState>,
    /// Invites issued by *this* replica (not replicated): secret → (role, expiry).
    invites: Mutex<BTreeMap<[u8; 16], (MemberRole, Expiry)>>,
    subs: Mutex<Vec<UnboundedSender<Change>>>,
}

/// What a joiner copies from the inviter.
pub(crate) struct AlbumSnapshot(AlbumState);

impl AlbumShared {
    pub(crate) fn new(meta: SharedAlbumMeta, device: EndpointId) -> Self {
        Self {
            state: Mutex::new(AlbumState::new(meta, device)),
            invites: Mutex::new(BTreeMap::new()),
            subs: Mutex::new(vec![]),
        }
    }

    pub(crate) fn from_snapshot(snapshot: AlbumSnapshot, device: EndpointId) -> Self {
        Self {
            state: Mutex::new(snapshot.0.fork_for(device)),
            invites: Mutex::new(BTreeMap::new()),
            subs: Mutex::new(vec![]),
        }
    }

    /// Validate an invite on the inviter's replica and copy its state.
    pub(crate) fn redeem(&self, ticket: &InviteTicket) -> Result<AlbumSnapshot> {
        match self.invites.lock().unwrap().get(&ticket.secret) {
            Some((role, expiry)) if *role == ticket.role && *expiry == ticket.expires => {
                Ok(AlbumSnapshot(self.state.lock().unwrap().clone()))
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

    pub(crate) fn install_member(&self, role: MemberRole, now_ms: u64) {
        self.shared
            .state
            .lock()
            .unwrap()
            .install_member(role, now_ms);
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
        Ok(self.shared.state.lock().unwrap().meta())
    }

    async fn add_items(&self, items: Vec<SharedItem>) -> Result<()> {
        let now = self.clock.now_ms();
        let fx = self.shared.state.lock().unwrap().add_items(&items, now)?;
        if fx.changed() {
            self.shared.emit(self.id);
        }
        Ok(())
    }

    async fn items(&self, page: Page) -> Result<Vec<SharedItem>> {
        Ok(self.shared.state.lock().unwrap().items(page))
    }

    async fn members(&self) -> Result<Vec<Member>> {
        Ok(self.shared.state.lock().unwrap().members())
    }

    async fn events(&self, since: Option<Hlc>) -> Result<Vec<AlbumEvent>> {
        Ok(self.shared.state.lock().unwrap().events(since))
    }

    async fn invite(&self, role: MemberRole, expiry: Expiry) -> Result<InviteTicket> {
        if self.shared.state.lock().unwrap().my_role() != Some(MemberRole::Owner) {
            return Err(StoreError::ReadOnly);
        }
        let nonce = self.network.nonce(self.device, "invite");
        let mut secret = [0u8; 16];
        secret.copy_from_slice(&nonce[..16]);
        self.shared
            .invites
            .lock()
            .unwrap()
            .insert(secret, (role, expiry));
        Ok(InviteTicket {
            album: self.id,
            role,
            expires: expiry,
            inviter: PeerAddr::id_only(self.device),
            secret,
            backend: vec![],
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
        if self
            .shared
            .state
            .lock()
            .unwrap()
            .merge_from(&remote)
            .changed()
        {
            self.shared.emit(self.id);
        }
        let mine = self.shared.state.lock().unwrap().clone();
        if theirs.state.lock().unwrap().merge_from(&mine).changed() {
            theirs.emit(self.id);
        }
        Ok(())
    }
}
