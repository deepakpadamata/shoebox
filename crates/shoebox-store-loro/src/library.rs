use crate::auth;
use crate::backend::Transport;
use crate::persist::Persist;
use crate::verdicts::{self, Verdicts};
use async_trait::async_trait;
use futures::channel::mpsc::{unbounded, UnboundedSender};
use futures::stream::BoxStream;
use loro::{ExportMode, LoroDoc, LoroValue, VersionVector};
use shoebox_core::clock::Clock;
use shoebox_core::entry_codec::{decode_library_entry, encode_library_entry, touched_key};
use shoebox_core::identity::KeyPair;
use shoebox_core::ids::{Blake3Hash, EndpointId, Timestamp};
use shoebox_core::library_state::{Effect, LibraryState, StateEntry};
use shoebox_core::model::*;
use shoebox_core::store::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tracing::{debug, warn};

/// Name of the Loro map holding every replicated record.
const ENTRIES: &str = "entries";
/// Records from authors whose certificate has not arrived yet wait here (bounded).
const MAX_PENDING: usize = 50_000;
/// How long a replica must go without writes before it compacts (see
/// [`LibraryShared::compact_when_idle`]): a sync in progress is left alone.
const IDLE_AFTER: std::time::Duration = std::time::Duration::from_secs(10);
/// Save the signature verdicts once this many are new, even without compacting.
const SAVE_VERDICTS_AFTER: usize = 256;

fn be(e: impl std::fmt::Display) -> StoreError {
    StoreError::Backend(e.to_string())
}

pub(crate) struct Inner {
    pub(crate) state: LibraryState,
    pub(crate) doc: LoroDoc,
    device: EndpointId,
    /// Hex of the first 8 bytes of this device's id: the map-key prefix of its own records.
    author: String,
    /// The current epoch's id: records are signed for it and peers address it.
    pub(crate) library: LibraryId,
    /// What certificates are bound to; constant across rotations.
    identity: LibraryId,
    root: RootPublicKey,
    signer: Option<KeyPair>,
    /// Where replicas live (`<base>/<epoch hex>/`); `None` = memory only.
    base_dir: Option<PathBuf>,
    persist: Option<Persist>,
    /// What has been written to the log already.
    logged: VersionVector,
    /// Signed records whose author is not (yet) known to be certified: `key → sealed value`.
    pending: HashMap<String, Vec<u8>>,
    /// Signatures already checked, kept across restarts next to the snapshot.
    verdicts: Verdicts,
}

pub(crate) struct LibraryShared {
    pub(crate) inner: Mutex<Inner>,
    subs: Mutex<Vec<UnboundedSender<Change>>>,
}

fn author_prefix(device: &EndpointId) -> String {
    hex::encode(&device.0[..8])
}

/// Which records follow a library into its next epoch.
#[derive(Clone, Copy)]
pub(crate) enum Carry {
    /// Everything still valid (the admin that starts a rotation).
    All,
    /// What this device wrote itself plus the device list (members following a rotation).
    Own,
    /// [`Carry::All`] without certificates: the new epoch trusts another root (a handover).
    AllButCerts,
    /// [`Carry::Own`] without certificates, for the same reason.
    OwnButCerts,
}

/// The capability of a rotated library: a new epoch id, the original identity in the payload.
pub(crate) fn rotated_capability(
    epoch: LibraryId,
    identity: LibraryId,
    root: RootPublicKey,
) -> LibraryCapability {
    let mut backend = vec![2];
    backend.extend_from_slice(&identity.0);
    LibraryCapability {
        library: epoch,
        root,
        mode: Capability::Write,
        backend,
    }
}

impl Inner {
    pub(crate) fn identity_id(&self) -> LibraryId {
        self.identity
    }

    pub(crate) fn root_key(&self) -> RootPublicKey {
        self.root
    }

