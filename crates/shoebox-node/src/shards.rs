//! Index shards on this device (S1-07): publishing the ones it makes, knowing which ones count,
//! and what to fetch and keep. Shard blobs are protected as search data (`idx/<hash>`), never
//! freed up, and released once a valid shard replaces them.

use crate::blobs::Protection;
use crate::library_node::LibraryNode;
use anyhow::{bail, Result};
use shoebox_core::ids::Blake3Hash;
use shoebox_core::model::{BlobKind, BlobRef, IndexShard, PhotoId};
use shoebox_core::policy::{index_plan, FetchItem};
use shoebox_core::store::LibraryStore;
use shoebox_search::shard::{live_shards, Fetched, Shard};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use tracing::warn;

/// The media type recorded for shard blobs.
pub const SHARD_MIME: &str = "application/x-shoebox-index";

/// What this device learned from the shard blobs it holds: their photo ids, or that they are
/// bad. A shard is immutable, so this never goes stale. Rows are not kept here (a phone could
/// not afford every shard decoded at once): readers decode a shard when they need it.
#[derive(Default)]
pub(crate) struct ShardBook {
    known: Mutex<HashMap<Blake3Hash, Known>>,
    /// The image–text model this build searches with (S1-15: other models' shards are let go
    /// once it covers as many photos).
    pub(crate) model: Mutex<Option<shoebox_core::model::ModelId>>,
    /// The face model people are worked out with (default: this build's).
    pub(crate) face_model: Mutex<Option<shoebox_core::model::ModelId>>,
}

#[derive(Clone)]
enum Known {
    Good(Arc<Vec<PhotoId>>),
    Bad,
}

/// Which shards count on this device right now.
#[derive(Clone, Debug, Default)]
pub struct ShardView {
    /// Every record, by blob.
    pub records: BTreeMap<Blake3Hash, IndexShard>,
    /// Shards to search: live, held and valid.
    pub live: BTreeSet<Blake3Hash>,
    /// Shards to fetch and keep: live once the ones not fetched yet turn out valid.
    pub wanted: BTreeSet<Blake3Hash>,
    /// Fetched blobs that did not match their records (ignored, never deleted from the library).
    pub bad: BTreeSet<Blake3Hash>,
}

impl<L: LibraryStore> LibraryNode<L> {
    /// Store `shard` as a blob, record it in the library and publish this device's holding.
    pub async fn publish_shard(&self, shard: &Shard) -> Result<IndexShard> {
        if shard.ids.is_empty() {
            bail!("an index shard needs at least one photo");
        }
        let bytes = shard.encode();
        let size = bytes.len() as u64;
        let hash = self
            .node()
            .blobs()
            .add_bytes(bytes, Protection::Index)
            .await?;
        let record = IndexShard {
            blob: BlobRef {
                hash,
                size,
                mime: SHARD_MIME.into(),
            },
            model: shard.model,
            kind: shard.kind(),
            count: shard.ids.len() as u32,
            replaces: shard.replaces.clone(),
            made_by: self.device(),
            hlc: shoebox_core::hlc::Hlc::GENESIS,
        };
        self.inner
            .shards
            .known
            .lock()
            .unwrap()
            .insert(hash, Known::Good(Arc::new(shard.ids.clone())));
        self.store().put_index_shard(record.clone()).await?;
        self.publish_holdings(vec![(hash, BlobKind::Index)]).await?;
        Ok(record)
    }

    /// The decoded shard if this device holds a valid copy of it.
    pub async fn load_shard(&self, record: &IndexShard) -> Result<Option<Arc<Shard>>> {
        let hash = record.blob.hash;
        if self.is_bad_shard(&hash) || !self.node().blobs().has(&hash).await? {
            return Ok(None);
        }
        let bytes = self.node().blobs().get(&hash).await?;
        let (known, shard) = match Shard::decode(&bytes) {
            Ok(s) if s.matches(record) => (Known::Good(Arc::new(s.ids.clone())), Some(Arc::new(s))),
            Ok(_) => {
                warn!("index shard {hash} does not match its record; ignoring it");
                (Known::Bad, None)
            }
            Err(e) => {
                warn!("index shard {hash} is unreadable ({e}); ignoring it");
                (Known::Bad, None)
            }
        };
        self.inner.shards.known.lock().unwrap().insert(hash, known);
        Ok(shard)
    }

