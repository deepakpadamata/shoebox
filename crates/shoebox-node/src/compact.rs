//! Shard compaction (S1-15): keep the shard set small and free of dead rows.
//!
//! For each model and kind, one always-on indexer (rendezvous over the candidates, as for photos)
//! merges the live shards that are small (under [`MAX_PHOTOS`] rows) or carry more than 10% dead
//! rows (purged photos, and rows that lose to another shard's row for the same photo) into full
//! shards. Every new shard lists all the shards it merges in `replaces`, so they stop counting as
//! soon as it is fetched and checked. Two devices compacting at once (a disagreement about the
//! device list) leave duplicates, which the next round merges away.
//!
//! Label shards of an older label list are re-labelled from the photos' 8-bit vectors on the way,
//! without encoding any image again.

use crate::library_node::LibraryNode;
use anyhow::Result;
use shoebox_core::ids::Blake3Hash;
use shoebox_core::model::{ModelId, PhotoId, ShardKind};
use shoebox_core::policy::{index_owner, share_candidates};
use shoebox_core::store::{LibraryStore, Page, TimeRange};
use shoebox_search::labels::{LABELS_VERSION, MIN_PROBABILITY};
use shoebox_search::ocr::OCR_MARK;
use shoebox_search::shard::{Rows, Shard, TextRow, MAX_PHOTOS};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use tracing::debug;

/// A shard whose dead rows exceed this share is rewritten even when full.
const DEAD_SHARE: f64 = 0.10;

/// What one compaction round did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompactReport {
    /// Shards merged away.
    pub merged: usize,
    /// Shards written.
    pub written: usize,
    /// Rows dropped (purged photos, duplicates).
    pub dropped: usize,
    /// Rows re-labelled from vectors.
    pub relabelled: usize,
}

impl<L: LibraryStore> LibraryNode<L> {
    /// Compact the (model, kind) groups this device is responsible for. With `force`, every group
    /// is compacted whoever would normally do it (tests, `shoebox-keeper index compact`).
    pub async fn compact_shards(&self, force: bool) -> Result<CompactReport> {
        let me = self.device();
        let statuses = self.store().indexer_statuses().await?;
        let candidates = share_candidates(&statuses, |_| true);
        let view = self.shard_view().await?;
        let mut groups: BTreeMap<(ModelId, ShardKind, bool), Vec<Blake3Hash>> = BTreeMap::new();
        for h in &view.live {
            let r = &view.records[h];
            // Read-text shards and label shards share a kind but never merge with each other.
            let ocr = r.kind == ShardKind::Text && r.model == shoebox_search::ocr::OCR_ENGINE;
            groups.entry((r.model, r.kind, ocr)).or_default().push(*h);
        }
        let known: HashSet<PhotoId> = self
            .store()
            .photos_by_time(TimeRange::ALL, Page::all())
            .await?
            .into_iter()
            .chain(self.store().deleted_photos().await?)
            .map(|p| p.id)
            .collect();
        let mut report = CompactReport::default();
        for ((model, kind, _), hashes) in groups {
            if kind == ShardKind::Faces {
                continue; // S1-17/S1-18 compact faces with the people snapshot.
            }
            let key = Blake3Hash::of_bytes(&[&model.0[..], &[kind.byte()]].concat());
            if !force && index_owner(&key, kind, &candidates) != Some(me) {
                continue;
            }
            let mut shards = vec![];
            for h in &hashes {
                if let Some(s) = self.load_shard(&view.records[h]).await? {
                    shards.push((*h, s));
                }
            }
            shards.sort_by_key(|(h, _)| *h);
            let r = self.compact_group(model, kind, &shards, &known).await?;
            report.merged += r.merged;
            report.written += r.written;
            report.dropped += r.dropped;
            report.relabelled += r.relabelled;
        }
        if report.written > 0 {
            debug!(
                "compacted {} shards into {} ({} rows dropped)",
                report.merged, report.written, report.dropped
            );
        }
        Ok(report)
    }

