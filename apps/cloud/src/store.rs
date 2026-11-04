//! What the service keeps: libraries (their root key and certificates), blobs and objects they uploaded, and
//! shares. The records ([`crate::meta::Meta`]: a data directory, or a Durable Object's database) are the truth; a
//! map in memory answers questions without reading them.
//!
//! ```text
//! <data>/libs/<library hex>/lib.postcard      root key + certificates
//! <data>/libs/<library hex>/blobs/<cid hex>   ciphertext
//! <data>/libs/<library hex>/objects/<name>    named encrypted objects (snapshots)
//! <data>/libs/<library hex>/index.log         with a bucket: which blobs and objects it holds, and their sizes
//! <data>/shares/<share id hex>/meta.postcard  who owns it, version, expiry, the cids it lists
//! <data>/shares/<share id hex>/manifest       the sealed manifest
//! <data>/reports/<share id hex>               a share taken down by a report
//! <data>/uploads/<token hex>                  an upload handed out and not yet committed
//! <data>/tmp/                                 uploads in progress
//! ```
//!
//! With an S3 bucket (`crate::s3`) the bytes (`blobs/`, `objects/`, `manifest`) live in the bucket under the same
//! names and this directory keeps only the bookkeeping. The bucket cannot be listed cheaply, so what it holds is
//! written to `index.log`, one line per change (`+b <cid> <size>`, `-b <cid>`, `+o <name> <size>`, `-o <name>`),
//! and rewritten without the history when the service starts.
//!
//! Nothing here is plaintext: blobs and manifests arrive encrypted and stay that way.

use crate::meta::Meta;
use serde::{Deserialize, Serialize};
use shoebox_core::identity::TrustChain;
use shoebox_core::model::{AdminCert, DeviceCert};
use shoebox_core::store::{LibraryId, RootPublicKey};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub type Cid = [u8; 32];
pub type ShareId = [u8; 16];

/// Most certificates kept per library: a library has a handful of devices; this bounds what a member can store.
const MAX_CERTS: usize = 512;

#[derive(Serialize, Deserialize)]
struct LibMeta {
    root: RootPublicKey,
    admins: Vec<AdminCert>,
    devices: Vec<DeviceCert>,
}

#[derive(Serialize, Deserialize)]
struct ShareMeta {
    library: [u8; 16],
    version: u32,
    expires_ms: Option<i64>,
    cids: Vec<Cid>,
    manifest_len: u64,
}

struct Library {
    meta: LibMeta,
    blobs: BTreeMap<Cid, u64>,
    objects: BTreeMap<String, u64>,
    /// Blobs + objects + manifests, plus uploads in flight.
    usage: u64,
}

pub struct Share {
    pub library: LibraryId,
    pub version: u32,
    pub expires_ms: Option<i64>,
    pub cids: HashSet<Cid>,
    pub manifest_len: u64,
}

/// What an upload is for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    Blob(Cid),
    Object(String),
}

/// An upload the service handed out a URL for and has reserved quota for, until it is committed or runs out. Kept
/// as a record too, so a restart (or a Durable Object that was put to sleep) neither loses the reservation nor
/// leaves bytes in the bucket that nobody deletes.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Upload {
    pub library: [u8; 16],
    pub target: Target,
    pub size: u64,
    /// What the bytes hash to (the blob's id, or the object's declared hash). Checked when they pass through the
    /// service; a bucket cannot check it, and a reader checks it anyway.
    pub hash: [u8; 32],
    pub expires_ms: i64,
    /// The bytes arrived and were recorded (they went through the service).
    pub done: bool,
}

pub type Token = [u8; 16];

#[derive(Default)]
struct Inner {
    libs: HashMap<[u8; 16], Library>,
    shares: HashMap<ShareId, Share>,
    uploads: HashMap<Token, Upload>,
}

