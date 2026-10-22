use crate::backend::LoroNetwork;
use async_trait::async_trait;
use futures::channel::mpsc::{unbounded, UnboundedSender};
use futures::stream::BoxStream;
use loro::{ExportMode, LoroDoc, LoroValue};
use shoebox_core::clock::Clock;
use shoebox_core::entry_codec::{decode_library_entry, encode_library_entry, touched_key};
use shoebox_core::ids::{Blake3Hash, EndpointId, Timestamp};
use shoebox_core::library_state::{Effect, LibraryState};
use shoebox_core::model::*;
use shoebox_core::store::*;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Name of the Loro map holding every replicated record.
const ENTRIES: &str = "entries";

pub(crate) struct Inner {
    state: LibraryState,
    doc: LoroDoc,
    author: String,
    /// Records already folded into `state`, by key, with a fingerprint of their bytes.
    known: HashMap<String, [u8; 32]>,
}

pub(crate) struct LibraryShared {
    inner: Mutex<Inner>,
    subs: Mutex<Vec<UnboundedSender<Change>>>,
}

impl LibraryShared {
    pub(crate) fn new(device: EndpointId, root: RootPublicKey, library: LibraryId) -> Self {
        let doc = LoroDoc::new();
        // A stable Loro peer id per device keeps its operations in one lane.
        let peer = u64::from_le_bytes(device.0[..8].try_into().expect("8 bytes"));
        let _ = doc.set_peer_id(peer.max(1));
        Self {
            inner: Mutex::new(Inner {
                state: LibraryState::with_root(device, root, library),
                doc,
                author: hex_short(&device),
                known: HashMap::new(),
            }),
            subs: Mutex::new(vec![]),
        }
    }

    fn emit(&self, changes: Vec<Change>) {
        let mut subs = self.subs.lock().unwrap();
        for change in changes {
            subs.retain(|tx| tx.unbounded_send(change.clone()).is_ok());
        }
    }
}

