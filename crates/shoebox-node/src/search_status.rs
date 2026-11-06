//! How much of the library each kind of search covers on this device, and who is working on
//! the rest (S1-16): "connectivity is honest" applied to search.

use crate::library_node::LibraryNode;
use anyhow::Result;
use shoebox_core::ids::EndpointId;
use shoebox_core::model::{PhotoId, ShardKind};
use shoebox_core::store::{LibraryStore, Page, TimeRange};
use shoebox_search::labels::LABELS_VERSION;
use shoebox_search::ocr::{wants_text, OCR_ENGINE, OCR_MARK};
use shoebox_search::shard::Rows;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexerInfo {
    pub device: EndpointId,
    pub photos_per_min: u32,
    pub always_on: bool,
    /// Reads text in photos.
    pub reads_text: bool,
    /// Looks for faces in photos.
    pub finds_faces: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchStatus {
    /// Photos in the library (not deleted).
    pub total: usize,
    /// Searchable by what is in them on this device (a picture vector of the search model).
    pub content: usize,
    /// With labels.
    pub labelled: usize,
    /// Read for text (whether or not any was found).
    pub text_read: usize,
    /// Searched for faces (S1-17).
    pub faces: usize,
    /// Photos likely to hold text (screenshots, and photos labelled as documents, signs and the
    /// like): the ones text is read in. `text_read` counts every photo read, wanted or not.
    pub text_wanted: usize,
    /// Of those, the ones already read.
    pub text_wanted_read: usize,
    /// Photos faces are looked for in (not videos).
    pub faces_wanted: usize,
    /// People is on for the library.
    pub people_on: bool,
    /// This device has a text model to search by meaning with.
    pub meaning: bool,
    /// Devices that index, fastest first.
    pub indexers: Vec<IndexerInfo>,
    /// Time until every photo has a picture vector, at the indexers' rates (`None`: done, or
    /// nobody indexing).
    pub eta_secs: Option<u64>,
    /// Search data held on this device: shards and the local index files.
    pub held_bytes: u64,
}

impl<L: LibraryStore> LibraryNode<L> {
    pub async fn search_status(&self) -> Result<SearchStatus> {
        let photos: HashSet<PhotoId> = self
            .store()
            .photos_by_time(TimeRange::ALL, Page::all())
            .await?
            .into_iter()
            .map(|p| p.id)
            .collect();
        let view = self.shard_view().await?;
        let model = *self.inner.shards.model.lock().unwrap();
        let (mut content, mut labelled, mut read, mut faces) = (
            HashSet::new(),
            HashSet::new(),
            HashSet::new(),
            HashSet::new(),
        );
        let mut label_rows: HashMap<PhotoId, Vec<(u16, u8)>> = HashMap::new();
        let mut held_bytes = 0u64;
        for h in &view.live {
            let r = &view.records[h];
            held_bytes += r.blob.size;
            let Some(s) = self.load_shard(r).await? else {
                continue;
            };
            let ids = s.ids.iter().filter(|id| photos.contains(id)).copied();
            match r.kind {
                ShardKind::Vec8 | ShardKind::Vec1 if model.is_none_or(|m| m == r.model) => {
                    content.extend(ids)
                }
                ShardKind::Text if s.dims == OCR_MARK => read.extend(ids),
                ShardKind::Text => {
                    if let Rows::Text(rows) = &s.rows {
                        if s.dims == LABELS_VERSION {
                            for (id, row) in s.ids.iter().zip(rows) {
                                label_rows
                                    .entry(*id)
                                    .or_default()
                                    .extend(row.labels.iter().copied());
                            }
                        }
                        labelled.extend(
                            s.ids
                                .iter()
                                .zip(rows)
                                .filter(|(id, row)| !row.labels.is_empty() && photos.contains(id))
                                .map(|(id, _)| *id),
                        );
                    }
                }
                ShardKind::Faces => faces.extend(ids),
                _ => {}
            }
        }
        if let Ok(dir) = std::fs::read_dir(self.node().config().data_dir.join("search")) {
            held_bytes += dir
                .flatten()
                .filter_map(|e| e.metadata().ok())
                .map(|m| m.len())
                .sum::<u64>();
        }
        let mut indexers: Vec<IndexerInfo> = self
            .store()
            .indexer_statuses()
            .await?
            .into_iter()
            .filter(|s| s.photos_per_min > 0)
            .map(|s| IndexerInfo {
                device: s.device,
                photos_per_min: s.photos_per_min,
                always_on: s.always_on,
                reads_text: s.models.contains(&OCR_ENGINE),
                finds_faces: s.kinds.contains(&ShardKind::Faces),
            })
            .collect();
        indexers.sort_by_key(|i| std::cmp::Reverse(i.photos_per_min));
        let left = photos.len().saturating_sub(content.len()) as u64;
        let rate: u64 = indexers.iter().map(|i| u64::from(i.photos_per_min)).sum();
        let meaning = self.meaning_configured();
        let labels = self.labels();
        let (mut text_wanted, mut text_wanted_read, mut faces_wanted) = (0, 0, 0);
        for p in self
            .store()
            .photos_by_time(TimeRange::ALL, Page::all())
            .await?
        {
            if !p.original.mime.starts_with("video/") {
                faces_wanted += 1;
            }
            let screenshot = shoebox_search::facts::PhotoFacts::of(&p).is_screenshot();
            let photo_labels = label_rows.get(&p.id).map(Vec::as_slice).unwrap_or(&[]);
            if wants_text(&labels, photo_labels, screenshot) {
                text_wanted += 1;
                text_wanted_read += read.contains(&p.id) as usize;
            }
        }
        let people_on = self.people_on().await?;
        Ok(SearchStatus {
            total: photos.len(),
            content: content.len(),
            labelled: labelled.len(),
            text_read: read.len(),
            faces: faces.len(),
            text_wanted,
            text_wanted_read,
            faces_wanted,
            people_on,
            meaning,
            eta_secs: (left > 0 && rate > 0).then(|| left * 60 / rate),
            indexers,
            held_bytes,
        })
    }
}