    fn build(
        device: EndpointId,
        root: RootPublicKey,
        identity: LibraryId,
        library: LibraryId,
        signer: Option<KeyPair>,
        base_dir: Option<&Path>,
    ) -> Result<Inner> {
        let doc = LoroDoc::new();
        // A stable Loro peer id per device keeps its operations in one lane.
        let peer = u64::from_le_bytes(device.0[..8].try_into().expect("8 bytes"));
        let _ = doc.set_peer_id(peer.max(1));
        let mut inner = Inner {
            state: LibraryState::with_root(device, root, identity),
            doc,
            device,
            author: author_prefix(&device),
            library,
            identity,
            root,
            signer,
            base_dir: base_dir.map(Path::to_path_buf),
            persist: None,
            logged: VersionVector::default(),
            pending: HashMap::new(),
            verdicts: Verdicts::default(),
        };
        if let Some(base) = base_dir {
            let dir = base.join(hex::encode(library.0));
            let (persist, snapshot, updates) = Persist::open(&dir).map_err(be)?;
            if let Some(s) = snapshot {
                inner.doc.import(&s).map_err(be)?;
            }
            for u in updates {
                // A damaged update inside a valid frame cannot happen unless the disk lies; skip it.
                if let Err(e) = inner.doc.import(&u) {
                    warn!("skipping unreadable logged update: {e}");
                }
            }
            inner.logged = inner.doc.oplog_vv();
            inner.persist = Some(persist);
            inner.verdicts = Verdicts::load(&dir, library);
            inner.fold_all();
        }
        Ok(inner)
    }

    /// The records that follow this replica into a new epoch.
    fn carried(&self, carry: Carry) -> Vec<StateEntry> {
        let certs = |e: &StateEntry| matches!(e, StateEntry::Device(_) | StateEntry::Admin(_));
        match carry {
            Carry::AllButCerts => self
                .carried(Carry::All)
                .into_iter()
                .filter(|e| !certs(e))
                .collect(),
            Carry::OwnButCerts => self
                .carried(Carry::Own)
                .into_iter()
                .filter(|e| !certs(e))
                .collect(),
            Carry::All => self
                .state
                .all_entries()
                .into_iter()
                // Holdings are single-writer: every device republishes its own afterwards.
                .filter(|e| match e {
                    StateEntry::Holding { device, .. } => *device == self.device,
                    _ => true,
                })
                .collect(),
            Carry::Own => {
                let prefix = format!("{}/", self.author);
                let map = self.doc.get_map(ENTRIES);
                let mut out = vec![];
                map.for_each(|k, v| {
                    let Some(record_key) = k.strip_prefix(&prefix) else {
                        return;
                    };
                    if let loro::ValueOrContainer::Value(LoroValue::Binary(bytes)) = v {
                        if let Some(o) = auth::open(&bytes) {
                            if let Ok(e) = decode_library_entry(record_key, o.payload) {
                                out.push(e);
                            }
                        }
                    }
                });
                // The device list is needed at once to know who to talk to; certificates are
                // self-authenticating, so everybody can carry them.
                out.extend(
                    self.state
                        .all_entries()
                        .into_iter()
                        .filter(|e| matches!(e, StateEntry::Device(_) | StateEntry::Admin(_))),
                );
                out
            }
        }
    }

    /// Write `entries` into this (fresh) replica as this device's own records.
    fn adopt_entries(&mut self, entries: Vec<StateEntry>) {
        let mut fx = Effect::default();
        for e in &entries {
            fx.extend(self.state.apply_entry(e));
        }
        self.persist_effect(&fx);
    }
}

impl LibraryShared {
    pub(crate) fn new(
        device: EndpointId,
        root: RootPublicKey,
        identity: LibraryId,
        library: LibraryId,
        signer: Option<KeyPair>,
        base_dir: Option<&Path>,
    ) -> Result<Arc<Self>> {
        let inner = Inner::build(device, root, identity, library, signer, base_dir)?;
        let has_log = inner.persist.is_some();
        let shared = Arc::new(Self {
            inner: Mutex::new(inner),
            subs: Mutex::new(vec![]),
        });
        if has_log {
            // Flush the update log periodically (see `Persist::append`) and compact once quiet;
            // ends with the library.
            let weak = Arc::downgrade(&shared);
            std::thread::spawn(move || {
                while let Some(shared) = weak.upgrade() {
                    if let Some(p) = shared.inner.lock().unwrap().persist.as_mut() {
                        let _ = p.flush();
                    }
                    shared.compact_when_idle(IDLE_AFTER);
                    drop(shared);
                    std::thread::sleep(crate::persist::FLUSH_INTERVAL);
                }
            });
        }
        Ok(shared)
    }

