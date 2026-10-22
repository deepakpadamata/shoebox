//! In-memory `LibraryStore`: a thin wrapper over the shared [`LibraryState`].

use crate::net::MemNetwork;
use async_trait::async_trait;
use futures::channel::mpsc::{unbounded, UnboundedSender};
use futures::stream::BoxStream;
use shoebox_core::clock::Clock;
use shoebox_core::ids::{Blake3Hash, EndpointId, Timestamp};
use shoebox_core::library_state::{Effect, LibraryState};
use shoebox_core::model::*;
use shoebox_core::store::*;
use std::sync::{Arc, Mutex};

pub(crate) struct LibShared {
    state: Mutex<LibraryState>,
    subs: Mutex<Vec<UnboundedSender<Change>>>,
}

impl LibShared {
    pub(crate) fn new(device: EndpointId, root: RootPublicKey, library: LibraryId) -> Self {
        Self {
            state: Mutex::new(LibraryState::with_root(device, root, library)),
            subs: Mutex::new(Vec::new()),
        }
    }

    fn emit(&self, changes: Vec<Change>) {
        let mut subs = self.subs.lock().unwrap();
        for change in changes {
            subs.retain(|tx| tx.unbounded_send(change.clone()).is_ok());
        }
    }
}

/// A personal library replica held in memory.
pub struct MemLibrary {
    shared: Arc<LibShared>,
    network: MemNetwork,
    library: LibraryId,
    clock: Arc<dyn Clock>,
    mode: Capability,
}

impl MemLibrary {
    pub(crate) fn new(
        shared: Arc<LibShared>,
        network: MemNetwork,
        library: LibraryId,
        clock: Arc<dyn Clock>,
        mode: Capability,
    ) -> Self {
        Self {
            shared,
            network,
            library,
            clock,
            mode,
        }
    }

    fn writable(&self) -> Result<()> {
        match self.mode {
            Capability::Write => Ok(()),
            Capability::Read => Err(StoreError::ReadOnly),
        }
    }

    fn now(&self) -> u64 {
        self.clock.now_ms()
    }

    /// Run `f` under the state lock, then publish the changes it reports.
    fn with_state<T>(&self, f: impl FnOnce(&mut LibraryState) -> Result<(T, Effect)>) -> Result<T> {
        let (out, effect) = f(&mut self.shared.state.lock().unwrap())?;
        self.shared.emit(effect.changes);
        Ok(out)
    }

    fn read<T>(&self, f: impl FnOnce(&LibraryState) -> T) -> T {
        f(&self.shared.state.lock().unwrap())
    }

    fn peer(&self, peer: &PeerAddr) -> Result<Arc<LibShared>> {
        self.network
            .0
            .lock()
            .unwrap()
            .libs
            .get(&(self.library, peer.id))
            .cloned()
            .ok_or(StoreError::Unreachable(peer.id))
    }
}

#[async_trait]
impl LibraryStore for MemLibrary {
    async fn upsert_photo(&self, rec: PhotoRecord) -> Result<()> {
        self.writable()?;
        self.with_state(|s| Ok(((), s.upsert_photo(&rec))))
    }

    async fn photo(&self, id: &PhotoId) -> Result<Option<PhotoRecord>> {
        Ok(self.read(|s| s.photo(id)))
    }

    async fn photos_by_time(&self, range: TimeRange, page: Page) -> Result<Vec<PhotoRecord>> {
        Ok(self.read(|s| s.photos_by_time(range, page)))
    }

    async fn deleted_photos(&self) -> Result<Vec<PhotoRecord>> {
        Ok(self.read(|s| s.deleted_photos()))
    }

    async fn set_favorite(&self, id: &PhotoId, fav: bool) -> Result<()> {
        self.writable()?;
        let now = self.now();
        self.with_state(|s| Ok(((), s.set_favorite(id, fav, now)?)))
    }

    async fn mark_deleted(&self, id: &PhotoId) -> Result<()> {
        self.writable()?;
        let now = self.now();
        self.with_state(|s| Ok(((), s.mark_deleted(id, now)?)))
    }

    async fn restore_photo(&self, id: &PhotoId) -> Result<()> {
        self.writable()?;
        let now = self.now();
        self.with_state(|s| Ok(((), s.restore_photo(id, now)?)))
    }

    async fn purge_expired(&self, now: Timestamp) -> Result<Vec<PhotoId>> {
        self.writable()?;
        let clock_now = self.now();
        self.with_state(|s| Ok(s.purge_expired(now, clock_now)))
    }

    async fn upsert_album(&self, album: AlbumMeta) -> Result<()> {
        self.writable()?;
        self.with_state(|s| Ok(((), s.upsert_album(&album))))
    }

    async fn album_add(&self, album: &AlbumId, items: &[PhotoId]) -> Result<()> {
        self.writable()?;
        let now = self.now();
        self.with_state(|s| Ok(((), s.album_add(album, items, now)?)))
    }

    async fn album_remove(&self, album: &AlbumId, items: &[PhotoId]) -> Result<()> {
        self.writable()?;
        self.with_state(|s| Ok(((), s.album_remove(album, items)?)))
    }

    async fn album_items(&self, album: &AlbumId, page: Page) -> Result<Vec<PhotoId>> {
        self.read(|s| s.album_items(album, page))
    }

    async fn albums(&self) -> Result<Vec<AlbumSummary>> {
        Ok(self.read(|s| s.albums()))
    }

    async fn device_list(&self) -> Result<SignedDeviceList> {
        Ok(self.read(|s| s.device_list()))
    }

    async fn apply_device_change(&self, change: SignedDeviceChange) -> Result<()> {
        self.writable()?;
        self.with_state(|s| Ok(((), s.apply_device_change(&change)?)))
    }

    async fn put_holdings(&self, device: EndpointId, delta: HoldingsDelta) -> Result<()> {
        self.writable()?;
        self.with_state(|s| Ok(((), s.put_holdings(device, &delta)?)))
    }

    async fn holders(&self, blob: &Blake3Hash) -> Result<Vec<Holding>> {
        Ok(self.read(|s| s.holders(blob)))
    }

    async fn at_risk(&self, min_copies: u8, page: Page) -> Result<Vec<PhotoId>> {
        Ok(self.read(|s| s.at_risk(min_copies, page)))
    }

    async fn setting(&self, key: &str) -> Result<Option<Vec<u8>>> {
        Ok(self.read(|s| s.setting(key)))
    }

    async fn put_setting(&self, key: &str, value: Vec<u8>) -> Result<()> {
        self.writable()?;
        let now = self.now();
        self.with_state(|s| Ok(((), s.put_setting(key, value, now)?)))
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
        // Phase 1: pull. Phase 2: push. Never hold both locks at once.
        let remote = theirs.state.lock().unwrap().clone();
        let effect = self.shared.state.lock().unwrap().merge_from(&remote);
        self.shared.emit(effect.changes);
        let mine = self.shared.state.lock().unwrap().clone();
        let effect = theirs.state.lock().unwrap().merge_from(&mine);
        theirs.emit(effect.changes);
        Ok(())
    }

    fn capability(&self) -> Capability {
        self.mode
    }
}
