//! `LibraryState`: the merged, indexed in-memory view of one library replica.
//!
//! Every backend keeps its *replicated* data somewhere (memory, iroh-docs entries, a CRDT) but
//! answers queries and applies writes through this one type, so the merge semantics in
//! [`crate::merge`] and the query behaviour the conformance suite checks are implemented exactly
//! once. A backend persists what a write [`Touched`] (via [`LibraryState::entry`]) and feeds
//! records that arrive from peers through [`LibraryState::apply_entry`].
//!
//! Indexes kept incrementally: the live timeline (newest first) and the set of devices holding
//! each original, so paging and `at_risk` do not rescan the library.

use crate::hlc::{Hlc, HlcClock};
use crate::identity::{IdentityError, TrustChain};
use crate::ids::{Blake3Hash, EndpointId, Timestamp};
use crate::merge::Lww;
use crate::model::*;
use crate::store::*;
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound;

/// A replicated record, as a backend stores and ships it. (Transient values; boxing the larger
/// variants would only complicate every `match`.)
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StateEntry {
    Photo(PhotoRecord),
    Album(AlbumMeta),
    Device(DeviceCert),
    Admin(AdminCert),
    /// What `device` holds of `blob` (`kind: None` = no longer held), ordered by `hlc`.
    Holding {
        device: EndpointId,
        blob: Blake3Hash,
        kind: Option<BlobKind>,
        hlc: Hlc,
    },
    /// Purge marker for a photo.
    Purged {
        id: PhotoId,
        hlc: Hlc,
    },
    /// A library setting (last writer wins per key).
    Setting {
        key: String,
        value: Vec<u8>,
        hlc: Hlc,
    },
}

/// Which stored record a write changed (and so must be persisted/announced).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Touched {
    Photo(PhotoId),
    Album(AlbumId),
    Device(EndpointId),
    Admin(EndpointId),
    Holding(EndpointId, Blake3Hash),
    Purged(PhotoId),
    Setting(String),
}

/// The result of a state mutation.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Effect {
    /// Observable changes, in order.
    pub changes: Vec<Change>,
    /// Records to persist (deduplicated, in first-touched order).
    pub touched: Vec<Touched>,
    /// Membership index for `touched`: a linear `contains` made merging a 100k-photo library
    /// quadratic (it took 18 s).
    seen: std::collections::HashSet<Touched>,
}

impl Effect {
    fn change(&mut self, c: Change) {
        self.changes.push(c);
    }
    fn touch(&mut self, t: Touched) {
        if self.seen.insert(t.clone()) {
            self.touched.push(t);
        }
    }
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty() && self.touched.is_empty()
    }

    /// Fold `other` into this effect (changes appended, touched records deduplicated).
    pub fn extend(&mut self, other: Effect) {
        self.changes.extend(other.changes);
        for t in other.touched {
            self.touch(t);
        }
    }
}

fn identity_err(e: IdentityError) -> StoreError {
    match e {
        IdentityError::BadSignature => StoreError::BadSignature,
        IdentityError::SignerNotAdmin | IdentityError::Expired | IdentityError::RootRequired => {
            StoreError::NotAuthorized
        }
    }
}

type TimelineKey = (Reverse<Timestamp>, PhotoId);

#[derive(Clone)]
pub struct LibraryState {
    device: EndpointId,
    hlc: HlcClock,
    max_seen: Hlc,
    photos: BTreeMap<PhotoId, PhotoRecord>,
    /// Non-deleted photos, newest first (ties by id).
    timeline: BTreeSet<TimelineKey>,
    /// Purge markers: a record whose latest tombstone HLC is not after the marker is dropped.
    purged: BTreeMap<PhotoId, Hlc>,
    albums: BTreeMap<AlbumId, AlbumMeta>,
    devices: BTreeMap<EndpointId, DeviceCert>,
    admins: BTreeMap<EndpointId, AdminCert>,
    /// Present once the library's root key is known: certs are then verified (M2-01) and
    /// `devices`/`admins` only hold the ones that chain to the root.
    chain: Option<TrustChain>,
    holdings: BTreeMap<(EndpointId, Blake3Hash), Lww<Option<BlobKind>>>,
    settings: BTreeMap<String, Lww<Vec<u8>>>,
    /// Devices currently holding each blob (any kind).
    holders_by_blob: BTreeMap<Blake3Hash, BTreeSet<EndpointId>>,
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

impl LibraryState {
    /// A state that verifies device and admin certs against `root` (M2-01).
    pub fn with_root(device: EndpointId, root: RootPublicKey, library: LibraryId) -> Self {
        let mut s = Self::new(device);
        s.chain = Some(TrustChain::new(root, library));
        s
    }

