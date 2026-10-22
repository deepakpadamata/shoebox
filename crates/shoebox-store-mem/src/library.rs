//! In-memory `LibraryStore`.

use crate::net::MemNetwork;
use async_trait::async_trait;
use futures::channel::mpsc::{unbounded, UnboundedSender};
use futures::stream::BoxStream;
use shoebox_core::clock::Clock;
use shoebox_core::hlc::{Hlc, HlcClock};
use shoebox_core::ids::{Blake3Hash, EndpointId, Timestamp};
use shoebox_core::merge::Lww;
use shoebox_core::model::*;
use shoebox_core::store::*;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

/// One replica's state. Merging two states is the whole sync protocol.
pub(crate) struct LibState {
    hlc: HlcClock,
    max_seen: Hlc,
    photos: BTreeMap<PhotoId, PhotoRecord>,
    /// Purge markers: a record whose latest tombstone HLC is not after the marker is dropped.
    purged: BTreeMap<PhotoId, Hlc>,
    albums: BTreeMap<AlbumId, AlbumMeta>,
    devices: BTreeMap<EndpointId, DeviceCert>,
    admins: BTreeMap<EndpointId, AdminCert>,
    holdings: BTreeMap<(EndpointId, Blake3Hash), Lww<Option<BlobKind>>>,
}

pub(crate) struct LibShared {
    state: Mutex<LibState>,
    subs: Mutex<Vec<UnboundedSender<Change>>>,
}