    /// Replace the replica by a fresh one for `new_epoch`, carrying `carry`; returns the old
    /// epoch's id. The old on-disk replica is removed once the new one holds the records.
    pub(crate) fn rotate(&self, new_epoch: LibraryId, carry: Carry) -> Result<LibraryId> {
        let root = self.inner.lock().unwrap().root;
        self.rotate_to(new_epoch, root, carry, vec![])
    }

    /// [`LibraryShared::rotate`] into an epoch that trusts `root`, writing `first` (its
    /// certificates) before the carried records.
    pub(crate) fn rotate_to(
        &self,
        new_epoch: LibraryId,
        root: RootPublicKey,
        carry: Carry,
        first: Vec<StateEntry>,
    ) -> Result<LibraryId> {
        let mut inner = self.inner.lock().unwrap();
        let old = inner.library;
        if old == new_epoch {
            // Rebuilding the epoch it is in would reuse, and then delete, its own directory.
            return Err(StoreError::Invalid(
                "this replica is already in that epoch".into(),
            ));
        }
        let mut records = first;
        records.extend(inner.carried(carry));
        let mut fresh = Inner::build(
            inner.device,
            root,
            inner.identity,
            new_epoch,
            inner.signer.clone(),
            inner.base_dir.as_deref(),
        )?;
        fresh.adopt_entries(records);
        if let Some(p) = fresh.persist.as_mut() {
            let _ = p.flush();
        }
        let old_dir = inner.base_dir.as_ref().map(|b| b.join(hex::encode(old.0)));
        *inner = fresh;
        drop(inner);
        if let Some(dir) = old_dir {
            let _ = std::fs::remove_dir_all(dir);
        }
        self.emit(vec![Change::DeviceListChanged]);
        Ok(old)
    }

    /// Once nothing has been written for `idle_after`: compact when the log is worth it, and
    /// save the signature verdicts when enough are new. Exporting needs the lock; writing the
    /// files does not, so a sync or a screen reading the library waits only for the export.
    pub(crate) fn compact_when_idle(&self, idle_after: std::time::Duration) {
        let (library, dir, snapshot, verdicts) = {
            let mut inner = self.inner.lock().unwrap();
            let Some(p) = &inner.persist else { return };
            if p.idle_for() < idle_after {
                return;
            }
            let (dir, log_len) = (p.dir().to_path_buf(), p.log_len());
            let compact = log_len >= crate::persist::COMPACT_IDLE_MIN_LOG;
            if !compact && inner.verdicts.unsaved() < SAVE_VERDICTS_AFTER {
                return;
            }
            let snapshot = if compact {
                match inner.doc.export(ExportMode::Snapshot) {
                    Ok(s) => Some((s, log_len)),
                    Err(e) => {
                        warn!("could not export a snapshot: {e}");
                        None
                    }
                }
            } else {
                None
            };
            let verdicts = inner.verdicts_for_disk();
            inner.verdicts.mark_saved();
            (inner.library, dir, snapshot, verdicts)
        };
        if let Err(e) = verdicts::write(&dir, &verdicts) {
            warn!("could not save signature verdicts: {e}");
        }
        let Some((snapshot, log_len)) = snapshot else {
            return;
        };
        let tmp = match crate::persist::write_temp(&dir, "snapshot.idle.tmp", &snapshot) {
            Ok(t) => t,
            Err(e) => {
                warn!("compaction failed: {e}");
                return;
            }
        };
        let mut inner = self.inner.lock().unwrap();
        // A rotation in the meantime replaced the replica (and removes this directory).
        let installed = inner.library == library
            && match inner.persist.as_mut() {
                Some(p) => p
                    .install_snapshot(&tmp, snapshot.len() as u64, log_len)
                    .unwrap_or_else(|e| {
                        warn!("compaction failed: {e}");
                        false
                    }),
                None => false,
            };
        if !installed {
            // Written to while exporting: the next quiet moment tries again.
            let _ = std::fs::remove_file(&tmp);
        }
    }

    pub(crate) fn device_of(&self) -> EndpointId {
        self.inner.lock().unwrap().device
    }

    pub(crate) fn epoch(&self) -> LibraryId {
        self.inner.lock().unwrap().library
    }