    /// Once any device is certified the library is "armed": replicated records must then come
    /// from certified authors (see the Loro backend). Before that nothing can be judged.
    pub fn is_armed(&self) -> bool {
        self.chain.is_some() && !self.devices.is_empty()
    }

    /// Devices with a valid cert (revoked ones included, so history stays readable) whose id
    /// starts with `prefix`.
    pub fn certified_with_prefix(&self, prefix: &[u8]) -> Vec<(EndpointId, Role)> {
        self.devices
            .iter()
            .filter(|(id, _)| id.0.starts_with(prefix))
            .map(|(id, c)| (*id, c.role))
            .collect()
    }

    /// Whether `device` is currently certified (and not revoked). Always true when no root is
    /// known, because nothing can be verified then.
    pub fn is_member(&self, device: &EndpointId) -> bool {
        match self.chain {
            Some(_) => self.devices.get(device).is_some_and(|c| !c.revoked),
            None => true,
        }
    }

    /// Whether a certificate this state knows marks `device` revoked.
    fn is_revoked(&self, device: &EndpointId) -> bool {
        self.devices.get(device).is_some_and(|c| c.revoked)
    }

    pub fn new(device: EndpointId) -> Self {
        Self {
            device,
            hlc: HlcClock::new(device),
            max_seen: Hlc {
                physical_ms: 0,
                counter: 0,
                device,
            },
            photos: BTreeMap::new(),
            timeline: BTreeSet::new(),
            purged: BTreeMap::new(),
            albums: BTreeMap::new(),
            devices: BTreeMap::new(),
            admins: BTreeMap::new(),
            chain: None,
            holdings: BTreeMap::new(),
            settings: BTreeMap::new(),
            holders_by_blob: BTreeMap::new(),
        }
    }

    pub fn device(&self) -> EndpointId {
        self.device
    }

    // ---------- clock ----------

    /// A fresh HLC strictly after everything this replica has seen.
    pub fn tick(&mut self, now_ms: u64) -> Hlc {
        let seen = self.max_seen;
        let h = self.hlc.observe(now_ms, &seen);
        self.max_seen = self.max_seen.max(h);
        h
    }

    pub fn observe(&mut self, h: &Hlc) {
        self.max_seen = self.max_seen.max(*h);
    }

    // ---------- internal merge helpers ----------

    fn alive(&self, rec: &PhotoRecord) -> bool {
        self.purged
            .get(&rec.id)
            .is_none_or(|marker| rec.deleted_at.hlc > *marker)
    }

    fn unindex(&mut self, rec: &PhotoRecord) {
        self.timeline.remove(&(Reverse(rec.taken_at), rec.id));
    }

    fn index(&mut self, rec: &PhotoRecord) {
        if !rec.is_deleted() {
            self.timeline.insert((Reverse(rec.taken_at), rec.id));
        }
    }

    /// Merge one record in; reports the change and what to persist if anything changed.
    fn merge_photo(&mut self, rec: &PhotoRecord, fx: &mut Effect) {
        self.observe(&rec.favorite.hlc);
        self.observe(&rec.deleted_at.hlc);
        let merged = match self.photos.get(&rec.id) {
            Some(existing) => existing.merge(rec).expect("same id"),
            None => rec.clone(),
        };
        if !self.alive(&merged) {
            return;
        }
        let old = self.photos.insert(rec.id, merged.clone());
        if old.as_ref() == Some(&merged) {
            return;
        }
        if let Some(old) = &old {
            self.unindex(old);
        }
        self.index(&merged);
        let was_deleted = old.as_ref().is_some_and(|o| o.is_deleted());
        fx.change(if merged.is_deleted() && !was_deleted {
            Change::PhotoDeleted(rec.id)
        } else {
            Change::PhotoUpserted(rec.id)
        });
        fx.touch(Touched::Photo(rec.id));
    }