pub struct Store {
    meta: Arc<dyn Meta>,
    /// Where the bytes are when they are not in a bucket.
    disk: Option<PathBuf>,
    inner: Mutex<Inner>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum StoreError {
    UnknownLibrary,
    WrongRoot,
    TooManyCerts,
    QuotaExceeded { used: u64, quota: u64 },
    Conflict(&'static str),
    InUse(Vec<ShareId>),
    Missing(Vec<Cid>),
}

fn lock(m: &Mutex<Inner>) -> std::sync::MutexGuard<'_, Inner> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn lib_key(lib: &[u8; 16], name: &str) -> String {
    format!("libs/{}/{name}", hex::encode(lib))
}
fn share_key(id: &ShareId, name: &str) -> String {
    format!("shares/{}/{name}", hex::encode(id))
}
fn report_key(id: &ShareId) -> String {
    format!("reports/{}", hex::encode(id))
}
fn upload_key(t: &Token) -> String {
    format!("uploads/{}", hex::encode(t))
}

fn hex_name<const N: usize>(name: &str) -> Option<[u8; N]> {
    hex::decode(name).ok()?.try_into().ok()
}

impl Store {
    /// Read every library, share and pending upload into memory. `disk` is where the bytes are kept when they
    /// are not in a bucket (a data directory, the same one the records are in).
    pub fn open(meta: Arc<dyn Meta>, disk: Option<PathBuf>) -> io::Result<Self> {
        #[cfg(feature = "server")]
        if let Some(dir) = &disk {
            crate::disk::prepare(dir)?;
        }
        #[cfg(not(feature = "server"))]
        if disk.is_some() {
            return Err(io::Error::other("this build keeps bytes in a bucket only"));
        }
        let mut inner = Inner::default();
        for name in meta.children("libs")? {
            let Some(id) = hex_name::<16>(&name) else {
                continue;
            };
            let Some(bytes) = meta.get(&lib_key(&id, "lib.postcard"))? else {
                continue;
            };
            let Ok(m) = postcard::from_bytes::<LibMeta>(&bytes) else {
                continue;
            };
            let mut lib = Library {
                meta: m,
                blobs: BTreeMap::new(),
                objects: BTreeMap::new(),
                usage: 0,
            };
            match &disk {
                #[cfg(feature = "server")]
                Some(dir) => crate::disk::scan_library(dir, &id, &mut lib.blobs, &mut lib.objects)?,
                _ => read_index(meta.as_ref(), &lib_key(&id, "index.log"), &mut lib)?,
            }
            lib.usage = lib.blobs.values().chain(lib.objects.values()).sum();
            inner.libs.insert(id, lib);
        }
        for name in meta.children("shares")? {
            let Some(id) = hex_name::<16>(&name) else {
                continue;
            };
            let Some(bytes) = meta.get(&share_key(&id, "meta.postcard"))? else {
                continue;
            };
            let Ok(m) = postcard::from_bytes::<ShareMeta>(&bytes) else {
                continue;
            };
            if let Some(lib) = inner.libs.get_mut(&m.library) {
                lib.usage += m.manifest_len;
            }
            inner.shares.insert(
                id,
                Share {
                    library: LibraryId(m.library),
                    version: m.version,
                    expires_ms: m.expires_ms,
                    cids: m.cids.into_iter().collect(),
                    manifest_len: m.manifest_len,
                },
            );
        }
        for name in meta.children("uploads")? {
            let Some(token) = hex_name::<16>(&name) else {
                continue;
            };
            let Some(u) = meta
                .get(&upload_key(&token))?
                .and_then(|b| postcard::from_bytes::<Upload>(&b).ok())
            else {
                continue;
            };
            // Still reserved until it is committed or runs out (one that went through the service is counted already).
            if !u.done {
                if let Some(lib) = inner.libs.get_mut(&u.library) {
                    lib.usage += u.size;
                }
            }
            inner.uploads.insert(token, u);
        }
        Ok(Self {
            meta,
            disk,
            inner: Mutex::new(inner),
        })
    }

    pub fn in_bucket(&self) -> bool {
        self.disk.is_none()
    }

    /// Note a change in what a library's bucket holds. A line lost to a crash leaves a blob the service does not
    /// know about: the client is told it is missing and uploads it again under the same name.
    fn log(&self, lib: LibraryId, line: std::fmt::Arguments<'_>) {
        if !self.in_bucket() {
            return;
        }
        let key = lib_key(&lib.0, "index.log");
        if let Err(e) = self.meta.append(&key, &line.to_string()) {
            tracing::error!(key, "could not note a change in the bucket: {e}");
        }
    }