    fn emit(&self, changes: Vec<Change>) {
        let mut subs = self.subs.lock().unwrap();
        for change in changes {
            subs.retain(|tx| tx.unbounded_send(change.clone()).is_ok());
        }
    }

    /// Import updates from a peer (or a file) and fold what changed into the state.
    pub(crate) fn import_bytes(&self, bytes: &[u8]) -> Result<()> {
        let changes = self.inner.lock().unwrap().import(bytes)?;
        self.emit(changes);
        Ok(())
    }

    pub(crate) fn oplog_vv(&self) -> VersionVector {
        self.inner.lock().unwrap().doc.oplog_vv()
    }

    pub(crate) fn updates_since(&self, vv: &VersionVector) -> Result<Vec<u8>> {
        self.inner
            .lock()
            .unwrap()
            .doc
            .export(ExportMode::updates(vv))
            .map_err(be)
    }

    // The `_in` forms are for replication: each checks, under the lock a rotation also takes, that
    // the document is still the one for `epoch`. A sync that started before a move must not mix
    // the two: old-epoch operations imported into the new document would make its version vector
    // claim operations it never got (a rotation restarts every counter), and the peer would then
    // never send them.

    pub(crate) fn oplog_vv_in(&self, epoch: LibraryId) -> Result<VersionVector> {
        let inner = self.inner.lock().unwrap();
        same_epoch(&inner, epoch)?;
        Ok(inner.doc.oplog_vv())
    }

    pub(crate) fn updates_since_in(&self, epoch: LibraryId, vv: &VersionVector) -> Result<Vec<u8>> {
        let inner = self.inner.lock().unwrap();
        same_epoch(&inner, epoch)?;
        inner.doc.export(ExportMode::updates(vv)).map_err(be)
    }

    pub(crate) fn import_bytes_in(&self, epoch: LibraryId, bytes: &[u8]) -> Result<()> {
        let changes = {
            let mut inner = self.inner.lock().unwrap();
            same_epoch(&inner, epoch)?;
            inner.import(bytes)?
        };
        self.emit(changes);
        Ok(())
    }
}

fn same_epoch(inner: &Inner, epoch: LibraryId) -> Result<()> {
    if inner.library == epoch {
        Ok(())
    } else {
        Err(StoreError::Backend(
            "the library moved to a new epoch during the sync".into(),
        ))
    }
}

enum Verdict {
    Accept,
    Pending,
    Reject(&'static str),
}

impl Inner {
    /// Write the records a mutation touched into the Loro document, then log the update.
    fn persist_effect(&mut self, effect: &Effect) -> Vec<Change> {
        let map = self.doc.get_map(ENTRIES);
        for touched in &effect.touched {
            let record_key = touched_key(touched);
            let key = format!("{}/{}", self.author, record_key);
            match self.state.entry(touched) {
                Some(entry) => {
                    let payload = encode_library_entry(&entry);
                    let value = auth::seal(self.signer.as_ref(), self.library, &key, &payload);
                    let _ = map.insert(&key, LoroValue::Binary(value.into()));
                }
                None => {
                    let _ = map.delete(&key);
                }
            }
        }
        self.doc.commit();
        self.log_new_ops();
        // A new certificate may settle records that were waiting for their author.
        let mut extra = vec![];
        if !self.pending.is_empty()
            && effect
                .changes
                .iter()
                .any(|c| matches!(c, Change::DeviceListChanged))
        {
            self.retry_pending(&mut extra);
        }
        extra
    }

    /// Append everything not yet logged to the update log (compacting when it grew large).
    fn log_new_ops(&mut self) {
        let Some(persist) = &mut self.persist else {
            return;
        };
        let now = self.doc.oplog_vv();
        if now == self.logged {
            return;
        }
        match self.doc.export(ExportMode::updates(&self.logged)) {
            Ok(update) => {
                if let Err(e) = persist.append(&update) {
                    warn!("could not write the update log: {e}");
                    return;
                }
                self.logged = now;
            }
            Err(e) => warn!("could not export updates: {e}"),
        }
        if persist.should_compact() {
            if let Err(e) = self.compact_now() {
                warn!("compaction failed: {e}");
            }
        }
    }

