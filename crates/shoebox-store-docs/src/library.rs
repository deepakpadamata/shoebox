//! `LibraryStore` on an iroh-docs namespace.
//!
//! Replicated records live as docs entries (see [`crate::codec`]); queries are answered from a
//! merged in-memory [`LibraryState`] that is rebuilt from the entries at open and kept current by
//! applying local writes and (debounced) rescans after remote inserts.

use crate::codec::*;
use crate::replica::Replica;
use async_trait::async_trait;
use futures::channel::mpsc::{unbounded, UnboundedSender};
use futures::stream::BoxStream;
use futures::StreamExt;
use iroh_docs::engine::LiveEvent;
use shoebox_core::clock::Clock;
use shoebox_core::ids::{Blake3Hash, EndpointId, Timestamp};
use shoebox_core::library_state::{Effect, LibraryState};
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
    state: LibraryState,
    replica: Replica,
}

struct Inner {
    core: AsyncMutex<Core>,
    subs: Mutex<Vec<UnboundedSender<Change>>>,
    clock: Arc<dyn Clock>,
    mode: Capability,
}

impl Inner {
    fn emit(&self, changes: Vec<Change>) {
        let mut subs = self.subs.lock().unwrap();
        for change in changes {
            subs.retain(|tx| tx.unbounded_send(change.clone()).is_ok());
        }
    }

    /// Fold new or changed docs entries into the state. `announce` controls change events
    /// (off for the initial load).
    /// Returns how many entries are still waiting for their content.
    async fn refresh(&self, announce: bool) -> Result<usize> {
        let started = std::time::Instant::now();
        let mut core = self.core.lock().await;
        let scan = core.replica.scan().await.map_err(backend_err)?;
        tracing::debug!(entries = scan.entries.len(), pending = scan.pending, took = ?started.elapsed(), "library scan");
        let (entries, pending) = (scan.entries, scan.pending);
        let mut changes = vec![];
        for (key, bytes) in entries {
            match decode_library_entry(&key, &bytes) {
                Ok(entry) => changes.extend(core.state.apply_entry(&entry).changes),
                Err(e) => warn!("ignoring invalid library entry `{key}`: {e}"),
            }
        }
        drop(core);
        if announce {
            self.emit(changes);
        }
        Ok(pending)
    }

    /// Rescan until nothing is waiting on content (bounded: content a peer cannot supply must
    /// not block forever).
    async fn settle(&self) -> Result<()> {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while self.refresh(true).await? > 0 && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        Ok(())
    }
}

/// A personal library replica on iroh-docs.
pub struct DocsLibrary {
    inner: Arc<Inner>,
}

impl DocsLibrary {
    /// Open an already-imported namespace: load its entries and start watching for remote ones.
    pub(crate) async fn open(
        replica: Replica,
        device: EndpointId,
        clock: Arc<dyn Clock>,
        mode: Capability,
        trust: (RootPublicKey, LibraryId),
    ) -> Result<Self> {
        let events = replica.doc.subscribe().await.map_err(backend_err)?;
        let inner = Arc::new(Inner {
            core: AsyncMutex::new(Core {
                state: LibraryState::with_root(device, trust.0, trust.1),
                replica,
            }),
            subs: Mutex::new(vec![]),
            clock,
            mode,
        });
        let (dirty, peers) = {
            let core = inner.core.lock().await;
            (core.replica.dirty(), core.replica.peers_handle())
        };
        inner.refresh(false).await?;
        spawn_watcher(&inner, events, dirty, peers);
        Ok(Self { inner })
    }

    fn writable(&self) -> Result<()> {
        match self.inner.mode {
            Capability::Write => Ok(()),
            Capability::Read => Err(StoreError::ReadOnly),
        }
    }

    fn now(&self) -> u64 {
        self.inner.clock.now_ms()
    }

    /// Mutate the state, persist exactly the records the write touched, then announce changes.
    async fn write<T>(
        &self,
        f: impl FnOnce(&mut LibraryState) -> Result<(T, Effect)>,
    ) -> Result<T> {
        self.writable()?;
        let mut core = self.inner.core.lock().await;
        let (out, effect) = f(&mut core.state)?;
        for touched in &effect.touched {
            let key = touched_key(touched);
            match core.state.entry(touched) {
                Some(entry) => {
                    let bytes = encode_library_entry(&entry);
                    core.replica.put(&key, bytes).await.map_err(backend_err)?;
                }
                None => core.replica.del(&key).await.map_err(backend_err)?,
            }
        }
        drop(core);
        self.inner.emit(effect.changes);
        Ok(out)
    }