    fn bytes_dir(&self) -> PathBuf {
        self.disk.clone().unwrap_or_default()
    }
    /// A temporary file for an upload going through the service.
    pub fn tmp_path(&self) -> PathBuf {
        let mut r = [0u8; 8];
        getrandom::fill(&mut r).expect("operating system randomness");
        self.bytes_dir().join("tmp").join(hex::encode(r))
    }
    pub fn blob_path(&self, lib: LibraryId, cid: &Cid) -> PathBuf {
        self.bytes_dir()
            .join(lib_key(&lib.0, "blobs"))
            .join(hex::encode(cid))
    }
    pub fn object_path(&self, lib: LibraryId, name: &str) -> PathBuf {
        self.bytes_dir().join(lib_key(&lib.0, "objects")).join(name)
    }
    pub fn manifest_path(&self, id: &ShareId) -> PathBuf {
        self.bytes_dir().join(share_key(id, "manifest"))
    }

    /// Take a share down because someone reported it (W1-10). The ciphertext stays, as does the share's record, so
    /// the owner can be told and the operator can review; the share just stops being served. Returns `false` for a
    /// share that does not exist. The first report's time and words are kept.
    pub fn report_share(&self, id: &ShareId, now_ms: i64, reason: &str) -> bool {
        if !lock(&self.inner).shares.contains_key(id) {
            return false;
        }
        if self.report_of(id).is_some() {
            return true;
        }
        let _ = self
            .meta
            .put(&report_key(id), format!("{now_ms}\n{reason}").as_bytes());
        true
    }

    /// When and why a share was reported, if it was. Reinstating is deleting the record ([`Store::reinstate`]).
    pub fn report_of(&self, id: &ShareId) -> Option<(i64, String)> {
        let bytes = self.meta.get(&report_key(id)).ok()??;
        let text = String::from_utf8(bytes).ok()?;
        let (at, reason) = text.split_once('\n')?;
        Some((at.parse().ok()?, reason.to_string()))
    }

    /// Every share taken down by a report, for the operator: id, owner, when and why.
    pub fn reports(&self) -> Vec<(ShareId, Option<LibraryId>, i64, String)> {
        let mut out = vec![];
        for name in self.meta.children("reports").unwrap_or_default() {
            let Some(id) = hex_name::<16>(&name) else {
                continue;
            };
            if let Some((at, why)) = self.report_of(&id) {
                let lib = lock(&self.inner).shares.get(&id).map(|s| s.library);
                out.push((id, lib, at, why));
            }
        }
        out.sort_by_key(|r| r.2);
        out
    }

    /// Serve a reported share again (the operator looked and found nothing wrong). False if it was not reported.
    pub fn reinstate(&self, id: &ShareId) -> bool {
        if self.report_of(id).is_none() {
            return false;
        }
        self.meta.delete(&report_key(id)).is_ok()
    }

    // ---- libraries and their certificates ----

    /// Check that a presented chain makes `device` a current member, and remember what it presented.
    /// An unknown library is created from its first request when `open` is true (the root key is then fixed).
    pub fn admit(
        &self,
        library: LibraryId,
        root: RootPublicKey,
        presented_admin: Option<&AdminCert>,
        presented_device: &DeviceCert,
        open: bool,
    ) -> Result<(), StoreError> {
        let mut inner = lock(&self.inner);
        let known = inner.libs.contains_key(&library.0);
        if !known && !open {
            return Err(StoreError::UnknownLibrary);
        }
        let lib = inner.libs.entry(library.0).or_insert_with(|| Library {
            meta: LibMeta {
                root,
                admins: vec![],
                devices: vec![],
            },
            blobs: BTreeMap::new(),
            objects: BTreeMap::new(),
            usage: 0,
        });
        if lib.meta.root != root {
            return Err(StoreError::WrongRoot);
        }
        let mut chain = TrustChain::new(root, library);
        for a in lib.meta.admins.iter().chain(presented_admin) {
            chain.add_admin(a.clone());
        }
        for d in lib.meta.devices.iter().chain([presented_device]) {
            chain.add_device(d.clone());
        }
        let member = chain.validate().is_member(&presented_device.device);
        if !member {
            if !known {
                inner.libs.remove(&library.0);
            }
            return Err(StoreError::UnknownLibrary);
        }
        let lib = inner.libs.get_mut(&library.0).expect("just inserted");
        let mut changed = !known;
        if let Some(a) = presented_admin {
            if !lib.meta.admins.contains(a) {
                lib.meta.admins.push(a.clone());
                changed = true;
            }
        }
        if !lib.meta.devices.contains(presented_device) {
            lib.meta.devices.push(presented_device.clone());
            changed = true;
        }
        if lib.meta.admins.len() + lib.meta.devices.len() > MAX_CERTS {
            return Err(StoreError::TooManyCerts);
        }
        if changed {
            self.save_lib(library, &lib.meta)
                .map_err(|_| StoreError::Conflict("could not save"))?;
        }
        Ok(())
    }