    /// Write a fresh snapshot, empty the log and save the signature verdicts, all under the lock
    /// (see [`LibraryShared::compact_when_idle`] for the usual way, which writes off the lock).
    fn compact_now(&mut self) -> Result<()> {
        let snapshot = self.doc.export(ExportMode::Snapshot).map_err(be)?;
        let verdicts = self.verdicts_for_disk();
        if let Some(p) = &mut self.persist {
            p.compact(&snapshot).map_err(be)?;
            verdicts::write(p.dir(), &verdicts).map_err(be)?;
            self.verdicts.mark_saved();
        }
        Ok(())
    }

    /// The verdicts file, keeping only verdicts for values still in the document.
    fn verdicts_for_disk(&mut self) -> Vec<u8> {
        let map = self.doc.get_map(ENTRIES);
        let mut current = std::collections::HashSet::new();
        map.for_each(|k, v| {
            if let loro::ValueOrContainer::Value(LoroValue::Binary(b)) = v {
                current.insert(verdicts::digest(k, &b));
            }
        });
        self.verdicts.retain(&current);
        self.verdicts.encode(self.library)
    }

    fn import(&mut self, bytes: &[u8]) -> Result<Vec<Change>> {
        let before = self.doc.state_frontiers();
        self.doc.import(bytes).map_err(be)?;
        self.log_new_ops();
        let after = self.doc.state_frontiers();
        if before == after {
            return Ok(vec![]);
        }
        let diff = self.doc.diff(&before, &after).map_err(be)?;
        let mut keys: Vec<String> = vec![];
        for (_, d) in diff.iter() {
            if let loro::event::Diff::Map(m) = d {
                keys.extend(m.updated.keys().map(|k| k.to_string()));
            }
        }
        Ok(self.fold_keys(keys))
    }

    /// Fold every record of the document (used once, when opening from disk).
    fn fold_all(&mut self) {
        let map = self.doc.get_map(ENTRIES);
        let mut keys = vec![];
        map.for_each(|k, _| keys.push(k.to_string()));
        self.fold_keys(keys);
    }