    fn merge_purge(&mut self, id: PhotoId, marker: Hlc, fx: &mut Effect) {
        self.observe(&marker);
        let current = self.purged.get(&id).copied();
        let newest = current.map_or(marker, |c| c.max(marker));
        if current != Some(newest) {
            self.purged.insert(id, newest);
            fx.touch(Touched::Purged(id));
        }
        if let Some(rec) = self.photos.get(&id).cloned() {
            if !self.alive(&rec) {
                self.photos.remove(&id);
                self.unindex(&rec);
                fx.change(Change::PhotoDeleted(id));
                fx.touch(Touched::Photo(id));
            }
        }
    }

    fn merge_album(&mut self, album: &AlbumMeta, fx: &mut Effect) {
        self.observe(&album.name.hlc);
        self.observe(&album.cover.hlc);
        for h in album
            .items
            .adds
            .values()
            .chain(album.items.removes.values())
            .flatten()
        {
            self.observe(h);
        }
        let merged = match self.albums.get(&album.id) {
            Some(existing) => existing.merge(album).expect("same id"),
            None => album.clone(),
        };
        if self.albums.insert(album.id, merged.clone()).as_ref() != Some(&merged) {
            fx.change(Change::AlbumChanged(album.id));
            fx.touch(Touched::Album(album.id));
        }
    }

    fn merge_device(&mut self, cert: &DeviceCert) -> bool {
        self.observe(&cert.hlc);
        match self.devices.get(&cert.device) {
            Some(existing) if cert_key(existing) >= cert_key(cert) => false,
            _ => {
                self.devices.insert(cert.device, cert.clone());
                true
            }
        }
    }

    fn merge_admin(&mut self, cert: &AdminCert) -> bool {
        self.observe(&cert.hlc);
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
        device: EndpointId,
        blob: Blake3Hash,
        value: &Lww<Option<BlobKind>>,
    ) -> bool {
        self.observe(&value.hlc);
        let merged = match self.holdings.get(&(device, blob)) {
            Some(existing) => existing.merge(value),
            None => value.clone(),
        };
        let changed = self.holdings.insert((device, blob), merged.clone()) != Some(merged.clone());
        if changed {
            if merged.value.is_some() {
                self.holders_by_blob.entry(blob).or_default().insert(device);
            } else if let Some(holders) = self.holders_by_blob.get_mut(&blob) {
                holders.remove(&device);
                if holders.is_empty() {
                    self.holders_by_blob.remove(&blob);
                }
            }
        }
        changed
    }

    fn merge_setting(&mut self, key: &str, value: &Lww<Vec<u8>>) -> bool {
        self.observe(&value.hlc);
        let merged = match self.settings.get(key) {
            Some(existing) => existing.merge(value),
            None => value.clone(),
        };
        self.settings.insert(key.to_string(), merged.clone()) != Some(merged)
    }

    // ---------- local writes ----------

    /// Write a library setting (last writer wins per key).
    pub fn put_setting(&mut self, key: &str, value: Vec<u8>, now_ms: u64) -> Result<Effect> {
        let valid = (1..=64).contains(&key.len())
            && key.bytes().all(|b| {
                b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')
            });
        if !valid {
            return Err(StoreError::Invalid(format!("invalid setting key `{key}`")));
        }
        let hlc = self.tick(now_ms);
        let mut fx = Effect::default();
        if self.merge_setting(key, &Lww::new(value, hlc)) {
            fx.change(Change::SettingChanged(key.to_string()));
            fx.touch(Touched::Setting(key.to_string()));
        }
        Ok(fx)
    }

    pub fn upsert_photo(&mut self, rec: &PhotoRecord) -> Effect {
        let mut fx = Effect::default();
        self.merge_photo(rec, &mut fx);
        fx
    }

    pub fn set_favorite(&mut self, id: &PhotoId, fav: bool, now_ms: u64) -> Result<Effect> {
        let hlc = self.tick(now_ms);
        let rec = self.photos.get_mut(id).ok_or(StoreError::NotFound)?;
        rec.set_favorite(fav, hlc);
        let mut fx = Effect::default();
        fx.change(Change::PhotoUpserted(*id));
        fx.touch(Touched::Photo(*id));
        Ok(fx)
    }