    /// Remember certificates a member sends (revocations above all), so a revoked device stops being served.
    pub fn add_certs(
        &self,
        library: LibraryId,
        admins: Vec<AdminCert>,
        devices: Vec<DeviceCert>,
    ) -> Result<(), StoreError> {
        let mut inner = lock(&self.inner);
        let lib = inner
            .libs
            .get_mut(&library.0)
            .ok_or(StoreError::UnknownLibrary)?;
        let chain_ok = |c: &TrustChain, d: &DeviceCert| c.signature_ok(d);
        let mut probe = TrustChain::new(lib.meta.root, library);
        for a in &lib.meta.admins {
            probe.add_admin(a.clone());
        }
        let mut new_admins = vec![];
        for a in admins {
            if probe.check_admin(&a).is_ok() && !lib.meta.admins.contains(&a) {
                new_admins.push(a);
            }
        }
        let mut new_devices = vec![];
        for d in devices {
            if chain_ok(&probe, &d) && !lib.meta.devices.contains(&d) {
                new_devices.push(d);
            }
        }
        if lib.meta.admins.len() + lib.meta.devices.len() + new_admins.len() + new_devices.len()
            > MAX_CERTS
        {
            return Err(StoreError::TooManyCerts);
        }
        lib.meta.admins.extend(new_admins);
        lib.meta.devices.extend(new_devices);
        self.save_lib(library, &lib.meta)
            .map_err(|_| StoreError::Conflict("could not save"))
    }

    fn save_lib(&self, library: LibraryId, meta: &LibMeta) -> io::Result<()> {
        self.meta.put(
            &lib_key(&library.0, "lib.postcard"),
            &postcard::to_stdvec(meta).expect("certificates encode"),
        )
    }

    // ---- usage ----

    pub fn usage(&self, lib: LibraryId) -> u64 {
        lock(&self.inner).libs.get(&lib.0).map_or(0, |l| l.usage)
    }

    /// Set aside `bytes` of a library's quota for an upload; give it back with [`Store::release`] if the upload fails.
    pub fn reserve(&self, lib: LibraryId, bytes: u64, quota: u64) -> Result<(), StoreError> {
        let mut inner = lock(&self.inner);
        let l = inner
            .libs
            .get_mut(&lib.0)
            .ok_or(StoreError::UnknownLibrary)?;
        if l.usage.saturating_add(bytes) > quota {
            return Err(StoreError::QuotaExceeded {
                used: l.usage,
                quota,
            });
        }
        l.usage += bytes;
        Ok(())
    }

    pub fn release(&self, lib: LibraryId, bytes: u64) {
        if let Some(l) = lock(&self.inner).libs.get_mut(&lib.0) {
            l.usage = l.usage.saturating_sub(bytes);
        }
    }

    // ---- blobs ----

    pub fn has_blob(&self, lib: LibraryId, cid: &Cid) -> Option<u64> {
        lock(&self.inner)
            .libs
            .get(&lib.0)
            .and_then(|l| l.blobs.get(cid).copied())
    }

    /// Record a blob whose file is in place and whose bytes were reserved.
    pub fn blob_stored(&self, lib: LibraryId, cid: Cid, len: u64) {
        if let Some(l) = lock(&self.inner).libs.get_mut(&lib.0) {
            if l.blobs.insert(cid, len).is_some() {
                // Raced with another upload of the same blob: the reservation was a double count.
                l.usage = l.usage.saturating_sub(len);
                return;
            }
            self.log(lib, format_args!("+b {} {len}", hex::encode(cid)));
        }
    }

