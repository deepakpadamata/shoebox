//! The cloud keeper (W1-09): the paid use of Shoebox Cloud, where a library's blobs are held there as ciphertext.
//!
//! The blobs are the same sealed objects a web link serves (`cid = blake3(ciphertext)`, key from the cloud secret and
//! the blob's own hash), so a shared album costs the keeper's library nothing more. Originals are sealed as they
//! are, with their location: this is the person's own backup, not a share. Besides the blobs, two small objects hold
//! what a restore needs, each sealed under a key from its own content and named by that content's hash:
//!
//! * `catalog.<hash>`: plaintext hash, ciphertext id, size and kind of every blob uploaded;
//! * `state.<hash>`: the library's whole record state (what [`LibraryStore::export_state`] returns).
//!
//! What this device has confirmed on the service lives in `cloud-catalog.bin` next to the blob store.

use super::*;
use shoebox_core::cloud::{decrypt_blob, encrypt_blob};
use shoebox_core::model::BlobKind;
use std::collections::HashSet;

const CATALOG_FILE: &str = "cloud-catalog.bin";
/// A confirmation counts for a week, as a device's does (free up space).
const CONFIRMED_FOR_MS: u64 = 7 * 24 * 60 * 60 * 1000;
const BATCH_BLOBS: usize = 32;