fn hex_short(id: &EndpointId) -> String {
    id.0.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

fn fingerprint(bytes: &[u8]) -> [u8; 32] {
    Blake3Hash::of_bytes(bytes).0
}

impl Inner {
    /// Write the records a mutation touched into the Loro document.
    fn persist(&mut self, effect: &Effect) {
        let map = self.doc.get_map(ENTRIES);
        for touched in &effect.touched {
            let key = format!("{}/{}", self.author, touched_key(touched));
            match self.state.entry(touched) {
                Some(entry) => {
                    let bytes = encode_library_entry(&entry);
                    self.known.insert(key.clone(), fingerprint(&bytes));
                    let _ = map.insert(&key, LoroValue::Binary(bytes.into()));
                }
                None => {
                    self.known.remove(&key);
                    let _ = map.delete(&key);
                }
            }
        }
        self.doc.commit();
    }

    /// Fold new or changed records of the Loro document into the state.
    fn rescan(&mut self) -> Vec<Change> {
        let map = self.doc.get_map(ENTRIES);
        let mut fresh: Vec<(String, Vec<u8>)> = vec![];
        map.for_each(|key, value| {
            if let Some(loro::ValueOrContainer::Value(LoroValue::Binary(bytes))) = Some(value) {
                if self.known.get(key) != Some(&fingerprint(&bytes)) {
                    fresh.push((key.to_string(), bytes.to_vec()));
                }
            }
        });
        let mut changes = vec![];
        for (key, bytes) in fresh {
            // Keys are `<author>/<record key>`; the record key is validated against the value.
            if let Some((_, record_key)) = key.split_once('/') {
                if let Ok(entry) = decode_library_entry(record_key, &bytes) {
                    changes.extend(self.state.apply_entry(&entry).changes);
                }
            }
            self.known.insert(key, fingerprint(&bytes));
        }
        changes
    }
}

/// A personal library replica on a Loro document.
pub struct LoroLibrary {
    shared: Arc<LibraryShared>,
    network: LoroNetwork,
    library: LibraryId,
    clock: Arc<dyn Clock>,
    mode: Capability,
}

impl LoroLibrary {
    pub(crate) fn new(
        shared: Arc<LibraryShared>,
        network: LoroNetwork,
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

    fn write<T>(&self, f: impl FnOnce(&mut LibraryState) -> Result<(T, Effect)>) -> Result<T> {
        self.writable()?;
        let mut inner = self.shared.inner.lock().unwrap();
        let (out, effect) = f(&mut inner.state)?;
        inner.persist(&effect);
        drop(inner);
        self.shared.emit(effect.changes);
        Ok(out)
    }

    fn read<T>(&self, f: impl FnOnce(&LibraryState) -> T) -> T {
        f(&self.shared.inner.lock().unwrap().state)
    }

    /// The whole document as one Loro snapshot (what would be stored as a blob).
    pub fn export_snapshot(&self) -> Vec<u8> {
        self.shared
            .inner
            .lock()
            .unwrap()
            .doc
            .export(ExportMode::Snapshot)
            .expect("snapshot")
    }

    /// Updates a peer at `peer_version` is missing (what an incremental sync would send).
    pub fn export_updates_for(&self, peer_snapshot: &[u8]) -> Vec<u8> {
        let probe = LoroDoc::new();
        let _ = probe.import(peer_snapshot);
        let inner = self.shared.inner.lock().unwrap();
        inner
            .doc
            .export(ExportMode::updates(&probe.oplog_vv()))
            .expect("updates")
    }

    /// Import a snapshot or updates produced by another replica and apply what changed.
    pub fn import(&self, bytes: &[u8]) -> Result<()> {
        let mut inner = self.shared.inner.lock().unwrap();
        inner
            .doc
            .import(bytes)
            .map_err(|e| StoreError::Backend(e.to_string()))?;
        let changes = inner.rescan();
        drop(inner);
        self.shared.emit(changes);
        Ok(())
    }

    fn peer(&self, peer: &PeerAddr) -> Result<Arc<LibraryShared>> {
        self.network
            .0
            .lock()
            .unwrap()
            .get(&(self.library, peer.id))
            .cloned()
            .ok_or(StoreError::Unreachable(peer.id))
    }
}

#[async_trait]
impl LibraryStore for LoroLibrary {
    async fn upsert_photo(&self, rec: PhotoRecord) -> Result<()> {
        self.write(|s| Ok(((), s.upsert_photo(&rec))))
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
        let now = self.now();
        self.write(|s| Ok(((), s.set_favorite(id, fav, now)?)))
    }
    async fn mark_deleted(&self, id: &PhotoId) -> Result<()> {
        let now = self.now();
        self.write(|s| Ok(((), s.mark_deleted(id, now)?)))
    }
    async fn restore_photo(&self, id: &PhotoId) -> Result<()> {
        let now = self.now();
        self.write(|s| Ok(((), s.restore_photo(id, now)?)))
    }
    async fn purge_expired(&self, now: Timestamp) -> Result<Vec<PhotoId>> {
        let clock_now = self.now();
        self.write(|s| Ok(s.purge_expired(now, clock_now)))
    }
    async fn upsert_album(&self, album: AlbumMeta) -> Result<()> {
        self.write(|s| Ok(((), s.upsert_album(&album))))
    }
    async fn album_add(&self, album: &AlbumId, items: &[PhotoId]) -> Result<()> {
        let now = self.now();
        self.write(|s| Ok(((), s.album_add(album, items, now)?)))
    }
    async fn album_remove(&self, album: &AlbumId, items: &[PhotoId]) -> Result<()> {
        self.write(|s| Ok(((), s.album_remove(album, items)?)))
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
        self.write(|s| Ok(((), s.apply_device_change(&change)?)))
    }
    async fn put_holdings(&self, device: EndpointId, delta: HoldingsDelta) -> Result<()> {
        self.write(|s| Ok(((), s.put_holdings(device, &delta)?)))
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
        let now = self.now();
        self.write(|s| Ok(((), s.put_setting(key, value, now)?)))
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
        // Real Loro exchange, version-vector based, in both directions (never holding both locks).
        let (my_vv, my_updates_for_them) = {
            let mine = self.shared.inner.lock().unwrap();
            let vv = mine.doc.oplog_vv();
            let theirs_inner = theirs.inner.lock().unwrap();
            let for_me = theirs_inner
                .doc
                .export(ExportMode::updates(&vv))
                .map_err(|e| StoreError::Backend(e.to_string()))?;
            let their_vv = theirs_inner.doc.oplog_vv();
            let for_them = mine
                .doc
                .export(ExportMode::updates(&their_vv))
                .map_err(|e| StoreError::Backend(e.to_string()))?;
            (for_me, for_them)
        };
        {
            let mut inner = self.shared.inner.lock().unwrap();
            inner
                .doc
                .import(&my_vv)
                .map_err(|e| StoreError::Backend(e.to_string()))?;
            let changes = inner.rescan();
            drop(inner);
            self.shared.emit(changes);
        }
        {
            let mut inner = theirs.inner.lock().unwrap();
            inner
                .doc
                .import(&my_updates_for_them)
                .map_err(|e| StoreError::Backend(e.to_string()))?;
            let changes = inner.rescan();
            drop(inner);
            theirs.emit(changes);
        }
        Ok(())
    }

    fn capability(&self) -> Capability {
        self.mode
    }
}