    pub fn list_blobs(&self, lib: LibraryId, after: Option<Cid>, limit: usize) -> Vec<(Cid, u64)> {
        let inner = lock(&self.inner);
        let Some(l) = inner.libs.get(&lib.0) else {
            return vec![];
        };
        use std::ops::Bound::{Excluded, Unbounded};
        let start = after.map_or(Unbounded, Excluded);
        l.blobs
            .range((start, Unbounded))
            .take(limit)
            .map(|(c, n)| (*c, *n))
            .collect()
    }

    pub fn missing_blobs(&self, lib: LibraryId, cids: &[Cid]) -> Vec<Cid> {
        let inner = lock(&self.inner);
        let Some(l) = inner.libs.get(&lib.0) else {
            return cids.to_vec();
        };
        cids.iter()
            .filter(|c| !l.blobs.contains_key(*c))
            .copied()
            .collect()
    }

    /// Forget a blob (false when it was not there). Refused while a share lists it. The caller deletes the bytes.
    pub fn remove_blob(&self, lib: LibraryId, cid: &Cid) -> Result<bool, StoreError> {
        let mut inner = lock(&self.inner);
        let using: Vec<ShareId> = inner
            .shares
            .iter()
            .filter(|(_, s)| s.library == lib && s.cids.contains(cid))
            .map(|(id, _)| *id)
            .collect();
        if !using.is_empty() {
            return Err(StoreError::InUse(using));
        }
        let l = inner
            .libs
            .get_mut(&lib.0)
            .ok_or(StoreError::UnknownLibrary)?;
        let Some(n) = l.blobs.remove(cid) else {
            return Ok(false);
        };
        l.usage = l.usage.saturating_sub(n);
        self.log(lib, format_args!("-b {}", hex::encode(cid)));
        Ok(true)
    }

    // ---- objects ----

    pub fn object_len(&self, lib: LibraryId, name: &str) -> Option<u64> {
        lock(&self.inner)
            .libs
            .get(&lib.0)
            .and_then(|l| l.objects.get(name).copied())
    }

    /// Record an object (replacing one of the same name). `reserved` is what the upload reserved.
    pub fn object_stored(&self, lib: LibraryId, name: &str, len: u64, reserved: u64) {
        if let Some(l) = lock(&self.inner).libs.get_mut(&lib.0) {
            let old = l.objects.insert(name.to_string(), len).unwrap_or(0);
            l.usage = l.usage.saturating_sub(old);
            l.usage = l.usage.saturating_sub(reserved) + len;
            self.log(lib, format_args!("+o {name} {len}"));
        }
    }

    pub fn list_objects(&self, lib: LibraryId) -> Vec<(String, u64)> {
        lock(&self.inner)
            .libs
            .get(&lib.0)
            .map(|l| l.objects.iter().map(|(n, s)| (n.clone(), *s)).collect())
            .unwrap_or_default()
    }

    /// Forget an object (false when it was not there). The caller deletes the bytes.
    pub fn remove_object(&self, lib: LibraryId, name: &str) -> bool {
        let mut inner = lock(&self.inner);
        let Some(l) = inner.libs.get_mut(&lib.0) else {
            return false;
        };
        let Some(n) = l.objects.remove(name) else {
            return false;
        };
        l.usage = l.usage.saturating_sub(n);
        self.log(lib, format_args!("-o {name}"));
        true
    }

    // ---- shares ----

    pub fn share_info(&self, id: &ShareId) -> Option<(LibraryId, u32, Option<i64>, u64)> {
        lock(&self.inner)
            .shares
            .get(id)
            .map(|s| (s.library, s.version, s.expires_ms, s.manifest_len))
    }

    pub fn shares_of(&self, lib: LibraryId) -> Vec<(ShareId, u32, Option<i64>, u64)> {
        let mut v: Vec<_> = lock(&self.inner)
            .shares
            .iter()
            .filter(|(_, s)| s.library == lib)
            .map(|(id, s)| (*id, s.version, s.expires_ms, s.manifest_len))
            .collect();
        v.sort();
        v
    }