    pub fn mark_deleted(&mut self, id: &PhotoId, now_ms: u64) -> Result<Effect> {
        let hlc = self.tick(now_ms);
        let rec = self.photos.get(id).cloned().ok_or(StoreError::NotFound)?;
        let mut fx = Effect::default();
        if rec.is_deleted() {
            return Ok(fx);
        }
        let mut updated = rec.clone();
        updated.mark_deleted(Timestamp(now_ms as i64), hlc);
        self.unindex(&rec);
        self.photos.insert(*id, updated);
        fx.change(Change::PhotoDeleted(*id));
        fx.touch(Touched::Photo(*id));
        Ok(fx)
    }

    pub fn restore_photo(&mut self, id: &PhotoId, now_ms: u64) -> Result<Effect> {
        let hlc = self.tick(now_ms);
        let rec = self.photos.get(id).cloned().ok_or(StoreError::NotFound)?;
        let mut fx = Effect::default();
        if !rec.is_deleted() {
            return Ok(fx);
        }
        let mut updated = rec;
        updated.restore(hlc);
        self.index(&updated);
        self.photos.insert(*id, updated);
        fx.change(Change::PhotoUpserted(*id));
        fx.touch(Touched::Photo(*id));
        Ok(fx)
    }

    /// Purge photos deleted at least [`RETENTION_MS`] before `now`.
    pub fn purge_expired(&mut self, now: Timestamp, clock_now_ms: u64) -> (Vec<PhotoId>, Effect) {
        let due: Vec<PhotoId> = self
            .photos
            .values()
            .filter(|p| {
                p.deleted_at
                    .value
                    .is_some_and(|t| now.0 - t.0 >= RETENTION_MS)
            })
            .map(|p| p.id)
            .collect();
        self.purge(due, clock_now_ms)
    }

    /// Purge these photos now, without waiting for the retention window: only the ones in
    /// Recently Deleted; live and unknown ids are left alone.
    pub fn purge_deleted(&mut self, ids: &[PhotoId], clock_now_ms: u64) -> (Vec<PhotoId>, Effect) {
        let mut due: Vec<PhotoId> = ids
            .iter()
            .filter(|id| self.photos.get(id).is_some_and(|p| p.is_deleted()))
            .copied()
            .collect();
        due.sort();
        due.dedup();
        self.purge(due, clock_now_ms)
    }

    fn purge(&mut self, due: Vec<PhotoId>, clock_now_ms: u64) -> (Vec<PhotoId>, Effect) {
        let mut fx = Effect::default();
        if due.is_empty() {
            return (due, fx);
        }
        let marker = self.tick(clock_now_ms);
        for id in &due {
            if let Some(rec) = self.photos.remove(id) {
                self.unindex(&rec);
            }
            self.purged.insert(*id, marker);
            fx.change(Change::PhotoDeleted(*id));
            fx.touch(Touched::Photo(*id));
            fx.touch(Touched::Purged(*id));
        }
        (due, fx)
    }

    pub fn upsert_album(&mut self, album: &AlbumMeta) -> Effect {
        let mut fx = Effect::default();
        self.merge_album(album, &mut fx);
        fx
    }

    pub fn album_add(&mut self, album: &AlbumId, items: &[PhotoId], now_ms: u64) -> Result<Effect> {
        if !self.albums.contains_key(album) {
            return Err(StoreError::NotFound);
        }
        let tags: Vec<Hlc> = items.iter().map(|_| self.tick(now_ms)).collect();
        let meta = self.albums.get_mut(album).expect("checked");
        for (item, tag) in items.iter().zip(tags) {
            meta.items.add(*item, tag);
        }
        let mut fx = Effect::default();
        fx.change(Change::AlbumChanged(*album));
        fx.touch(Touched::Album(*album));
        Ok(fx)
    }

    pub fn album_remove(&mut self, album: &AlbumId, items: &[PhotoId]) -> Result<Effect> {
        let meta = self.albums.get_mut(album).ok_or(StoreError::NotFound)?;
        let before = meta.items.clone();
        for item in items {
            meta.items.remove(item);
        }
        let mut fx = Effect::default();
        if meta.items != before {
            fx.change(Change::AlbumChanged(*album));
            fx.touch(Touched::Album(*album));
        }
        Ok(fx)
    }