    /// Fold the records stored under `keys`. Certificates go first so that records in the same
    /// batch can already be judged; the (signature-checking) judging of a large batch of other
    /// records then runs on all cores before the results are applied in order.
    fn fold_keys(&mut self, keys: Vec<String>) -> Vec<Change> {
        let is_cert = |k: &str| match k.split_once('/').map(|(_, r)| r) {
            Some(r) => r.starts_with("devices/") || r.starts_with("admins/"),
            None => false,
        };
        let (certs, rest): (Vec<String>, Vec<String>) = keys.into_iter().partition(|k| is_cert(k));
        let map = self.doc.get_map(ENTRIES);
        let value_of = |key: &str| match map.get(key) {
            Some(loro::ValueOrContainer::Value(LoroValue::Binary(v))) => Some(v.to_vec()),
            _ => None,
        };
        let mut changes = vec![];
        for key in certs {
            if let Some(value) = value_of(&key) {
                let judged = judge(
                    &self.state,
                    self.library,
                    self.signer.is_some(),
                    &self.verdicts,
                    &key,
                    &value,
                );
                self.admit(&key, &value, judged, &mut changes);
            }
        }
        let batch: Vec<(String, Vec<u8>)> = rest
            .into_iter()
            .filter_map(|k| value_of(&k).map(|v| (k, v)))
            .collect();
        let (state, library, strict, verdicts) = (
            &self.state,
            self.library,
            self.signer.is_some(),
            &self.verdicts,
        );
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
        let judged: Vec<Judged> = if batch.len() < 2_000 || threads < 2 {
            batch
                .iter()
                .map(|(k, v)| judge(state, library, strict, verdicts, k, v))
                .collect()
        } else {
            let chunk = batch.len().div_ceil(threads);
            std::thread::scope(|scope| {
                let handles: Vec<_> = batch
                    .chunks(chunk)
                    .map(|part| {
                        scope.spawn(move || {
                            part.iter()
                                .map(|(k, v)| judge(state, library, strict, verdicts, k, v))
                                .collect::<Vec<Judged>>()
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .flat_map(|h| h.join().expect("judging thread"))
                    .collect()
            })
        };
        for ((key, value), judged) in batch.into_iter().zip(judged) {
            self.admit(&key, &value, judged, &mut changes);
        }
        changes
    }

    /// Merge one judged record into the state (or park / drop it).
    fn admit(&mut self, key: &str, value: &[u8], judged: Judged, changes: &mut Vec<Change>) {
        if let Some((digest, author)) = judged.checked {
            self.verdicts.insert(digest, author);
        }
        match judged.verdict {
            Verdict::Accept => {
                if let Some(entry) = judged.entry {
                    let effect = self.state.apply_entry(&entry);
                    let devices_changed = effect
                        .changes
                        .iter()
                        .any(|c| matches!(c, Change::DeviceListChanged));
                    changes.extend(effect.changes);
                    self.pending.remove(key);
                    if devices_changed && !self.pending.is_empty() {
                        self.retry_pending(changes);
                    }
                }
            }
            Verdict::Pending => {
                if self.pending.len() < MAX_PENDING {
                    self.pending.insert(key.to_string(), value.to_vec());
                }
            }
            Verdict::Reject(why) => {
                debug!(key, why, "rejected a replicated record");
                self.pending.remove(key);
            }
        }
    }

    fn retry_pending(&mut self, changes: &mut Vec<Change>) {
        let waiting: Vec<(String, Vec<u8>)> = self.pending.drain().collect();
        for (key, value) in waiting {
            let judged = judge(
                &self.state,
                self.library,
                self.signer.is_some(),
                &self.verdicts,
                &key,
                &value,
            );
            match judged.verdict {
                Verdict::Pending => {
                    self.pending.insert(key, value);
                }
                _ => self.admit(&key, &value, judged, changes),
            }
        }
    }
}

struct Judged {
    verdict: Verdict,
    entry: Option<StateEntry>,
    /// A signature this judging checked (and found valid): remember it.
    checked: Option<(verdicts::Digest, EndpointId)>,
}

fn rejected(why: &'static str) -> Judged {
    Judged {
        verdict: Verdict::Reject(why),
        entry: None,
        checked: None,
    }
}

/// Decide whether a stored record may enter the state: decodes it, checks the author's
/// signature against the certified devices and applies the role rules.
fn judge(
    state: &LibraryState,
    library: LibraryId,
    strict: bool,
    verdicts: &Verdicts,
    key: &str,
    value: &[u8],
) -> Judged {
    let Some((prefix, record_key)) = key.split_once('/') else {
        return rejected("malformed key");
    };
    let Some(opened) = auth::open(value) else {
        return rejected("malformed value");
    };
    let Ok(entry) = decode_library_entry(record_key, opened.payload) else {
        return rejected("undecodable record");
    };
    let accept = |entry, checked| Judged {
        verdict: Verdict::Accept,
        entry: Some(entry),
        checked,
    };
    // Certificates are judged by the certificate chain itself, whoever carries them.
    if matches!(entry, StateEntry::Device(_) | StateEntry::Admin(_)) {
        return accept(entry, None);
    }
    let Ok(prefix_bytes) = hex::decode(prefix) else {
        return rejected("malformed author");
    };
    if opened.signature.is_none() {
        // Replicas without a signing key (in-process tests) never enforce signatures.
        return if strict && state.is_armed() {
            rejected("unsigned record in a library with certified devices")
        } else {
            accept(entry, None)
        };
    }
    let candidates = state.certified_with_prefix(&prefix_bytes);
    if candidates.is_empty() {
        return Judged {
            verdict: Verdict::Pending,
            entry: None,
            checked: None,
        };
    }
    // A signature checked before (see `verdicts`) is not checked again; who signed it must still
    // be a certified device, and the rules below still apply as they are now.
    let digest = verdicts::digest(key, value);
    let known = verdicts
        .author(&digest)
        .and_then(|a| candidates.iter().find(|(id, _)| *id == a).copied());
    let (author, role, checked) = match known {
        Some((author, role)) => (author, role, None),
        None => {
            let Some((author, role)) = candidates
                .into_iter()
                .find(|(id, _)| auth::verify(&id.0, library, key, &opened))
            else {
                return rejected("bad signature");
            };
            (author, role, Some((digest, author)))
        }
    };
    let mut judged = match &entry {
        // A device may only publish its own holdings.
        StateEntry::Holding { device, .. } => {
            if *device == author {
                accept(entry, None)
            } else {
                rejected("holdings of another device")
            }
        }
        _ if role == Role::Viewer => rejected("a viewer cannot publish library records"),
        _ => accept(entry, None),
    };
    judged.checked = checked;
    judged
}

/// A personal library replica on a Loro document.
pub struct LoroLibrary {
    shared: Arc<LibraryShared>,
    transport: Transport,
    clock: Arc<dyn Clock>,
    mode: Capability,
}

impl LoroLibrary {
    pub(crate) fn new(
        shared: Arc<LibraryShared>,
        transport: Transport,
        clock: Arc<dyn Clock>,
        mode: Capability,
    ) -> Self {
        Self {
            shared,
            transport,
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
        let extra = inner.persist_effect(&effect);
        drop(inner);
        self.shared.emit(effect.changes);
        self.shared.emit(extra);
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
        self.shared.import_bytes(bytes)
    }

    /// What the background thread does once the replica has been quiet for `idle_after`
    /// (normally ten seconds): for tests.
    #[cfg(feature = "testing")]
    #[doc(hidden)]
    pub fn compact_if_idle(&self, idle_after: std::time::Duration) {
        self.shared.compact_when_idle(idle_after);
    }

    /// Write a fresh snapshot now and empty the update log (normally automatic).
    pub fn compact(&self) -> Result<()> {
        self.shared.inner.lock().unwrap().compact_now()
    }

    /// Bytes on disk: `(snapshot, update log)`; `(0, 0)` for a memory-only replica.
    pub fn disk_usage(&self) -> (u64, u64) {
        self.shared
            .inner
            .lock()
            .unwrap()
            .persist
            .as_ref()
            .map_or((0, 0), |p| p.sizes())
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
    async fn purge_deleted(&self, ids: &[PhotoId]) -> Result<Vec<PhotoId>> {
        let clock_now = self.now();
        self.write(|s| Ok(s.purge_deleted(ids, clock_now)))
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
        let epoch = self.shared.epoch();
        self.transport.sync(&self.shared, epoch, peer).await
    }

    fn capability(&self) -> Capability {
        self.mode
    }

    fn replicates_by_push(&self) -> bool {
        true
    }

    async fn begin_rotation(&self) -> Result<LibraryCapability> {
        self.writable()?;
        let mut id = [0u8; 16];
        getrandom::fill(&mut id).expect("operating system randomness");
        let new = LibraryId(id);
        let (identity, root) = {
            let inner = self.shared.inner.lock().unwrap();
            (inner.identity_id(), inner.root_key())
        };
        let old = self.shared.rotate(new, Carry::All)?;
        self.transport.rotated(old, new, &self.shared);
        Ok(rotated_capability(new, identity, root))
    }

    async fn adopt_rotation(&self, new: &LibraryCapability) -> Result<()> {
        self.writable()?;
        let old = self.shared.rotate(new.library, Carry::Own)?;
        self.transport.rotated(old, new.library, &self.shared);
        Ok(())
    }

    async fn begin_root_rotation(
        &self,
        new_root: RootPublicKey,
        certs: Vec<SignedDeviceChange>,
    ) -> Result<LibraryCapability> {
        self.writable()?;
        let mut id = [0u8; 16];
        getrandom::fill(&mut id).expect("operating system randomness");
        let new = LibraryId(id);
        let identity = self.shared.inner.lock().unwrap().identity_id();
        let first = certs
            .into_iter()
            .map(|c| match c {
                SignedDeviceChange::Admin(a) => StateEntry::Admin(a),
                SignedDeviceChange::Device(d) => StateEntry::Device(d),
            })
            .collect();
        let old = self
            .shared
            .rotate_to(new, new_root, Carry::AllButCerts, first)?;
        self.transport.rotated(old, new, &self.shared);
        Ok(rotated_capability(new, identity, new_root))
    }

    async fn adopt_root_rotation(&self, new: &LibraryCapability) -> Result<()> {
        self.writable()?;
        let old = self
            .shared
            .rotate_to(new.library, new.root, Carry::OwnButCerts, vec![])?;
        self.transport.rotated(old, new.library, &self.shared);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sync that began before a move must not touch the new epoch's document (see
    /// `import_bytes_in`): its operations would poison the new version vector.
    #[test]
    fn replication_refuses_another_epochs_document() {
        let root = KeyPair::generate();
        let (identity, old, new) = (LibraryId([7; 16]), LibraryId([1; 16]), LibraryId([2; 16]));
        let shared = LibraryShared::new(
            EndpointId([3; 32]),
            root.public(),
            identity,
            old,
            None,
            None,
        )
        .unwrap();
        // Operations from another device's replica of the old epoch.
        let other = LoroDoc::new();
        other.set_peer_id(42).unwrap();
        other.get_map(ENTRIES).insert("x", 1).unwrap();
        other.commit();
        let old_ops = other.export(ExportMode::all_updates()).unwrap();

        shared.rotate(new, Carry::Own).unwrap();
        let before = shared.oplog_vv();
        assert!(shared.import_bytes_in(old, &old_ops).is_err());
        assert!(shared.oplog_vv_in(old).is_err());
        assert!(shared.updates_since_in(old, &before).is_err());
        assert_eq!(
            shared.oplog_vv(),
            before,
            "nothing from the old epoch got in"
        );

        shared.import_bytes_in(new, &old_ops).unwrap();
        assert_ne!(
            shared.oplog_vv_in(new).unwrap(),
            before,
            "the right epoch imports"
        );
    }

    /// A remembered verdict skips only the signature check: who signed must still be certified,
    /// and the rules about roles still apply as the certificates are now.
    #[test]
    fn remembered_signatures_skip_only_the_cryptography() {
        use shoebox_core::entry_codec::library_key;
        use shoebox_core::hlc::Hlc;
        use shoebox_core::identity::issue_device_cert;

        let root = KeyPair::from_seed([42; 32]);
        let lib = LibraryId([5; 16]);
        let (author, other) = (KeyPair::from_seed([1; 32]), KeyPair::from_seed([9; 32]));
        let hlc = |ms| Hlc {
            physical_ms: ms,
            counter: 0,
            device: author.endpoint_id(),
        };
        let certify = |state: &mut LibraryState, role, ms| {
            let cert =
                issue_device_cert(&root, lib, author.endpoint_id(), role, "a", hlc(ms), false);
            state.apply_entry(&StateEntry::Device(cert));
        };
        let mut state = LibraryState::with_root(EndpointId([3; 32]), root.root_public(), lib);
        certify(&mut state, Role::Keeper, 1);

        let entry = StateEntry::Setting {
            key: "k".into(),
            value: vec![1],
            hlc: hlc(2),
        };
        let key = format!(
            "{}/{}",
            author_prefix(&author.endpoint_id()),
            library_key(&entry)
        );
        let payload = encode_library_entry(&entry);
        let good = auth::seal(Some(&author), lib, &key, &payload);
        // Signed by a key that is not the author's: only a remembered verdict could let it in.
        let forged = auth::seal(Some(&other), lib, &key, &payload);
        let accepted = |j: &Judged| matches!(j.verdict, Verdict::Accept);

        let none = Verdicts::default();
        let j = judge(&state, lib, true, &none, &key, &good);
        assert!(accepted(&j));
        let (digest, signer) = j.checked.expect("a checked signature is reported");
        assert_eq!(
            (digest, signer),
            (verdicts::digest(&key, &good), author.endpoint_id())
        );
        assert!(!accepted(&judge(&state, lib, true, &none, &key, &forged)));

        let mut known = Verdicts::default();
        known.insert(verdicts::digest(&key, &forged), author.endpoint_id());
        let j = judge(&state, lib, true, &known, &key, &forged);
        assert!(
            accepted(&j),
            "the remembered verdict stands in for the check"
        );
        assert!(j.checked.is_none(), "and nothing new was checked");

        // Remembered for a device that is not certified: checked again (and refused).
        let mut stranger = Verdicts::default();
        stranger.insert(verdicts::digest(&key, &forged), other.endpoint_id());
        assert!(!accepted(&judge(
            &state, lib, true, &stranger, &key, &forged
        )));

        // The author is now a viewer: refused, remembered signature or not.
        certify(&mut state, Role::Viewer, 3);
        assert!(!accepted(&judge(&state, lib, true, &known, &key, &forged)));
        assert!(!accepted(&judge(&state, lib, true, &none, &key, &good)));
    }
}