    /// May this share serve this blob right now? Returns the library that owns the blob.
    pub fn share_serves(&self, id: &ShareId, cid: &Cid) -> Option<LibraryId> {
        lock(&self.inner)
            .shares
            .get(id)
            .filter(|s| s.cids.contains(cid))
            .map(|s| s.library)
    }

    /// May this share be created or replaced like this? Every cid must already be among the library's blobs and a
    /// lower version than the stored one is refused (so a late, stale upload cannot roll a share back).
    pub fn check_share(
        &self,
        lib: LibraryId,
        id: &ShareId,
        version: u32,
        cids: &[Cid],
        manifest_len: u64,
        quota: u64,
    ) -> Result<(), StoreError> {
        let inner = lock(&self.inner);
        Self::check_share_locked(&inner, lib, id, version, cids, manifest_len, quota).map(|_| ())
    }

    fn check_share_locked(
        inner: &Inner,
        lib: LibraryId,
        id: &ShareId,
        version: u32,
        cids: &[Cid],
        manifest_len: u64,
        quota: u64,
    ) -> Result<u64, StoreError> {
        let l = inner.libs.get(&lib.0).ok_or(StoreError::UnknownLibrary)?;
        let missing: Vec<Cid> = cids
            .iter()
            .filter(|c| !l.blobs.contains_key(*c))
            .take(50)
            .copied()
            .collect();
        if !missing.is_empty() {
            return Err(StoreError::Missing(missing));
        }
        let old_len = match inner.shares.get(id) {
            Some(s) if s.library != lib => return Err(StoreError::Conflict("share id in use")),
            Some(s) if version < s.version => return Err(StoreError::Conflict("older version")),
            Some(s) => s.manifest_len,
            None => 0,
        };
        let after = l.usage.saturating_sub(old_len) + manifest_len;
        if after > quota {
            return Err(StoreError::QuotaExceeded {
                used: l.usage,
                quota,
            });
        }
        Ok(after)
    }

    /// Create or replace a share (see [`Store::check_share`]). `manifest` is written beside the record, unless the
    /// bytes are in a bucket, where the caller has put the `manifest_len` bytes already.
    #[allow(clippy::too_many_arguments)]
    pub fn put_share(
        &self,
        lib: LibraryId,
        id: ShareId,
        version: u32,
        expires_ms: Option<i64>,
        cids: Vec<Cid>,
        manifest: Option<&[u8]>,
        manifest_len: u64,
        quota: u64,
    ) -> Result<(), StoreError> {
        let mut inner = lock(&self.inner);
        let after =
            Self::check_share_locked(&inner, lib, &id, version, &cids, manifest_len, quota)?;
        let new_len = manifest_len;
        let l = inner.libs.get_mut(&lib.0).expect("checked above");
        let meta = ShareMeta {
            library: lib.0,
            version,
            expires_ms,
            cids: cids.clone(),
            manifest_len: new_len,
        };
        manifest
            .map_or(Ok(()), |m| self.meta.put(&share_key(&id, "manifest"), m))
            .and_then(|_| {
                self.meta.put(
                    &share_key(&id, "meta.postcard"),
                    &postcard::to_stdvec(&meta).expect("share meta encodes"),
                )
            })
            .map_err(|_| StoreError::Conflict("could not save"))?;
        l.usage = after;
        inner.shares.insert(
            id,
            Share {
                library: lib,
                version,
                expires_ms,
                cids: cids.into_iter().collect(),
                manifest_len: new_len,
            },
        );
        Ok(())
    }

    /// Delete a share (the owner's). Its blobs stay with the library.
    pub fn delete_share(&self, lib: LibraryId, id: &ShareId) -> bool {
        let mut inner = lock(&self.inner);
        match inner.shares.get(id) {
            Some(s) if s.library == lib => {}
            _ => return false,
        }
        let s = inner.shares.remove(id).expect("checked");
        if let Some(l) = inner.libs.get_mut(&lib.0) {
            l.usage = l.usage.saturating_sub(s.manifest_len);
        }
        let _ = self.meta.delete(&share_key(id, "manifest"));
        let _ = self.meta.delete(&share_key(id, "meta.postcard"));
        let _ = self.meta.delete(&report_key(id));
        true
    }