    /// Photo ids of a held, valid shard (decoding it once).
    async fn shard_ids(&self, record: &IndexShard) -> Result<Option<Arc<Vec<PhotoId>>>> {
        let known = self
            .inner
            .shards
            .known
            .lock()
            .unwrap()
            .get(&record.blob.hash)
            .cloned();
        match known {
            // Released since: no longer held.
            Some(Known::Good(_)) if !self.node().blobs().kept(&record.blob.hash).await? => Ok(None),
            Some(Known::Good(ids)) => Ok(Some(ids)),
            Some(Known::Bad) => Ok(None),
            None => Ok(self
                .load_shard(record)
                .await?
                .map(|s| Arc::new(s.ids.clone()))),
        }
    }

    /// Which shards count, which to fetch and which are bad, from the library's records and the
    /// blobs this device holds (epic S1 §3.2).
    pub async fn shard_view(&self) -> Result<ShardView> {
        let records: BTreeMap<Blake3Hash, IndexShard> = self
            .store()
            .index_shards()
            .await?
            .into_iter()
            .map(|r| (r.blob.hash, r))
            .collect();
        let mut fetched: HashMap<Blake3Hash, Fetched> = HashMap::new();
        for (hash, r) in &records {
            let state = match self.shard_ids(r).await? {
                Some(ids) => Fetched::Valid(ids.to_vec()),
                None if self.is_bad_shard(hash) => Fetched::Invalid,
                None => Fetched::Unknown,
            };
            fetched.insert(*hash, state);
        }
        let live = live_shards(records.values(), |h| {
            fetched.get(h).cloned().unwrap_or(Fetched::Unknown)
        });
        // To plan, assume what is not fetched yet is valid: then a shard already replaced by
        // one still on its way is not fetched for nothing.
        let wanted = live_shards(records.values(), |h| match fetched.get(h) {
            Some(Fetched::Unknown) | None => Fetched::Assumed,
            Some(f) => f.clone(),
        });
        let bad = fetched
            .iter()
            .filter(|(_, f)| **f == Fetched::Invalid)
            .map(|(h, _)| *h)
            .collect();
        let held = |h: &Blake3Hash| matches!(fetched.get(h), Some(Fetched::Valid(_)));
        let mut wanted: BTreeSet<Blake3Hash> = wanted.union(&live).copied().collect();
        // A model upgrade (S1-15): this build searches with its own model only. Another model's
        // shards are kept until this one covers at least as many photos, then let go.
        let current = *self.inner.shards.model.lock().unwrap();
        if let Some(current) = current {
            let mut covered: HashMap<shoebox_core::model::ModelId, usize> = HashMap::new();
            for h in &live {
                let r = &records[h];
                if r.kind == shoebox_core::model::ShardKind::Vec8 {
                    *covered.entry(r.model).or_default() += r.count as usize;
                }
            }
            let mine = covered.get(&current).copied().unwrap_or(0);
            let retired: BTreeSet<shoebox_core::model::ModelId> = covered
                .iter()
                .filter(|(m, n)| **m != current && mine > 0 && mine >= **n)
                .map(|(m, _)| *m)
                .collect();
            wanted.retain(|h| !retired.contains(&records[h].model));
        }
        Ok(ShardView {
            wanted,
            live: live.into_iter().filter(held).collect(),
            records,
            bad,
        })
    }

    fn is_bad_shard(&self, hash: &Blake3Hash) -> bool {
        matches!(
            self.inner.shards.known.lock().unwrap().get(hash),
            Some(Known::Bad)
        )
    }

