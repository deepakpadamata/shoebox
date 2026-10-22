//! `AlbumStore` on an iroh-docs namespace (one per shared album).

use crate::codec::*;
use crate::replica::Replica;
use async_trait::async_trait;
use futures::channel::mpsc::{unbounded, UnboundedSender};
use futures::stream::BoxStream;
use futures::StreamExt;
use iroh_docs::engine::LiveEvent;
use shoebox_core::album_state::{AlbumState, MAX_ALBUM_ITEMS, MAX_ALBUM_MEMBERS};
use shoebox_core::clock::Clock;
use shoebox_core::hlc::Hlc;
use shoebox_core::ids::EndpointId;
use shoebox_core::model::*;
use shoebox_core::store::*;
use shoebox_node::to_endpoint_addr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;
use tracing::warn;

fn backend_err(e: impl std::fmt::Display) -> StoreError {
    StoreError::Backend(e.to_string())
}

struct Core {
    state: AlbumState,
    replica: Replica,
}

struct Inner {
    core: AsyncMutex<Core>,
    subs: Mutex<Vec<UnboundedSender<Change>>>,
    id: AlbumId,
    device: EndpointId,
    clock: Arc<dyn Clock>,
    node: shoebox_node::Node,
    /// Whether this replica holds the album's write capability (viewers do not).
    writable: bool,
}

impl Inner {
    fn emit(&self) {
        self.subs
            .lock()
            .unwrap()
            .retain(|tx| tx.unbounded_send(Change::AlbumChanged(self.id)).is_ok());
    }

    /// Returns how many entries are still waiting for their content.
    async fn refresh(&self, announce: bool) -> Result<usize> {
        let mut core = self.core.lock().await;
        let scan = core.replica.scan().await.map_err(backend_err)?;
        let (entries, pending) = (scan.entries, scan.pending);
        let mut changed = false;
        for (key, bytes) in entries {
            match decode_album_entry(&key, &bytes) {
                Ok(entry) => changed |= core.state.apply_entry(&entry).changed(),
                Err(e) => warn!("ignoring invalid album entry `{key}`: {e}"),
            }
        }
        drop(core);
        if announce && changed {
            self.emit();
        }
        Ok(pending)
    }

    /// Rescan until nothing is waiting on content (bounded).
    async fn settle(&self) -> Result<()> {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while self.refresh(true).await? > 0 && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        Ok(())
    }
}

/// A shared album replica on iroh-docs.
pub struct DocsAlbum {
    inner: Arc<Inner>,
}

impl DocsAlbum {
    /// Start a brand-new album owned by `device` (writes its meta and owner membership).
    pub(crate) async fn create(
        replica: Replica,
        meta: SharedAlbumMeta,
        device: EndpointId,
        clock: Arc<dyn Clock>,
        node: shoebox_node::Node,
    ) -> Result<Self> {
        let events = replica.doc.subscribe().await.map_err(backend_err)?;
        let id = meta.id;
        let mut state = AlbumState::new(meta, device);
        state.install_member(MemberRole::Owner, clock.now_ms());
        let album = Self::assemble(replica, state, id, device, clock, node, events, true);
        {
            let mut core = album.inner.core.lock().await;
            for entry in core.state.all_entries() {
                let bytes = encode_album_entry(&entry);
                core.replica
                    .put(&album_key(&entry), bytes)
                    .await
                    .map_err(backend_err)?;
            }
        }
        Ok(album)
    }

    /// Join an album whose namespace has just been imported and synced: read its meta, load
    /// everything, then record this device as a member (unless the capability is read-only).
    pub(crate) async fn join(
        mut replica: Replica,
        role: MemberRole,
        device: EndpointId,
        clock: Arc<dyn Clock>,
        node: shoebox_node::Node,
        inviter: EndpointId,
    ) -> Result<Self> {
        let events = replica.doc.subscribe().await.map_err(backend_err)?;
        let scanned = replica.scan().await.map_err(backend_err)?.entries;
        let meta = scanned
            .iter()
            .find(|(k, _)| k == "meta")
            .and_then(|(k, b)| match decode_album_entry(k, b) {
                Ok(shoebox_core::album_state::AlbumEntry::Meta(m)) => Some(m),
                _ => None,
            })
            .ok_or(StoreError::Unreachable(inviter))?;
        let id = meta.id;
        let mut state = AlbumState::new(meta, device);
        for (key, bytes) in &scanned {
            if let Ok(entry) = decode_album_entry(key, bytes) {
                state.apply_entry(&entry);
            }
        }
        let album = Self::assemble(
            replica,
            state,
            id,
            device,
            clock.clone(),
            node,
            events,
            role != MemberRole::Viewer,
        );
        if role != MemberRole::Viewer {
            // A viewer holds a read-only capability and cannot publish its membership.
            let mut core = album.inner.core.lock().await;
            let effect = core.state.install_member(role, clock.now_ms());
            for t in &effect.touched {
                if let Some(entry) = core.state.entry(t) {
                    let bytes = encode_album_entry(&entry);
                    core.replica
                        .put(&album_key(&entry), bytes)
                        .await
                        .map_err(backend_err)?;
                }
            }
        }
        Ok(album)
    }