    /// The library that owns a share, whoever asks (the operator deleting a reported share).
    pub fn share_owner(&self, id: &ShareId) -> Option<LibraryId> {
        lock(&self.inner).shares.get(id).map(|s| s.library)
    }

    // ---- uploads handed out ----

    /// Remember an upload whose quota was reserved with [`Store::reserve`].
    pub fn upload_begun(&self, token: Token, u: Upload) -> Result<(), StoreError> {
        self.meta
            .put(
                &upload_key(&token),
                &postcard::to_stdvec(&u).expect("an upload encodes"),
            )
            .map_err(|_| StoreError::Conflict("could not save"))?;
        lock(&self.inner).uploads.insert(token, u);
        Ok(())
    }

    pub fn upload(&self, token: &Token) -> Option<Upload> {
        lock(&self.inner).uploads.get(token).cloned()
    }

    /// The bytes of an upload arrived through the service and were recorded.
    pub fn upload_done(&self, token: &Token) {
        let mut inner = lock(&self.inner);
        if let Some(u) = inner.uploads.get_mut(token) {
            u.done = true;
            let _ = self.meta.put(
                &upload_key(token),
                &postcard::to_stdvec(&*u).expect("an upload encodes"),
            );
        }
    }

    /// Take an upload out (committed, or given up). Only one caller gets it, so two commits count once.
    pub fn take_upload(&self, token: &Token) -> Option<Upload> {
        let u = lock(&self.inner).uploads.remove(token)?;
        let _ = self.meta.delete(&upload_key(token));
        Some(u)
    }

    /// Take out every upload that ran out by `now_ms` (a janitor run); the caller releases their quota.
    pub fn take_expired_uploads(&self, now_ms: i64) -> Vec<Upload> {
        let tokens: Vec<Token> = lock(&self.inner)
            .uploads
            .iter()
            .filter(|(_, u)| u.expires_ms <= now_ms)
            .map(|(t, _)| *t)
            .collect();
        tokens.iter().filter_map(|t| self.take_upload(t)).collect()
    }

    /// Delete expired shares (a janitor run), returning which. With a bucket the caller deletes their manifests.
    pub fn sweep_expired(&self, now_ms: i64) -> Vec<ShareId> {
        let expired: Vec<(LibraryId, ShareId)> = lock(&self.inner)
            .shares
            .iter()
            .filter(|(_, s)| s.expires_ms.is_some_and(|e| now_ms >= e))
            .map(|(id, s)| (s.library, *id))
            .collect();
        expired
            .into_iter()
            .filter(|(lib, id)| self.delete_share(*lib, id))
            .map(|(_, id)| id)
            .collect()
    }
}

/// Read a library's `index.log` into it, then write it back with only what is there now.
fn read_index(meta: &dyn Meta, key: &str, lib: &mut Library) -> io::Result<()> {
    let Some(bytes) = meta.get(key)? else {
        return Ok(());
    };
    let text = String::from_utf8_lossy(&bytes);
    for line in text.lines() {
        let mut parts = line.split(' ');
        match (parts.next(), parts.next(), parts.next()) {
            (Some("+b"), Some(cid), Some(n)) => {
                if let (Some(cid), Ok(n)) = (hex_name::<32>(cid), n.parse()) {
                    lib.blobs.insert(cid, n);
                }
            }
            (Some("-b"), Some(cid), None) => {
                if let Some(cid) = hex_name::<32>(cid) {
                    lib.blobs.remove(&cid);
                }
            }
            (Some("+o"), Some(name), Some(n)) => {
                if let Ok(n) = n.parse() {
                    lib.objects.insert(name.to_string(), n);
                }
            }
            (Some("-o"), Some(name), None) => {
                lib.objects.remove(name);
            }
            _ => tracing::warn!(key, "skipping a line it cannot read: {line}"),
        }
    }
    let mut out = String::new();
    for (cid, n) in &lib.blobs {
        out.push_str(&format!("+b {} {n}\n", hex::encode(cid)));
    }
    for (name, n) in &lib.objects {
        out.push_str(&format!("+o {name} {n}\n"));
    }
    meta.put(key, out.as_bytes())
}