    pub fn apply_device_change(&mut self, change: &SignedDeviceChange) -> Result<Effect> {
        let mut fx = Effect::default();
        let Some(chain) = &mut self.chain else {
            // No root known (tests, legacy): only an empty signature is rejected.
            let signature = match change {
                SignedDeviceChange::Device(c) => &c.signature,
                SignedDeviceChange::Admin(c) => &c.signature,
            };
            if signature.is_empty() {
                return Err(StoreError::BadSignature);
            }
            match change {
                SignedDeviceChange::Device(c) => {
                    if self.merge_device(c) {
                        fx.change(Change::DeviceListChanged);
                        fx.touch(Touched::Device(c.device));
                    }
                }
                SignedDeviceChange::Admin(c) => {
                    if self.merge_admin(c) {
                        fx.change(Change::DeviceListChanged);
                        fx.touch(Touched::Admin(c.admin));
                    }
                }
            }
            return Ok(fx);
        };
        match change {
            SignedDeviceChange::Admin(c) => {
                chain.check_admin(c).map_err(identity_err)?;
                chain.add_admin(c.clone());
            }
            SignedDeviceChange::Device(c) => {
                chain.check_device_cert(c).map_err(identity_err)?;
                chain.add_device(c.clone());
            }
        }
        self.refresh_chain(&mut fx);
        Ok(fx)
    }

    /// Recompute the verified device/admin view after certs were added; touched entries are
    /// the ones that changed.
    fn refresh_chain(&mut self, fx: &mut Effect) {
        let Some(chain) = &self.chain else { return };
        let valid = chain.validate();
        let mut admins: BTreeMap<EndpointId, AdminCert> = BTreeMap::new();
        for a in valid.admins {
            match admins.get(&a.admin) {
                Some(e) if (e.hlc, &e.signature) >= (a.hlc, &a.signature) => {}
                _ => {
                    admins.insert(a.admin, a);
                }
            }
        }
        let devices: BTreeMap<EndpointId, DeviceCert> =
            valid.devices.into_iter().map(|c| (c.device, c)).collect();
        let mut changed = false;
        for (id, cert) in &devices {
            if self.devices.get(id) != Some(cert) {
                self.observe(&cert.hlc);
                fx.touch(Touched::Device(*id));
                changed = true;
            }
        }
        for (id, cert) in &admins {
            if self.admins.get(id) != Some(cert) {
                self.observe(&cert.hlc);
                fx.touch(Touched::Admin(*id));
                changed = true;
            }
        }
        if changed {
            fx.change(Change::DeviceListChanged);
        }
        self.devices = devices;
        self.admins = admins;
    }

    /// Single writer: only this replica's own device may be written.
    pub fn put_holdings(&mut self, device: EndpointId, delta: &HoldingsDelta) -> Result<Effect> {
        if device != self.device {
            return Err(StoreError::Invalid(format!(
                "holdings are single-writer: {device} is not this device"
            )));
        }
        let mut fx = Effect::default();
        let mut changed = false;
        for (blob, kind) in &delta.added {
            if self.merge_holding(device, *blob, &Lww::new(Some(*kind), delta.hlc)) {
                changed = true;
                fx.touch(Touched::Holding(device, *blob));
            }
        }
        for blob in &delta.removed {
            if self.merge_holding(device, *blob, &Lww::new(None, delta.hlc)) {
                changed = true;
                fx.touch(Touched::Holding(device, *blob));
            }
        }
        if changed {
            fx.change(Change::HoldingsChanged { device });
        }
        Ok(fx)
    }

    // ---------- replication ----------

    /// Merge one replicated record (from a peer or from storage).
    pub fn apply_entry(&mut self, entry: &StateEntry) -> Effect {
        let mut fx = Effect::default();
        match entry {
            StateEntry::Photo(rec) => self.merge_photo(rec, &mut fx),
            StateEntry::Album(album) => self.merge_album(album, &mut fx),
            StateEntry::Device(cert) => match &mut self.chain {
                None => {
                    if self.merge_device(cert) {
                        fx.change(Change::DeviceListChanged);
                        fx.touch(Touched::Device(cert.device));
                    }
                }
                Some(chain) => {
                    // A cert may arrive before the admin cert that authorises it, so it is kept
                    // and judged on every refresh; only a broken signature is dropped.
                    if !chain.signature_ok(cert) {
                        return fx;
                    }
                    chain.add_device(cert.clone());
                    self.refresh_chain(&mut fx);
                }
            },
            StateEntry::Admin(cert) => match &mut self.chain {
                None => {
                    if self.merge_admin(cert) {
                        fx.change(Change::DeviceListChanged);
                        fx.touch(Touched::Admin(cert.admin));
                    }
                }
                Some(chain) => {
                    if chain.check_admin(cert).is_err() {
                        return fx;
                    }
                    chain.add_admin(cert.clone());
                    self.refresh_chain(&mut fx);
                }
            },
            StateEntry::Holding {
                device,
                blob,
                kind,
                hlc,
            } => {
                if self.merge_holding(*device, *blob, &Lww::new(*kind, *hlc)) {
                    fx.change(Change::HoldingsChanged { device: *device });
                    fx.touch(Touched::Holding(*device, *blob));
                }
            }
            StateEntry::Purged { id, hlc } => self.merge_purge(*id, *hlc, &mut fx),
            StateEntry::Setting { key, value, hlc } => {
                if self.merge_setting(key, &Lww::new(value.clone(), *hlc)) {
                    fx.change(Change::SettingChanged(key.clone()));
                    fx.touch(Touched::Setting(key.clone()));
                }
            }
        }
        fx
    }