impl LibShared {
    pub(crate) fn new(device: EndpointId) -> Self {
        Self {
            state: Mutex::new(LibState {
                hlc: HlcClock::new(device),
                max_seen: Hlc {
                    physical_ms: 0,
                    counter: 0,
                    device,
                },
                photos: BTreeMap::new(),
                purged: BTreeMap::new(),
                albums: BTreeMap::new(),
                devices: BTreeMap::new(),
                admins: BTreeMap::new(),
                holdings: BTreeMap::new(),
            }),
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

fn cert_key(c: &DeviceCert) -> (Hlc, bool, Role, &str, &[u8]) {
    (
        c.hlc,
        c.revoked,
        c.role,
        c.name.as_str(),
        c.signature.as_slice(),
    )
}

impl LibState {
    fn tick(&mut self, now_ms: u64) -> Hlc {
        let seen = self.max_seen;
        let h = self.hlc.observe(now_ms, &seen);
        self.max_seen = self.max_seen.max(h);
        h
    }

    fn observe(&mut self, h: &Hlc) {
        self.max_seen = self.max_seen.max(*h);
    }

    fn observe_record(&mut self, rec: &PhotoRecord) {
        self.observe(&rec.favorite.hlc);
        self.observe(&rec.deleted_at.hlc);
    }

    /// Largest HLC anywhere in this state.
    fn max_hlc(&self) -> Hlc {
        let mut max = self.max_seen;
        let mut see = |h: &Hlc| max = max.max(*h);
        for p in self.photos.values() {
            see(&p.favorite.hlc);
            see(&p.deleted_at.hlc);
        }
        self.purged.values().for_each(&mut see);
        for a in self.albums.values() {
            see(&a.name.hlc);
            see(&a.cover.hlc);
            a.items
                .adds
                .values()
                .chain(a.items.removes.values())
                .flatten()
                .for_each(&mut see);
        }
        self.devices.values().for_each(|c| see(&c.hlc));
        self.admins.values().for_each(|c| see(&c.hlc));
        self.holdings.values().for_each(|l| see(&l.hlc));
        max
    }

    fn alive(&self, rec: &PhotoRecord) -> bool {
        self.purged
            .get(&rec.id)
            .is_none_or(|marker| rec.deleted_at.hlc > *marker)
    }

    /// Insert/merge one record; returns the change to report, if any.
    fn merge_photo(&mut self, rec: &PhotoRecord) -> Option<Change> {
        let merged = match self.photos.get(&rec.id) {
            Some(existing) => existing.merge(rec).expect("same id"),
            None => rec.clone(),
        };
        if !self.alive(&merged) {
            return None;
        }
        let old = self.photos.insert(rec.id, merged.clone());
        if old.as_ref() == Some(&merged) {
            return None;
        }
        let was_deleted = old.as_ref().is_some_and(|o| o.is_deleted());
        Some(if merged.is_deleted() && !was_deleted {
            Change::PhotoDeleted(rec.id)
        } else {
            Change::PhotoUpserted(rec.id)
        })
    }

    fn merge_device(&mut self, cert: &DeviceCert) -> bool {
        match self.devices.get(&cert.device) {
            Some(existing) if cert_key(existing) >= cert_key(cert) => false,
            _ => {
                self.devices.insert(cert.device, cert.clone());
                true
            }
        }
    }

    fn merge_admin(&mut self, cert: &AdminCert) -> bool {
        let key = |c: &AdminCert| (c.hlc, c.signature.clone());
        match self.admins.get(&cert.admin) {
            Some(existing) if key(existing) >= key(cert) => false,
            _ => {
                self.admins.insert(cert.admin, cert.clone());
                true
            }
        }
    }

    fn merge_holding(
        &mut self,
        key: (EndpointId, Blake3Hash),
        value: &Lww<Option<BlobKind>>,
    ) -> bool {
        let merged = match self.holdings.get(&key) {
            Some(existing) => existing.merge(value),
            None => value.clone(),
        };
        self.holdings.insert(key, merged.clone()) != Some(merged)
    }

    /// Merge `other` into `self`; returns what changed.
    fn merge_from(&mut self, other: &LibState) -> Vec<Change> {
        let mut changes = vec![];
        for (id, marker) in &other.purged {
            let slot = self.purged.entry(*id).or_insert(*marker);
            *slot = (*slot).max(*marker);
        }
        // Drop local records that a (possibly just-learned) purge marker supersedes.
        let dead: Vec<PhotoId> = self
            .photos
            .values()
            .filter(|p| !self.alive(p))
            .map(|p| p.id)
            .collect();
        for id in dead {
            self.photos.remove(&id);
            changes.push(Change::PhotoDeleted(id));
        }
        for rec in other.photos.values() {
            changes.extend(self.merge_photo(rec));
        }
        for (id, album) in &other.albums {
            let merged = match self.albums.get(id) {
                Some(existing) => existing.merge(album).expect("same id"),
                None => album.clone(),
            };
            if self.albums.insert(*id, merged.clone()).as_ref() != Some(&merged) {
                changes.push(Change::AlbumChanged(*id));
            }
        }
        let mut devices_changed = false;
        for cert in other.devices.values() {
            devices_changed |= self.merge_device(cert);
        }
        for cert in other.admins.values() {
            devices_changed |= self.merge_admin(cert);
        }
        if devices_changed {
            changes.push(Change::DeviceListChanged);
        }
        let mut touched = BTreeSet::new();
        for (key, value) in &other.holdings {
            if self.merge_holding(*key, value) {
                touched.insert(key.0);
            }
        }
        changes.extend(
            touched
                .into_iter()
                .map(|device| Change::HoldingsChanged { device }),
        );
        let other_max = other.max_hlc();
        self.observe(&other_max);
        changes
    }

    /// Is this photo shown in album listings? Soft-deleted and purged photos are hidden;
    /// photos not synced yet are not.
    fn photo_visible_in_album(&self, id: &PhotoId) -> bool {
        match self.photos.get(id) {
            Some(p) => !p.is_deleted(),
            None => !self.purged.contains_key(id),
        }
    }

    fn visible_album_items(&self, album: &AlbumMeta) -> Vec<PhotoId> {
        album
            .items
            .items()
            .into_iter()
            .filter(|id| self.photo_visible_in_album(id))
            .collect()
    }
}

/// A personal library replica held in memory.
pub struct MemLibrary {
    shared: Arc<LibShared>,
    network: MemNetwork,
    library: LibraryId,
    device: EndpointId,
    clock: Arc<dyn Clock>,
    mode: Capability,
}

impl MemLibrary {
    pub(crate) fn new(
        shared: Arc<LibShared>,
        network: MemNetwork,
        library: LibraryId,
        device: EndpointId,
        clock: Arc<dyn Clock>,
        mode: Capability,
    ) -> Self {
        Self {
            shared,
            network,
            library,
            device,
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

    /// Run `f` under the state lock, then publish the changes it returns.
    fn with_state<T>(
        &self,
        f: impl FnOnce(&mut LibState) -> Result<(T, Vec<Change>)>,
    ) -> Result<T> {
        let (out, changes) = f(&mut self.shared.state.lock().unwrap())?;
        self.shared.emit(changes);
        Ok(out)
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

/// Copy of the parts of a replica another replica needs (taken under its lock, then released).
fn snapshot(shared: &LibShared) -> LibState {
    let s = shared.state.lock().unwrap();
    LibState {
        hlc: s.hlc.clone(),
        max_seen: s.max_seen,
        photos: s.photos.clone(),
        purged: s.purged.clone(),
        albums: s.albums.clone(),
        devices: s.devices.clone(),
        admins: s.admins.clone(),
        holdings: s.holdings.clone(),
    }
}

#[async_trait]
impl LibraryStore for MemLibrary {
    async fn upsert_photo(&self, rec: PhotoRecord) -> Result<()> {
        self.writable()?;
        self.with_state(|s| {
            s.observe_record(&rec);
            let changes = s.merge_photo(&rec).into_iter().collect();
            Ok(((), changes))
        })
    }

    async fn photo(&self, id: &PhotoId) -> Result<Option<PhotoRecord>> {
        Ok(self.shared.state.lock().unwrap().photos.get(id).cloned())
    }

    async fn photos_by_time(&self, range: TimeRange, page: Page) -> Result<Vec<PhotoRecord>> {
        let s = self.shared.state.lock().unwrap();
        let order = |taken_at: Timestamp, id: PhotoId| (Reverse(taken_at), id);
        let mut photos: Vec<&PhotoRecord> = s
            .photos
            .values()
            .filter(|p| !p.is_deleted() && range.contains(p.taken_at))
            .collect();
        photos.sort_by_key(|p| order(p.taken_at, p.id));
        let after = page.after.map(|c| order(c.taken_at, c.id));
        Ok(photos
            .into_iter()
            .filter(|p| after.is_none_or(|a| order(p.taken_at, p.id) > a))
            .take(page.limit)
            .cloned()
            .collect())
    }

    async fn deleted_photos(&self) -> Result<Vec<PhotoRecord>> {
        let s = self.shared.state.lock().unwrap();
        let mut out: Vec<PhotoRecord> = s
            .photos
            .values()
            .filter(|p| p.is_deleted())
            .cloned()
            .collect();
        out.sort_by_key(|p| (Reverse(p.deleted_at.value), p.id));
        Ok(out)
    }

    async fn set_favorite(&self, id: &PhotoId, fav: bool) -> Result<()> {
        self.writable()?;
        let now = self.now();
        self.with_state(|s| {
            let hlc = s.tick(now);
            let rec = s.photos.get_mut(id).ok_or(StoreError::NotFound)?;
            rec.set_favorite(fav, hlc);
            Ok(((), vec![Change::PhotoUpserted(*id)]))
        })
    }

    async fn mark_deleted(&self, id: &PhotoId) -> Result<()> {
        self.writable()?;
        let now = self.now();
        self.with_state(|s| {
            let hlc = s.tick(now);
            let rec = s.photos.get_mut(id).ok_or(StoreError::NotFound)?;
            if rec.is_deleted() {
                return Ok(((), vec![]));
            }
            rec.mark_deleted(Timestamp(now as i64), hlc);
            Ok(((), vec![Change::PhotoDeleted(*id)]))
        })
    }

    async fn restore_photo(&self, id: &PhotoId) -> Result<()> {
        self.writable()?;
        let now = self.now();
        self.with_state(|s| {
            let hlc = s.tick(now);
            let rec = s.photos.get_mut(id).ok_or(StoreError::NotFound)?;
            if !rec.is_deleted() {
                return Ok(((), vec![]));
            }
            rec.restore(hlc);
            Ok(((), vec![Change::PhotoUpserted(*id)]))
        })
    }

    async fn purge_expired(&self, now: Timestamp) -> Result<Vec<PhotoId>> {
        self.writable()?;
        let clock_now = self.now();
        self.with_state(|s| {
            let due: Vec<PhotoId> = s
                .photos
                .values()
                .filter(|p| {
                    p.deleted_at
                        .value
                        .is_some_and(|t| now.0 - t.0 >= RETENTION_MS)
                })
                .map(|p| p.id)
                .collect();
            if due.is_empty() {
                return Ok((vec![], vec![]));
            }
            let marker = s.tick(clock_now);
            for id in &due {
                s.photos.remove(id);
                s.purged.insert(*id, marker);
            }
            let changes = due.iter().map(|id| Change::PhotoDeleted(*id)).collect();
            Ok((due, changes))
        })
    }

    async fn upsert_album(&self, album: AlbumMeta) -> Result<()> {
        self.writable()?;
        self.with_state(|s| {
            let id = album.id;
            let merged = match s.albums.get(&id) {
                Some(existing) => existing.merge(&album).expect("same id"),
                None => album,
            };
            let mut max = s.max_seen;
            max = max.max(merged.name.hlc).max(merged.cover.hlc);
            merged
                .items
                .adds
                .values()
                .chain(merged.items.removes.values())
                .flatten()
                .for_each(|h| max = max.max(*h));
            s.max_seen = max;
            let changed = s.albums.insert(id, merged.clone()).as_ref() != Some(&merged);
            Ok((
                (),
                if changed {
                    vec![Change::AlbumChanged(id)]
                } else {
                    vec![]
                },
            ))
        })
    }

    async fn album_add(&self, album: &AlbumId, items: &[PhotoId]) -> Result<()> {
        self.writable()?;
        let now = self.now();
        self.with_state(|s| {
            if !s.albums.contains_key(album) {
                return Err(StoreError::NotFound);
            }
            let tags: Vec<Hlc> = items.iter().map(|_| s.tick(now)).collect();
            let meta = s.albums.get_mut(album).expect("checked");
            for (item, tag) in items.iter().zip(tags) {
                meta.items.add(*item, tag);
            }
            Ok(((), vec![Change::AlbumChanged(*album)]))
        })
    }

    async fn album_remove(&self, album: &AlbumId, items: &[PhotoId]) -> Result<()> {
        self.writable()?;
        self.with_state(|s| {
            let meta = s.albums.get_mut(album).ok_or(StoreError::NotFound)?;
            let before = meta.items.clone();
            for item in items {
                meta.items.remove(item);
            }
            let changes = if meta.items != before {
                vec![Change::AlbumChanged(*album)]
            } else {
                vec![]
            };
            Ok(((), changes))
        })
    }

    async fn album_items(&self, album: &AlbumId, page: Page) -> Result<Vec<PhotoId>> {
        let s = self.shared.state.lock().unwrap();
        let meta = s.albums.get(album).ok_or(StoreError::NotFound)?;
        let after = page.after.map(|c| c.id);
        Ok(s.visible_album_items(meta)
            .into_iter()
            .filter(|id| after.is_none_or(|a| *id > a))
            .take(page.limit)
            .collect())
    }

    async fn albums(&self) -> Result<Vec<AlbumSummary>> {
        let s = self.shared.state.lock().unwrap();
        Ok(s.albums
            .values()
            .map(|a| AlbumSummary {
                id: a.id,
                name: a.name.value.clone(),
                cover: a.cover.value,
                item_count: s.visible_album_items(a).len(),
            })
            .collect())
    }

    async fn device_list(&self) -> Result<SignedDeviceList> {
        let s = self.shared.state.lock().unwrap();
        Ok(SignedDeviceList {
            devices: s.devices.values().cloned().collect(),
            admins: s.admins.values().cloned().collect(),
        })
    }

    async fn apply_device_change(&self, change: SignedDeviceChange) -> Result<()> {
        self.writable()?;
        // Stub until M2-01: only an empty signature is rejected.
        let signature = match &change {
            SignedDeviceChange::Device(c) => &c.signature,
            SignedDeviceChange::Admin(c) => &c.signature,
        };
        if signature.is_empty() {
            return Err(StoreError::BadSignature);
        }
        self.with_state(|s| {
            let changed = match &change {
                SignedDeviceChange::Device(c) => {
                    s.observe(&c.hlc);
                    s.merge_device(c)
                }
                SignedDeviceChange::Admin(c) => {
                    s.observe(&c.hlc);
                    s.merge_admin(c)
                }
            };
            Ok((
                (),
                if changed {
                    vec![Change::DeviceListChanged]
                } else {
                    vec![]
                },
            ))
        })
    }

    async fn put_holdings(&self, device: EndpointId, delta: HoldingsDelta) -> Result<()> {
        self.writable()?;
        if device != self.device {
            return Err(StoreError::Invalid(format!(
                "holdings are single-writer: {device} is not this device"
            )));
        }
        self.with_state(|s| {
            s.observe(&delta.hlc);
            let mut changed = false;
            for (blob, kind) in &delta.added {
                changed |= s.merge_holding((device, *blob), &Lww::new(Some(*kind), delta.hlc));
            }
            for blob in &delta.removed {
                changed |= s.merge_holding((device, *blob), &Lww::new(None, delta.hlc));
            }
            Ok((
                (),
                if changed {
                    vec![Change::HoldingsChanged { device }]
                } else {
                    vec![]
                },
            ))
        })
    }

    async fn holders(&self, blob: &Blake3Hash) -> Result<Vec<Holding>> {
        let s = self.shared.state.lock().unwrap();
        Ok(s.holdings
            .iter()
            .filter(|((_, b), _)| b == blob)
            .filter_map(|((device, b), lww)| {
                lww.value.map(|kind| Holding {
                    device: *device,
                    blob: *b,
                    kind,
                    since: lww.hlc,
                })
            })
            .collect())
    }

    async fn at_risk(&self, min_copies: u8, page: Page) -> Result<Vec<PhotoId>> {
        let s = self.shared.state.lock().unwrap();
        let after = page.after.map(|c| c.id);
        let copies = |p: &PhotoRecord| {
            s.holdings
                .iter()
                .filter(|((_, b), l)| *b == p.original.hash && l.value == Some(BlobKind::Original))
                .count()
        };
        Ok(s.photos
            .values()
            .filter(|p| !p.is_deleted() && after.is_none_or(|a| p.id > a))
            .filter(|p| copies(p) < min_copies as usize)
            .map(|p| p.id)
            .take(page.limit)
            .collect())
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
        let remote = snapshot(&theirs);
        let changes = self.shared.state.lock().unwrap().merge_from(&remote);
        self.shared.emit(changes);
        let mine = snapshot(&self.shared);
        let changes = theirs.state.lock().unwrap().merge_from(&mine);
        theirs.emit(changes);
        Ok(())
    }

    fn capability(&self) -> Capability {
        self.mode
    }
}