    async fn compact_group(
        &self,
        model: ModelId,
        kind: ShardKind,
        shards: &[(Blake3Hash, Arc<Shard>)],
        known: &HashSet<PhotoId>,
    ) -> Result<CompactReport> {
        let winners = crate::shards::winning_rows(shards);
        // Old label shards are re-labelled; they are always rewritten.
        let stale_labels =
            |s: &Shard| kind == ShardKind::Text && s.dims != OCR_MARK && s.dims != LABELS_VERSION;
        let mut chosen: Vec<usize> = vec![];
        for (i, (_, s)) in shards.iter().enumerate() {
            let dead = s
                .ids
                .iter()
                .enumerate()
                .filter(|(row, id)| !known.contains(id) || winners.get(id) != Some(&(i, *row)))
                .count();
            let small = s.ids.len() < MAX_PHOTOS;
            if stale_labels(s) || dead as f64 > DEAD_SHARE * s.ids.len() as f64 || small {
                chosen.push(i);
            }
        }
        let small_only = chosen
            .iter()
            .all(|i| shards[*i].1.ids.len() < MAX_PHOTOS && !stale_labels(&shards[*i].1));
        let rows_in_chosen: usize = chosen.iter().map(|i| shards[*i].1.ids.len()).sum();
        // One small shard alone, with nothing dead and nothing to re-label, is left as it is.
        let dead_anywhere = chosen.iter().any(|i| {
            shards[*i]
                .1
                .ids
                .iter()
                .enumerate()
                .any(|(row, id)| !known.contains(id) || winners.get(id) != Some(&(*i, row)))
        });
        if chosen.is_empty() || (chosen.len() == 1 && small_only && !dead_anywhere) {
            return Ok(CompactReport::default());
        }
        // The rows that survive: the winning row of each known photo in the chosen shards.
        let mut ids = vec![];
        let mut picks: Vec<(usize, usize)> = vec![];
        for &i in &chosen {
            for (row, id) in shards[i].1.ids.iter().enumerate() {
                if known.contains(id) && winners.get(id) == Some(&(i, row)) {
                    ids.push(*id);
                    picks.push((i, row));
                }
            }
        }
        let dims = shards[chosen[0]].1.dims;
        let replaces: Vec<Blake3Hash> = chosen.iter().map(|i| shards[*i].0).collect();
        let mut report = CompactReport {
            merged: chosen.len(),
            dropped: rows_in_chosen - ids.len(),
            ..Default::default()
        };
        // Re-labelling needs each photo's vector of the same model.
        let vectors: HashMap<PhotoId, Vec<f32>> = if kind == ShardKind::Text
            && dims != OCR_MARK
            && chosen.iter().any(|i| stale_labels(&shards[*i].1))
        {
            let vec8 = self.live_shards_of(model, ShardKind::Vec8).await?;
            let w8 = crate::shards::winning_rows(&vec8);
            ids.iter()
                .filter_map(|id| {
                    let (i, r) = w8.get(id)?;
                    match &vec8[*i].1.rows {
                        Rows::Vec8(rows) => rows.get(*r).map(|v| (*id, v.to_f32())),
                        _ => None,
                    }
                })
                .collect()
        } else {
            HashMap::new()
        };
        let labels = self.labels();
        let relabel = labels.model() == Some(model);
        let mut out_dims = dims;
        let mut rows: Vec<RowOut> = Vec::with_capacity(picks.len());
        for ((i, row), id) in picks.iter().zip(&ids) {
            let s = &shards[*i].1;
            rows.push(match &s.rows {
                Rows::Vec8(r) => RowOut::Vec8(r[*row].clone()),
                Rows::Vec1(r) => RowOut::Vec1(r[*row].clone()),
                Rows::Text(r) => {
                    let mut t = r[*row].clone();
                    if stale_labels(s) {
                        t.labels = match vectors.get(id) {
                            Some(v) if relabel => {
                                report.relabelled += 1;
                                labels.assign(v, MIN_PROBABILITY)
                            }
                            _ => vec![],
                        };
                    }
                    RowOut::Text(t)
                }
                Rows::Faces(r) => RowOut::Faces(r[*row].clone()),
            });
        }
        if kind == ShardKind::Text && dims != OCR_MARK {
            out_dims = LABELS_VERSION;
        }
        for chunk in (0..ids.len()).collect::<Vec<_>>().chunks(MAX_PHOTOS) {
            let cids: Vec<PhotoId> = chunk.iter().map(|&k| ids[k]).collect();
            let crow: Vec<&RowOut> = chunk.iter().map(|&k| &rows[k]).collect();
            let rows = match kind {
                ShardKind::Vec8 => Rows::Vec8(crow.iter().filter_map(|r| r.vec8()).collect()),
                ShardKind::Vec1 => Rows::Vec1(crow.iter().filter_map(|r| r.vec1()).collect()),
                ShardKind::Text => Rows::Text(crow.iter().filter_map(|r| r.text()).collect()),
                ShardKind::Faces => Rows::Faces(crow.iter().filter_map(|r| r.faces()).collect()),
            };
            let shard = Shard::build(model, out_dims, replaces.clone(), rows, cids)?;
            self.publish_shard(&shard).await?;
            report.written += 1;
        }
        if ids.is_empty() {
            // Everything in them is gone: an empty replacement cannot be written (a shard holds
            // at least one photo), so they stay until the photos' facts drop them at query time.
            report.merged = 0;
        }
        Ok(report)
    }
}

enum RowOut {
    Vec8(shoebox_search::shard::Vec8),
    Vec1(Vec<u8>),
    Text(TextRow),
    Faces(Vec<shoebox_search::shard::Face>),
}

impl RowOut {
    fn vec8(&self) -> Option<shoebox_search::shard::Vec8> {
        match self {
            RowOut::Vec8(v) => Some(v.clone()),
            _ => None,
        }
    }
    fn vec1(&self) -> Option<Vec<u8>> {
        match self {
            RowOut::Vec1(v) => Some(v.clone()),
            _ => None,
        }
    }
    fn text(&self) -> Option<TextRow> {
        match self {
            RowOut::Text(v) => Some(v.clone()),
            _ => None,
        }
    }
    fn faces(&self) -> Option<Vec<shoebox_search::shard::Face>> {
        match self {
            RowOut::Faces(v) => Some(v.clone()),
            _ => None,
        }
    }
}