    /// The current merged record for `what`, for persistence. `None` = nothing to store (for a
    /// purged photo: delete the stored record).
    pub fn entry(&self, what: &Touched) -> Option<StateEntry> {
        match what {
            Touched::Photo(id) => self.photos.get(id).cloned().map(StateEntry::Photo),
            Touched::Album(id) => self.albums.get(id).cloned().map(StateEntry::Album),
            Touched::Device(id) => self.devices.get(id).cloned().map(StateEntry::Device),
            Touched::Admin(id) => self.admins.get(id).cloned().map(StateEntry::Admin),
            Touched::Holding(device, blob) => {
                self.holdings
                    .get(&(*device, *blob))
                    .map(|l| StateEntry::Holding {
                        device: *device,
                        blob: *blob,
                        kind: l.value,
                        hlc: l.hlc,
                    })
            }
            Touched::Purged(id) => self
                .purged
                .get(id)
                .map(|hlc| StateEntry::Purged { id: *id, hlc: *hlc }),
            Touched::Setting(key) => self.settings.get(key).map(|l| StateEntry::Setting {
                key: key.clone(),
                value: l.value.clone(),
                hlc: l.hlc,
            }),
        }
    }

    /// Every record, for full export (tests, snapshots, initial persistence).
    pub fn all_entries(&self) -> Vec<StateEntry> {
        let mut out: Vec<StateEntry> = vec![];
        out.extend(self.photos.values().cloned().map(StateEntry::Photo));
        out.extend(self.albums.values().cloned().map(StateEntry::Album));
        out.extend(self.devices.values().cloned().map(StateEntry::Device));
        out.extend(self.admins.values().cloned().map(StateEntry::Admin));
        out.extend(self.holdings.iter().map(|((d, b), l)| StateEntry::Holding {
            device: *d,
            blob: *b,
            kind: l.value,
            hlc: l.hlc,
        }));
        out.extend(
            self.purged
                .iter()
                .map(|(id, hlc)| StateEntry::Purged { id: *id, hlc: *hlc }),
        );
        out.extend(self.settings.iter().map(|(k, l)| StateEntry::Setting {
            key: k.clone(),
            value: l.value.clone(),
            hlc: l.hlc,
        }));
        out
    }

    /// Merge everything in `other` (used by the in-memory backend's simulated sync).
    pub fn merge_from(&mut self, other: &LibraryState) -> Effect {
        let mut fx = Effect::default();
        // Purge markers first so superseded local records are dropped before merging records.
        for (id, marker) in &other.purged {
            self.merge_purge(*id, *marker, &mut fx);
        }
        for entry in other.all_entries() {
            if !matches!(entry, StateEntry::Purged { .. }) {
                let e = self.apply_entry(&entry);
                fx.changes.extend(e.changes);
                for t in e.touched {
                    fx.touch(t);
                }
            }
        }
        fx
    }

    // ---------- reads ----------

    pub fn photo(&self, id: &PhotoId) -> Option<PhotoRecord> {
        self.photos.get(id).cloned()
    }

    pub fn setting(&self, key: &str) -> Option<Vec<u8>> {
        self.settings.get(key).map(|l| l.value.clone())
    }

    pub fn photo_count(&self) -> usize {
        self.photos.len()
    }

