//! What the service keeps: libraries (their root key and certificates), blobs and objects they uploaded, and
//! shares. Files on disk are the truth; a map in memory answers questions without touching the disk.
//!
//! ```text
//! <data>/libs/<library hex>/lib.postcard      root key + certificates
//! <data>/libs/<library hex>/blobs/<cid hex>   ciphertext
//! <data>/libs/<library hex>/objects/<name>    named encrypted objects (snapshots)
//! <data>/shares/<share id hex>/meta.postcard  who owns it, version, expiry, the cids it lists
//! <data>/shares/<share id hex>/manifest       the sealed manifest
//! <data>/tmp/                                 uploads in progress
//! ```
//!
//! Nothing here is plaintext: blobs and manifests arrive encrypted and stay that way.

use serde::{Deserialize, Serialize};
use shoebox_core::identity::TrustChain;
use shoebox_core::model::{AdminCert, DeviceCert};
use shoebox_core::store::{LibraryId, RootPublicKey};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

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

#[derive(Default)]
struct Inner {
    libs: HashMap<[u8; 16], Library>,
    shares: HashMap<ShareId, Share>,
}

pub struct Store {
    dir: PathBuf,
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

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

fn hex_name<const N: usize>(name: &str) -> Option<[u8; N]> {
    hex::decode(name).ok()?.try_into().ok()
}

impl Store {
    /// Open (and create) a data directory, reading every library and share into memory.
    pub fn open(dir: &Path) -> io::Result<Self> {
        for sub in ["libs", "shares", "tmp"] {
            std::fs::create_dir_all(dir.join(sub))?;
        }
        // Uploads that were in flight when the service stopped are not resumable.
        for e in std::fs::read_dir(dir.join("tmp"))? {
            let _ = std::fs::remove_file(e?.path());
        }
        let mut inner = Inner::default();
        for e in std::fs::read_dir(dir.join("libs"))? {
            let e = e?;
            let Some(id) = hex_name::<16>(&e.file_name().to_string_lossy()) else {
                continue;
            };
            let Ok(bytes) = std::fs::read(e.path().join("lib.postcard")) else {
                continue;
            };
            let Ok(meta) = postcard::from_bytes::<LibMeta>(&bytes) else {
                continue;
            };
            let mut lib = Library {
                meta,
                blobs: BTreeMap::new(),
                objects: BTreeMap::new(),
                usage: 0,
            };
            for (sub, is_blob) in [("blobs", true), ("objects", false)] {
                let Ok(rd) = std::fs::read_dir(e.path().join(sub)) else {
                    continue;
                };
                for f in rd {
                    let f = f?;
                    let name = f.file_name().to_string_lossy().into_owned();
                    let len = f.metadata()?.len();
                    if is_blob {
                        if let Some(cid) = hex_name::<32>(&name) {
                            lib.blobs.insert(cid, len);
                            lib.usage += len;
                        }
                    } else {
                        lib.objects.insert(name, len);
                        lib.usage += len;
                    }
                }
            }
            inner.libs.insert(id, lib);
        }
        for e in std::fs::read_dir(dir.join("shares"))? {
            let e = e?;
            let Some(id) = hex_name::<16>(&e.file_name().to_string_lossy()) else {
                continue;
            };
            let Ok(bytes) = std::fs::read(e.path().join("meta.postcard")) else {
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
        Ok(Self {
            dir: dir.to_path_buf(),
            inner: Mutex::new(inner),
        })
    }

    pub fn tmp_path(&self) -> PathBuf {
        let mut r = [0u8; 8];
        getrandom::fill(&mut r).expect("operating system randomness");
        self.dir.join("tmp").join(hex::encode(r))
    }
    fn lib_dir(&self, lib: LibraryId) -> PathBuf {
        self.dir.join("libs").join(hex::encode(lib.0))
    }
    pub fn blob_path(&self, lib: LibraryId, cid: &Cid) -> PathBuf {
        self.lib_dir(lib).join("blobs").join(hex::encode(cid))
    }
    pub fn object_path(&self, lib: LibraryId, name: &str) -> PathBuf {
        self.lib_dir(lib).join("objects").join(name)
    }
    fn share_dir(&self, id: &ShareId) -> PathBuf {
        self.dir.join("shares").join(hex::encode(id))
    }
    pub fn manifest_path(&self, id: &ShareId) -> PathBuf {
        self.share_dir(id).join("manifest")
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
        let dir = self.lib_dir(library);
        std::fs::create_dir_all(dir.join("blobs"))?;
        std::fs::create_dir_all(dir.join("objects"))?;
        write_atomic(
            &dir.join("lib.postcard"),
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
            }
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

    /// Forget a blob. Refused while a share lists it.
    pub fn remove_blob(&self, lib: LibraryId, cid: &Cid) -> Result<Option<PathBuf>, StoreError> {
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
        Ok(l.blobs.remove(cid).map(|n| {
            l.usage = l.usage.saturating_sub(n);
            self.blob_path(lib, cid)
        }))
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
        }
    }

    pub fn list_objects(&self, lib: LibraryId) -> Vec<(String, u64)> {
        lock(&self.inner)
            .libs
            .get(&lib.0)
            .map(|l| l.objects.iter().map(|(n, s)| (n.clone(), *s)).collect())
            .unwrap_or_default()
    }

    pub fn remove_object(&self, lib: LibraryId, name: &str) -> Option<PathBuf> {
        let mut inner = lock(&self.inner);
        let l = inner.libs.get_mut(&lib.0)?;
        l.objects.remove(name).map(|n| {
            l.usage = l.usage.saturating_sub(n);
            self.object_path(lib, name)
        })
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

    /// Create or replace a share. Every cid must already be among the library's blobs; a lower version than the
    /// stored one is refused (so a late, stale upload cannot roll a share back).
    #[allow(clippy::too_many_arguments)]
    pub fn put_share(
        &self,
        lib: LibraryId,
        id: ShareId,
        version: u32,
        expires_ms: Option<i64>,
        cids: Vec<Cid>,
        manifest: &[u8],
        quota: u64,
    ) -> Result<(), StoreError> {
        let mut inner = lock(&self.inner);
        {
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
        }
        let old_len = match inner.shares.get(&id) {
            Some(s) if s.library != lib => return Err(StoreError::Conflict("share id in use")),
            Some(s) if version < s.version => return Err(StoreError::Conflict("older version")),
            Some(s) => s.manifest_len,
            None => 0,
        };
        let new_len = manifest.len() as u64;
        let l = inner.libs.get_mut(&lib.0).expect("checked above");
        let after = l.usage.saturating_sub(old_len) + new_len;
        if after > quota {
            return Err(StoreError::QuotaExceeded {
                used: l.usage,
                quota,
            });
        }
        let dir = self.share_dir(&id);
        std::fs::create_dir_all(&dir).map_err(|_| StoreError::Conflict("could not save"))?;
        let meta = ShareMeta {
            library: lib.0,
            version,
            expires_ms,
            cids: cids.clone(),
            manifest_len: new_len,
        };
        write_atomic(&dir.join("manifest"), manifest)
            .and_then(|_| {
                write_atomic(
                    &dir.join("meta.postcard"),
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
        let _ = std::fs::remove_dir_all(self.share_dir(id));
        true
    }

    /// Delete expired shares (a janitor run).
    pub fn sweep_expired(&self, now_ms: i64) -> usize {
        let expired: Vec<(LibraryId, ShareId)> = lock(&self.inner)
            .shares
            .iter()
            .filter(|(_, s)| s.expires_ms.is_some_and(|e| now_ms >= e))
            .map(|(id, s)| (s.library, *id))
            .collect();
        expired
            .iter()
            .filter(|(lib, id)| self.delete_share(*lib, id))
            .count()
    }
}