    async fn read<T>(&self, f: impl FnOnce(&LibraryState) -> T) -> T {
        f(&self.inner.core.lock().await.state)
    }
}

/// Rescan (debounced) whenever peers deliver entries or content; stops when the library drops.
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
            // Remember who we exchanged entries with: they are where missing content comes from.
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
                warn!("library rescan failed: {e}");
            }
        }
    });
}

#[async_trait]
impl LibraryStore for DocsLibrary {
    async fn upsert_photo(&self, rec: PhotoRecord) -> Result<()> {
        self.write(|s| Ok(((), s.upsert_photo(&rec)))).await
    }

    async fn photo(&self, id: &PhotoId) -> Result<Option<PhotoRecord>> {
        Ok(self.read(|s| s.photo(id)).await)
    }

    async fn photos_by_time(&self, range: TimeRange, page: Page) -> Result<Vec<PhotoRecord>> {
        Ok(self.read(|s| s.photos_by_time(range, page)).await)
    }

    async fn deleted_photos(&self) -> Result<Vec<PhotoRecord>> {
        Ok(self.read(|s| s.deleted_photos()).await)
    }

    async fn set_favorite(&self, id: &PhotoId, fav: bool) -> Result<()> {
        let now = self.now();
        self.write(|s| Ok(((), s.set_favorite(id, fav, now)?)))
            .await
    }

    async fn mark_deleted(&self, id: &PhotoId) -> Result<()> {
        let now = self.now();
        self.write(|s| Ok(((), s.mark_deleted(id, now)?))).await
    }

    async fn restore_photo(&self, id: &PhotoId) -> Result<()> {
        let now = self.now();
        self.write(|s| Ok(((), s.restore_photo(id, now)?))).await
    }

    async fn purge_expired(&self, now: Timestamp) -> Result<Vec<PhotoId>> {
        let clock_now = self.now();
        self.write(|s| Ok(s.purge_expired(now, clock_now))).await
    }

    async fn upsert_album(&self, album: AlbumMeta) -> Result<()> {
        self.write(|s| Ok(((), s.upsert_album(&album)))).await
    }

    async fn album_add(&self, album: &AlbumId, items: &[PhotoId]) -> Result<()> {
        let now = self.now();
        self.write(|s| Ok(((), s.album_add(album, items, now)?)))
            .await
    }

    async fn album_remove(&self, album: &AlbumId, items: &[PhotoId]) -> Result<()> {
        self.write(|s| Ok(((), s.album_remove(album, items)?)))
            .await
    }

    async fn album_items(&self, album: &AlbumId, page: Page) -> Result<Vec<PhotoId>> {
        self.read(|s| s.album_items(album, page)).await
    }

    async fn albums(&self) -> Result<Vec<AlbumSummary>> {
        Ok(self.read(|s| s.albums()).await)
    }

    async fn device_list(&self) -> Result<SignedDeviceList> {
        Ok(self.read(|s| s.device_list()).await)
    }

    async fn apply_device_change(&self, change: SignedDeviceChange) -> Result<()> {
        self.write(|s| Ok(((), s.apply_device_change(&change)?)))
            .await
    }

    async fn put_holdings(&self, device: EndpointId, delta: HoldingsDelta) -> Result<()> {
        self.write(|s| Ok(((), s.put_holdings(device, &delta)?)))
            .await
    }

    async fn holders(&self, blob: &Blake3Hash) -> Result<Vec<Holding>> {
        Ok(self.read(|s| s.holders(blob)).await)
    }

    async fn at_risk(&self, min_copies: u8, page: Page) -> Result<Vec<PhotoId>> {
        Ok(self.read(|s| s.at_risk(min_copies, page)).await)
    }

    async fn setting(&self, key: &str) -> Result<Option<Vec<u8>>> {
        Ok(self.read(|s| s.setting(key)).await)
    }

    async fn put_setting(&self, key: &str, value: Vec<u8>) -> Result<()> {
        let now = self.now();
        self.write(|s| Ok(((), s.put_setting(key, value, now)?)))
            .await
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
        // Do not hold the state lock while waiting on the network.
        let mut events = doc.subscribe().await.map_err(backend_err)?;
        node.note_activity();
        crate::replica::sync_and_wait(&doc, &node, &mut events, target.clone())
            .await
            .map_err(|e| {
                warn!("sync with {} failed: {e:#}", peer.id);
                StoreError::Unreachable(peer.id)
            })?;
        self.inner.settle().await
    }

    fn capability(&self) -> Capability {
        self.inner.mode
    }
}