/// What to upload and how much, for this run. The caller decides *when*: a keeper overnight, a desktop on mains
/// power, a phone on Wi-Fi while charging (or on mobile data only if the person allowed it).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CloudPolicy {
    /// Also upload originals and motion clips. Without it only thumbnails and previews go (a phone on mobile data).
    pub originals: bool,
    /// Stop after about this many bytes of ciphertext; the next run carries on.
    pub byte_budget: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CloudBackupReport {
    pub uploaded: u32,
    pub uploaded_bytes: u64,
    /// Confirmed on the service within the last week: not asked again.
    pub already: u32,
    /// Left for the next run (the budget ran out).
    pub left: u32,
    /// Not on this device (a phone that has previews only): another device uploads those.
    pub not_here: u32,
    pub failed: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CloudRestoreReport {
    pub restored: u32,
    pub already: u32,
    pub state_imported: bool,
    pub failed: Vec<String>,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
struct Entry {
    cid: [u8; 32],
    size: u64,
    kind: u8,
    confirmed_ms: u64,
}

#[derive(Default, Serialize, Deserialize)]
struct Catalog(BTreeMap<[u8; 32], Entry>);

fn kind_byte(k: BlobKind) -> u8 {
    match k {
        BlobKind::Original => 0,
        BlobKind::Preview => 1,
        BlobKind::Thumb => 2,
        BlobKind::Index => 3,
        BlobKind::Motion => 4,
    }
}

fn load_catalog(dir: &std::path::Path) -> Catalog {
    std::fs::read(dir.join(CATALOG_FILE))
        .ok()
        .and_then(|b| postcard::from_bytes(&b).ok())
        .unwrap_or_default()
}

fn save_catalog(dir: &std::path::Path, c: &Catalog) -> Result<()> {
    let tmp = dir.join("cloud-catalog.tmp");
    std::fs::write(&tmp, postcard::to_stdvec(c)?)?;
    std::fs::rename(&tmp, dir.join(CATALOG_FILE))?;
    Ok(())
}

impl Session {
    async fn get_bytes(&self, path: &str, now_ms: i64) -> Result<Vec<u8>> {
        let res = self.call("GET", path, vec![], now_ms).await?;
        if !res.status().is_success() {
            return Err(Self::fail(res, "downloading").await);
        }
        Ok(res.bytes().await?.to_vec())
    }

    async fn objects(&self, now_ms: i64) -> Result<Vec<String>> {
        let v: serde_json::Value =
            serde_json::from_slice(&self.get_bytes("/v1/objects", now_ms).await?)?;
        Ok(v["objects"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|o| o["name"].as_str().map(String::from))
            .collect())
    }

    /// Store a small sealed object under `<prefix>.<hash of its plaintext>` and drop the older ones with that prefix.
    async fn put_sealed_object(&self, prefix: &str, plain: &[u8], now_ms: i64) -> Result<()> {
        let sealed = encrypt_blob(&self.secret, plain);
        let name = format!("{prefix}.{}", hex::encode(blake3::hash(plain).as_bytes()));
        let path = format!("/v1/objects/{name}");
        let res = self
            .http
            .put(self.url(&path))
            .header(
                AUTHORIZATION,
                self.auth("PUT", &path, &sealed.cid.0, now_ms),
            )
            .header("x-content-blake3", hex::encode(sealed.cid.0))
            .header(CONTENT_LENGTH, sealed.ciphertext.len())
            .body(sealed.ciphertext)
            .send()
            .await
            .context("uploading library state")?;
        if !res.status().is_success() {
            return Err(Self::fail(res, "uploading library state").await);
        }
        let old = format!("{prefix}.");
        for other in self.objects(now_ms).await? {
            if other.starts_with(&old) && other != name {
                let p = format!("/v1/objects/{other}");
                let _ = self.call("DELETE", &p, vec![], now_ms).await?;
            }
        }
        Ok(())
    }

    /// The newest object with this prefix, opened and checked against the hash in its name.
    async fn get_sealed_object(&self, prefix: &str, now_ms: i64) -> Result<Option<Vec<u8>>> {
        let want = format!("{prefix}.");
        let Some(name) = self
            .objects(now_ms)
            .await?
            .into_iter()
            .find(|n| n.starts_with(&want))
        else {
            return Ok(None);
        };
        let hash: [u8; 32] = hex::decode(&name[want.len()..])
            .ok()
            .and_then(|b| b.try_into().ok())
            .context("a stored object has a malformed name")?;
        let sealed = self
            .get_bytes(&format!("/v1/objects/{name}"), now_ms)
            .await?;
        let plain = decrypt_blob(&self.secret.blob_key(&Blake3Hash(hash)), &sealed)
            .map_err(|e| anyhow!("{name} does not open: {e}"))?;
        ensure!(
            *blake3::hash(&plain).as_bytes() == hash,
            "{name} does not match its name"
        );
        Ok(Some(plain))
    }
}

/// The library setting that says the person wants the whole library held in the cloud. Off until they turn it on:
/// it uses their quota, and a keeper starts uploading as soon as it sees it.
pub const CLOUD_BACKUP_KEY: &str = "cloud-backup";

impl<L: LibraryStore> LibraryNode<L> {
    pub async fn set_cloud_backup(&self, on: bool) -> Result<()> {
        self.store()
            .put_setting(CLOUD_BACKUP_KEY, vec![u8::from(on)])
            .await?;
        Ok(())
    }

    pub async fn cloud_backup_enabled(&self) -> Result<bool> {
        Ok(self
            .store()
            .setting(CLOUD_BACKUP_KEY)
            .await?
            .is_some_and(|b| b.first() == Some(&1)))
    }

    /// One round of everything a keeper keeps up on its own: web links brought up to date (Cloud and Live) and, when
    /// the person turned the cloud copy on, the library uploaded.
    pub async fn cloud_upkeep(&self) -> Result<CloudBackupReport> {
        let mut report = CloudBackupReport::default();
        if !self.cloud_enabled().await? || self.cloud_url().await?.is_none() {
            return Ok(report);
        }
        let links = self.refresh_web_shares().await?;
        report.failed.extend(links.failed);
        let live = self.sync_live_shares(false).await?;
        report.failed.extend(live.failed);
        if self.cloud_backup_enabled().await? {
            let b = self
                .cloud_backup(&CloudPolicy {
                    originals: true,
                    byte_budget: None,
                })
                .await?;
            report.uploaded += b.uploaded;
            report.uploaded_bytes += b.uploaded_bytes;
            report.already += b.already;
            report.not_here += b.not_here;
            report.failed.extend(b.failed);
        }
        Ok(report)
    }

    /// Plaintext hashes of the originals this device has seen on the service within the last week: the encrypted
    /// copy, as free up space counts it.
    pub(crate) fn cloud_held(&self, now_ms: u64) -> HashSet<Blake3Hash> {
        let dir = self.node().config().data_dir;
        load_catalog(&dir)
            .0
            .into_iter()
            .filter(|(_, e)| {
                e.kind == kind_byte(BlobKind::Original)
                    && now_ms.saturating_sub(e.confirmed_ms) <= CONFIRMED_FOR_MS
            })
            .map(|(h, _)| Blake3Hash(h))
            .collect()
    }

    /// Upload what the service does not hold yet, newest photos first, then the catalog and the library state.
    /// Safe to stop at any point: each blob is stored whole or not at all (the service checks its hash), and the next
    /// run asks which are there.
    pub async fn cloud_backup(&self, policy: &CloudPolicy) -> Result<CloudBackupReport> {
        let s = self.session().await?;
        let dir = self.node().config().data_dir;
        let mut catalog = load_catalog(&dir);
        let now = self.now_ms();
        let mut report = CloudBackupReport::default();
        let blobs = self.node().blobs();

        // What this library has, in the order it should go up.
        let photos = self
            .store()
            .photos_by_time(shoebox_core::store::TimeRange::ALL, Page::all())
            .await?;
        let mut wanted: Vec<(Blake3Hash, BlobKind)> = vec![];
        for p in &photos {
            wanted.push((p.thumb.hash, BlobKind::Thumb));
            wanted.push((p.preview.hash, BlobKind::Preview));
            if policy.originals {
                wanted.push((p.original.hash, BlobKind::Original));
                if let Some(m) = self.store().motion(&p.id).await? {
                    wanted.push((m.video.hash, BlobKind::Motion));
                }
            }
        }
        let mut seen = HashSet::new();
        let mut todo: Vec<(Blake3Hash, BlobKind)> = vec![];
        for (h, k) in wanted {
            if !seen.insert(h) {
                continue;
            }
            if !blobs.has(&h).await? {
                report.not_here += 1;
                continue;
            }
            let fresh = catalog
                .0
                .get(&h.0)
                .is_some_and(|e| now.saturating_sub(e.confirmed_ms) <= CONFIRMED_FOR_MS);
            if fresh {
                report.already += 1;
            } else {
                todo.push((h, k));
            }
        }

        let mut budget = policy.byte_budget;
        let mut rest = todo.as_slice();
        while !rest.is_empty() {
            if budget == Some(0) {
                report.left += rest.len() as u32;
                break;
            }
            let (batch, tail) = rest.split_at(rest.len().min(BATCH_BLOBS));
            rest = tail;
            let mut staged: Vec<(Staged, BlobKind, Blake3Hash)> = vec![];
            let mut broke = false;
            for (h, k) in batch {
                match self.stage(&s, Plain::Stored(*h), false).await {
                    Ok(st) => staged.push((st, *k, *h)),
                    Err(e) => {
                        report.failed.push(format!("{h}: {e:#}"));
                        broke = true;
                        break;
                    }
                }
            }
            let cids: Vec<[u8; 32]> = staged.iter().map(|(st, _, _)| st.cid).collect();
            let result: Result<()> = async {
                let present = s.has(&cids, now_i64(self.now_ms())).await?;
                for (st, k, h) in &staged {
                    if !present.contains(&st.cid) {
                        self.upload(&s, st).await?;
                        report.uploaded += 1;
                        report.uploaded_bytes += st.size;
                        if let Some(b) = budget.as_mut() {
                            *b = b.saturating_sub(st.size);
                        }
                    } else {
                        report.already += 1;
                    }
                    catalog.0.insert(
                        h.0,
                        Entry {
                            cid: st.cid,
                            size: st.size,
                            kind: kind_byte(*k),
                            confirmed_ms: self.now_ms(),
                        },
                    );
                }
                Ok(())
            }
            .await;
            for (st, _, _) in &staged {
                if let Some(f) = &st.file {
                    let _ = std::fs::remove_file(f);
                }
            }
            // Whatever went up is remembered even when the batch failed halfway.
            save_catalog(&dir, &catalog)?;
            s.save_cache();
            if let Err(e) = result {
                report.failed.push(format!("{e:#}"));
                break;
            }
            if broke {
                break;
            }
        }

        // The catalog and the state go last, so a restore never finds a catalog that names blobs not yet there.
        if report.failed.is_empty() && (report.uploaded > 0 || !self.cloud_state_present(&s).await)
        {
            let rows: Vec<([u8; 32], [u8; 32], u64, u8)> = catalog
                .0
                .iter()
                .map(|(h, e)| (*h, e.cid, e.size, e.kind))
                .collect();
            let t = i64::try_from(self.now_ms()).unwrap_or(i64::MAX);
            let put = async {
                s.put_sealed_object("catalog", &postcard::to_stdvec(&rows)?, t)
                    .await?;
                let state = self.store().export_state().await?;
                s.put_sealed_object("state", &state, t).await
            }
            .await;
            if let Err(e) = put {
                report.failed.push(format!("library state: {e:#}"));
            }
        }
        Ok(report)
    }

    async fn cloud_state_present(&self, s: &Session) -> bool {
        let t = i64::try_from(self.now_ms()).unwrap_or(i64::MAX);
        s.objects(t)
            .await
            .map(|o| o.iter().any(|n| n.starts_with("state.")))
            .unwrap_or(false)
    }

    /// Bring back what this device lacks: fold in the stored library state, then download, check and decrypt every
    /// blob of the catalog that is not here. Each blob is checked twice: its ciphertext against its id, and the
    /// opened bytes against the hash the library knows it by. Anything that does not match is reported and not kept.
    pub async fn cloud_restore(&self) -> Result<CloudRestoreReport> {
        let s = self.session().await?;
        let t = || i64::try_from(self.now_ms()).unwrap_or(i64::MAX);
        let mut report = CloudRestoreReport::default();
        if let Some(state) = s.get_sealed_object("state", t()).await? {
            match self.store().import_state(&state).await {
                Ok(()) => report.state_imported = true,
                Err(e) => report.failed.push(format!("library state: {e}")),
            }
        }
        let rows: Vec<([u8; 32], [u8; 32], u64, u8)> =
            match s.get_sealed_object("catalog", t()).await? {
                Some(b) => postcard::from_bytes(&b).context("the stored catalog is malformed")?,
                None => bail!("nothing has been backed up to this service yet"),
            };
        let blobs = self.node().blobs();
        for (plain, cid, size, _) in rows {
            let hash = Blake3Hash(plain);
            if blobs.has(&hash).await? {
                report.already += 1;
                continue;
            }
            match self.restore_blob(&s, &hash, &cid, size).await {
                Ok(()) => report.restored += 1,
                Err(e) => report.failed.push(format!("{hash}: {e:#}")),
            }
        }
        Ok(report)
    }

    async fn restore_blob(
        &self,
        s: &Session,
        plain: &Blake3Hash,
        cid: &[u8; 32],
        size: u64,
    ) -> Result<()> {
        let mut r = [0u8; 8];
        getrandom::fill(&mut r).map_err(|e| anyhow!("randomness: {e}"))?;
        let sealed_path = s.tmp.join(format!("r-{}", hex::encode(r)));
        let plain_path = s.tmp.join(format!("p-{}", hex::encode(r)));
        let result = async {
            // Download to a file, hashing as it arrives.
            let path = format!("/v1/blobs/{}", hex::encode(cid));
            let mut res = s.call("GET", &path, vec![], now_i64(self.now_ms())).await?;
            if !res.status().is_success() {
                return Err(Session::fail(res, "downloading a blob").await);
            }
            let mut file = tokio::fs::File::create(&sealed_path).await?;
            let mut hasher = blake3::Hasher::new();
            let mut got = 0u64;
            while let Some(chunk) = res.chunk().await? {
                hasher.update(&chunk);
                got += chunk.len() as u64;
                file.write_all(&chunk).await?;
            }
            file.flush().await?;
            drop(file);
            ensure!(
                hasher.finalize().as_bytes() == cid,
                "the service sent bytes that do not match the blob's id"
            );
            ensure!(got == size, "the blob is not the size the catalog says");
            // Open piece by piece into a second file, hashing the plaintext.
            use tokio::io::AsyncReadExt;
            let key = s.secret.blob_key(plain);
            let mut input = tokio::fs::File::open(&sealed_path).await?;
            let mut out = tokio::fs::File::create(&plain_path).await?;
            let mut ph = blake3::Hasher::new();
            let piece = CHUNK + TAG;
            let mut index = 0u64;
            let mut left = got;
            let mut buf = vec![0u8; piece];
            while left > 0 || index == 0 {
                let n = (left as usize).min(piece);
                input.read_exact(&mut buf[..n]).await?;
                left -= n as u64;
                let opened = key
                    .open_piece(index, left == 0, &buf[..n])
                    .map_err(|e| anyhow!("a piece does not open: {e}"))?;
                ph.update(&opened);
                out.write_all(&opened).await?;
                index += 1;
            }
            out.flush().await?;
            drop(out);
            ensure!(
                ph.finalize().as_bytes() == &plain.0,
                "the opened bytes do not match the blob's hash"
            );
            let added = self
                .node()
                .blobs()
                .add_path(
                    &plain_path,
                    crate::blobs::ImportMode::Copy,
                    crate::blobs::Protection::Library,
                )
                .await?;
            ensure!(added == *plain, "the store hashed it differently");
            Ok(())
        }
        .await;
        let _ = std::fs::remove_file(&sealed_path);
        let _ = std::fs::remove_file(&plain_path);
        result
    }
}