    /// Live photos in `range`, newest first, following `page`.
    pub fn photos_by_time(&self, range: TimeRange, page: Page) -> Vec<PhotoRecord> {
        let start: Bound<TimelineKey> = match page.after {
            Some(c) => Bound::Excluded((Reverse(c.taken_at), c.id)),
            None => Bound::Unbounded,
        };
        self.timeline
            .range((start, Bound::Unbounded))
            .filter(|(Reverse(t), _)| range.contains(*t))
            .take(page.limit)
            .filter_map(|(_, id)| self.photos.get(id).cloned())
            .collect()
    }

    pub fn deleted_photos(&self) -> Vec<PhotoRecord> {
        let mut out: Vec<PhotoRecord> = self
            .photos
            .values()
            .filter(|p| p.is_deleted())
            .cloned()
            .collect();
        out.sort_by_key(|p| (Reverse(p.deleted_at.value), p.id));
        out
    }

    /// Soft-deleted and purged photos are hidden; photos that have not synced yet are not.
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

    pub fn album_items(&self, album: &AlbumId, page: Page) -> Result<Vec<PhotoId>> {
        let meta = self.albums.get(album).ok_or(StoreError::NotFound)?;
        let after = page.after.map(|c| c.id);
        Ok(self
            .visible_album_items(meta)
            .into_iter()
            .filter(|id| after.is_none_or(|a| *id > a))
            .take(page.limit)
            .collect())
    }

    pub fn albums(&self) -> Vec<AlbumSummary> {
        self.albums
            .values()
            .map(|a| AlbumSummary {
                id: a.id,
                name: a.name.value.clone(),
                cover: a.cover.value,
                item_count: self.visible_album_items(a).len(),
            })
            .collect()
    }

    pub fn device_list(&self) -> SignedDeviceList {
        SignedDeviceList {
            devices: self.devices.values().cloned().collect(),
            admins: self.admins.values().cloned().collect(),
        }
    }

    /// Who holds `blob`. A revoked device is left out, whenever it recorded the holding: it is
    /// no backup, no source to fetch from and no reason to free up space. (Only a revocation
    /// excludes: a device this state has no certificate for, such as the one that created the
    /// library, still counts.)
    pub fn holders(&self, blob: &Blake3Hash) -> Vec<Holding> {
        let Some(devices) = self.holders_by_blob.get(blob) else {
            return vec![];
        };
        devices
            .iter()
            .filter(|d| !self.is_revoked(d))
            .filter_map(|device| {
                let lww = self.holdings.get(&(*device, *blob))?;
                lww.value.map(|kind| Holding {
                    device: *device,
                    blob: *blob,
                    kind,
                    since: lww.hlc,
                })
            })
            .collect()
    }

    /// Number of distinct devices holding `original` as an original (a thumb is not a copy, nor
    /// is a revoked device's).
    pub fn copy_count(&self, original: &Blake3Hash) -> usize {
        self.holders_by_blob.get(original).map_or(0, |devices| {
            devices
                .iter()
                .filter(|d| {
                    !self.is_revoked(d)
                        && self
                            .holdings
                            .get(&(**d, *original))
                            .is_some_and(|l| l.value == Some(BlobKind::Original))
                })
                .count()
        })
    }

    /// Whether a keeper (a certified, not revoked device with the Keeper role) holds the original.
    /// One keeper is enough to call a photo backed up: that is what a keeper is for.
    pub fn held_by_keeper(&self, original: &Blake3Hash) -> bool {
        self.holders_by_blob.get(original).is_some_and(|devices| {
            devices.iter().any(|d| {
                self.holdings
                    .get(&(*d, *original))
                    .is_some_and(|l| l.value == Some(BlobKind::Original))
                    && self
                        .devices
                        .get(d)
                        .is_some_and(|c| c.role == Role::Keeper && !c.revoked)
            })
        })
    }

    /// Photos (not deleted) that are neither on `min_copies` devices nor on a keeper.
    pub fn at_risk(&self, min_copies: u8, page: Page) -> Vec<PhotoId> {
        let after = page.after.map(|c| c.id);
        let start = match after {
            Some(id) => Bound::Excluded(id),
            None => Bound::Unbounded,
        };
        self.photos
            .range((start, Bound::Unbounded))
            .filter(|(_, p)| {
                !p.is_deleted()
                    && self.copy_count(&p.original.hash) < min_copies as usize
                    && !self.held_by_keeper(&p.original.hash)
            })
            .map(|(id, _)| *id)
            .take(page.limit)
            .collect()
    }
}