    /// Reopen an album this device already belongs to (after a restart): nothing is announced.
    pub(crate) async fn open(
        mut replica: Replica,
        device: EndpointId,
        clock: Arc<dyn Clock>,
        node: shoebox_node::Node,
        writable: bool,
    ) -> Result<Self> {
        let events = replica.doc.subscribe().await.map_err(backend_err)?;
        let scanned = replica.scan().await.map_err(backend_err)?.entries;
        let meta = scanned
            .iter()
            .find(|(k, _)| k == "meta")
            .and_then(|(k, b)| match decode_album_entry(k, b) {
                Ok(shoebox_core::album_state::AlbumEntry::Meta(m)) => Some(m),
                _ => None,
            })
            .ok_or_else(|| StoreError::Invalid("the album has no data on this device".into()))?;
        let id = meta.id;
        let mut state = AlbumState::new(meta, device);
        for (key, bytes) in &scanned {
            if let Ok(entry) = decode_album_entry(key, bytes) {
                state.apply_entry(&entry);
            }
        }
        Ok(Self::assemble(
            replica, state, id, device, clock, node, events, writable,
        ))
    }

    /// Keys of the members this replica knows about (devices to sync with and fetch from).
    pub fn known_member_ids(&self) -> Vec<EndpointId> {
        match self.inner.core.try_lock() {
            Ok(core) => core.state.members().into_iter().map(|m| m.key).collect(),
            Err(_) => vec![],
        }
    }

    pub async fn item_count(&self) -> usize {
        self.inner.core.lock().await.state.item_count()
    }

    /// Whether this device may add photos (owners and contributors).
    pub async fn can_add(&self) -> bool {
        matches!(
            self.inner.core.lock().await.state.my_role(),
            Some(MemberRole::Owner | MemberRole::Contributor)
        )
    }

    pub async fn my_role(&self) -> Option<MemberRole> {
        self.inner.core.lock().await.state.my_role()
    }

    /// Tell the other members what to call this device (a viewer cannot).
    pub async fn set_display_name(&self, name: &str) -> Result<()> {
        let now = self.inner.clock.now_ms();
        let mut core = self.inner.core.lock().await;
        let effect = core.state.set_my_name(name, now)?;
        for t in &effect.touched {
            if let Some(entry) = core.state.entry(t) {
                let bytes = encode_album_entry(&entry);
                core.replica
                    .put(&album_key(&entry), bytes)
                    .await
                    .map_err(backend_err)?;
            }
        }
        drop(core);
        if effect.changed() {
            self.inner.emit();
        }
        Ok(())
    }

    /// The iroh-docs capability (serialised) that lets this device reopen the album later.
    pub async fn capability(&self) -> Result<Vec<u8>> {
        use iroh_docs::api::protocol::{AddrInfoOptions, ShareMode};
        let mode = if self.inner.writable {
            ShareMode::Write
        } else {
            ShareMode::Read
        };
        let core = self.inner.core.lock().await;
        let ticket = core
            .replica
            .doc
            .share(mode, AddrInfoOptions::Id)
            .await
            .map_err(backend_err)?;
        postcard::to_stdvec(&ticket.capability).map_err(backend_err)
    }

    #[allow(clippy::too_many_arguments)]
    fn assemble<S>(
        replica: Replica,
        state: AlbumState,
        id: AlbumId,
        device: EndpointId,
        clock: Arc<dyn Clock>,
        node: shoebox_node::Node,
        events: S,
        writable: bool,
    ) -> Self
    where
        S: futures::Stream<Item = anyhow::Result<LiveEvent>> + Unpin + Send + 'static,
    {
        let (dirty, peers) = (replica.dirty(), replica.peers_handle());
        let inner = Arc::new(Inner {
            core: AsyncMutex::new(Core { state, replica }),
            subs: Mutex::new(vec![]),
            id,
            device,
            clock,
            node,
            writable,
        });
        spawn_watcher(&inner, events, dirty, peers);
        Self { inner }
    }
}