    /// Shards this device should fetch (not held yet), in plan order.
    pub(crate) async fn shard_fetch_plan(&self, library_size: usize) -> Result<Vec<FetchItem>> {
        let view = self.shard_view().await?;
        let role = self.node().config().role;
        let mut plan = vec![];
        for item in index_plan(
            role,
            library_size,
            view.wanted.iter().filter_map(|h| view.records.get(h)),
        ) {
            if !self.node().blobs().has(&item.blob.hash).await? {
                plan.push(item);
            }
        }
        // The people grouping snapshot, held by every device (S1-18).
        if let Some(v) = self.store().setting(PEOPLE_SNAPSHOT_KEY).await? {
            if let Ok(blob) = postcard::from_bytes::<BlobRef>(&v) {
                if !self.node().blobs().has(&blob.hash).await? {
                    plan.push(FetchItem {
                        kind: BlobKind::Index,
                        blob,
                    });
                }
            }
        }
        Ok(plan)
    }

    /// Search data this device keeps: wanted shards, the people snapshot and this build's
    /// model files.
    pub(crate) async fn index_referenced(&self) -> Result<BTreeSet<Blake3Hash>> {
        let view = self.shard_view().await?;
        let mut keep = view.wanted;
        keep.extend(self.people_snapshot_blob().await?);
        keep.extend(
            shoebox_search::models::ALL
                .iter()
                .map(|f| Blake3Hash(f.blake3)),
        );
        Ok(keep)
    }

    /// Live shards of one model and kind, decoded, in blob hash order (the order in which they
    /// win a photo, see [`winning_rows`]).
    pub async fn live_shards_of(
        &self,
        model: shoebox_core::model::ModelId,
        kind: shoebox_core::model::ShardKind,
    ) -> Result<Vec<(Blake3Hash, Arc<Shard>)>> {
        let view = self.shard_view().await?;
        let mut out = vec![];
        for h in &view.live {
            let r = &view.records[h];
            if r.model == model && r.kind == kind {
                if let Some(s) = self.load_shard(r).await? {
                    out.push((*h, s));
                }
            }
        }
        Ok(out)
    }

    /// The image–text model this build searches with: shards of an older model are let go once
    /// this one covers as many photos (S1-15).
    pub fn set_search_model(&self, model: shoebox_core::model::ModelId) {
        *self.inner.shards.model.lock().unwrap() = Some(model);
    }

    /// The face model whose shards people are worked out from (S1-18; default: this build's).
    pub fn set_face_model(&self, model: shoebox_core::model::ModelId) {
        *self.inner.shards.face_model.lock().unwrap() = Some(model);
    }

    pub fn face_model(&self) -> shoebox_core::model::ModelId {
        self.inner
            .shards
            .face_model
            .lock()
            .unwrap()
            .unwrap_or(shoebox_search::models::FACES_MODEL)
    }

    /// The blob of the newest people grouping snapshot (S1-18), if any.
    pub async fn people_snapshot_blob(&self) -> Result<Option<Blake3Hash>> {
        Ok(self
            .store()
            .setting(PEOPLE_SNAPSHOT_KEY)
            .await?
            .and_then(|v| postcard::from_bytes::<BlobRef>(&v).ok())
            .map(|b| b.hash))
    }
}

/// Library setting naming the people grouping snapshot's blob (postcard `BlobRef`).
pub const PEOPLE_SNAPSHOT_KEY: &str = "people.snapshot";

/// For each photo, where its row is: `(index in shards, row)`, from the shard with the lowest
/// blob hash that holds it. `shards` must be in blob hash order.
pub fn winning_rows(shards: &[(Blake3Hash, Arc<Shard>)]) -> BTreeMap<PhotoId, (usize, usize)> {
    let mut out = BTreeMap::new();
    for (i, (_, s)) in shards.iter().enumerate() {
        for (row, id) in s.ids.iter().enumerate() {
            out.entry(*id).or_insert((i, row));
        }
    }
    out
}