fn spawn_watcher<S>(
    inner: &Arc<Inner>,
    mut events: S,
    dirty: Arc<tokio::sync::Notify>,
    peers: Arc<Mutex<std::collections::HashSet<iroh::EndpointId>>>,
) where
    S: futures::Stream<Item = anyhow::Result<LiveEvent>> + Unpin + Send + 'static,
{
    // iroh-docs delivers events through bounded channels and stalls the *whole engine* if any
    // subscriber stops draining. Rescans need the engine, so draining and rescanning must be
    // separate tasks: this one only notes that something changed.
    let signal = dirty.clone();
    tokio::spawn(async move {
        while let Some(event) = events.next().await {
            match &event {
                Ok(LiveEvent::InsertRemote { from, .. }) => {
                    peers.lock().unwrap().insert(*from);
                }
                Ok(LiveEvent::SyncFinished(ev)) if ev.result.is_ok() => {
                    peers.lock().unwrap().insert(ev.peer);
                }
                _ => {}
            }
            if matches!(
                event,
                Ok(LiveEvent::InsertRemote { .. }
                    | LiveEvent::ContentReady { .. }
                    | LiveEvent::PendingContentReady
                    | LiveEvent::SyncFinished(_))
            ) {
                signal.notify_one();
            }
        }
    });

    // Coalesce bursts (a big sync emits one event per entry) into few rescans.
    let weak = Arc::downgrade(inner);
    tokio::spawn(async move {
        loop {
            dirty.notified().await;
            tokio::time::sleep(Duration::from_millis(50)).await;
            let Some(inner) = weak.upgrade() else { break };
            if let Err(e) = inner.refresh(true).await {
                warn!("album rescan failed: {e}");
            }
        }
    });
}

#[async_trait]
impl AlbumStore for DocsAlbum {
    async fn meta(&self) -> Result<SharedAlbumMeta> {
        Ok(self.inner.core.lock().await.state.meta())
    }

    async fn add_items(&self, items: Vec<SharedItem>) -> Result<()> {
        let now = self.inner.clock.now_ms();
        let mut core = self.inner.core.lock().await;
        let have = core.state.item_count();
        let fresh = items.iter().filter(|i| !core.state.has_item(&i.id)).count();
        if have + fresh > MAX_ALBUM_ITEMS {
            return Err(StoreError::Invalid(format!(
                "a shared album holds at most {MAX_ALBUM_ITEMS} photos"
            )));
        }
        let effect = core.state.add_items(&items, now)?;
        for t in &effect.touched {
            if let Some(entry) = core.state.entry(t) {
                let bytes = encode_album_entry(&entry);
                core.replica
                    .put(&album_key(&entry), bytes)
                    .await
                    .map_err(backend_err)?;
            }
        }
        drop(core);
        if effect.changed() {
            self.inner.emit();
        }
        Ok(())
    }

    async fn items(&self, page: Page) -> Result<Vec<SharedItem>> {
        Ok(self.inner.core.lock().await.state.items(page))
    }

    async fn members(&self) -> Result<Vec<Member>> {
        Ok(self.inner.core.lock().await.state.members())
    }

    async fn events(&self, since: Option<Hlc>) -> Result<Vec<AlbumEvent>> {
        Ok(self.inner.core.lock().await.state.events(since))
    }

    async fn invite(&self, role: MemberRole, expiry: Expiry) -> Result<InviteTicket> {
        use iroh_docs::api::protocol::{AddrInfoOptions, ShareMode};
        let core = self.inner.core.lock().await;
        if core.state.my_role() != Some(MemberRole::Owner) {
            return Err(StoreError::ReadOnly);
        }
        if core.state.members().len() >= MAX_ALBUM_MEMBERS {
            return Err(StoreError::Invalid(format!(
                "a shared album has at most {MAX_ALBUM_MEMBERS} members"
            )));
        }
        // The capability *is* the bearer secret: viewers get a read-only namespace key.
        let mode = if role == MemberRole::Viewer {
            ShareMode::Read
        } else {
            ShareMode::Write
        };
        let doc_ticket = core
            .replica
            .doc
            .share(mode, AddrInfoOptions::Addresses)
            .await
            .map_err(backend_err)?;
        let mut secret = [0u8; 16];
        secret.copy_from_slice(&doc_ticket.capability.id().to_bytes()[..16]);
        Ok(InviteTicket {
            album: self.inner.id,
            role,
            expires: expiry,
            inviter: shoebox_node::from_endpoint_addr(&self.inner.node.addr()),
            secret,
            backend: doc_ticket.to_string().into_bytes(),
        })
    }

    fn subscribe(&self) -> BoxStream<'static, Change> {
        let (tx, rx) = unbounded();
        self.inner.subs.lock().unwrap().push(tx);
        Box::pin(rx)
    }

    async fn sync_with(&self, peer: &PeerAddr) -> Result<()> {
        let target = to_endpoint_addr(peer)?;
        let (doc, node) = {
            let core = self.inner.core.lock().await;
            core.replica.note_peer(target.id);
            (core.replica.doc.clone(), core.replica.node().clone())
        };
        let mut events = doc.subscribe().await.map_err(backend_err)?;
        node.note_activity();
        crate::replica::sync_and_wait(&doc, &node, &mut events, target.clone())
            .await
            .map_err(|_| StoreError::Unreachable(peer.id))?;
        self.inner.settle().await
    }
}

#[allow(dead_code)]
fn _device(inner: &Inner) -> EndpointId {
    inner.device
}
