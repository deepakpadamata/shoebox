//! People on this device (S1-18): grouping faces on the one device whose turn it is, and
//! working out who is in each photo on every device, from face shards, people's labels and
//! the newest grouping snapshot.

use crate::blobs::Protection;
use crate::library_node::LibraryNode;
use crate::shards::PEOPLE_SNAPSHOT_KEY;
use anyhow::Result;
use shoebox_core::ids::{Blake3Hash, EndpointId};
use shoebox_core::model::{BlobKind, BlobRef, ModelId, Person, PersonId, PhotoId, ShardKind};
use shoebox_core::policy::share_candidates;
use shoebox_core::store::{LibraryStore, Page, TimeRange};
use shoebox_search::faces::FaceKey;
use shoebox_search::people::{group, resolve, FaceIn, Snapshot, Who};
use shoebox_search::shard::Rows;
use std::collections::{BTreeMap, HashSet};
use tracing::debug;

/// Library setting: the person who is "me" (16 bytes, S1-18).
pub const ME_SETTING: &str = "people.me";
/// Regroup once there are this many times as many faces as in the last snapshot.
const REGROUP_GROWTH: f64 = 1.05;

/// Who is in the library's photos, as this device works it out.
#[derive(Clone, Debug, Default)]
pub struct PeopleView {
    /// Every face with who it is.
    pub faces: Vec<(FaceIn, Who)>,
    /// Person records, by id.
    pub persons: BTreeMap<PersonId, Person>,
    pub me: Option<PersonId>,
}

impl PeopleView {
    /// People (followed through merges) in a photo.
    pub fn people_in(&self, photo: &PhotoId) -> Vec<PersonId> {
        let mut out: Vec<PersonId> = self
            .faces
            .iter()
            .filter(|(f, _)| f.photo == *photo)
            .filter_map(|(_, w)| match w {
                Who::Person(p) => Some(*p),
                _ => None,
            })
            .collect();
        out.sort();
        out.dedup();
        out
    }
}

impl<L: LibraryStore> LibraryNode<L> {
    /// Every face of the live face shards of `model` (one row per photo: the winning shard's).
    pub async fn face_inputs(&self, model: ModelId) -> Result<Vec<FaceIn>> {
        let shards = self.live_shards_of(model, ShardKind::Faces).await?;
        let rows = crate::shards::winning_rows(&shards);
        let alive: HashSet<PhotoId> = self
            .store()
            .photos_by_time(TimeRange::ALL, Page::all())
            .await?
            .into_iter()
            .map(|p| p.id)
            .collect();
        let mut out = vec![];
        for (photo, (i, r)) in rows {
            if !alive.contains(&photo) {
                continue;
            }
            let Rows::Faces(faces) = &shards[i].1.rows else {
                continue;
            };
            for f in &faces[r] {
                out.push(FaceIn {
                    photo,
                    rect: f.bbox.map(|v| f32::from(v) / 65_535.0),
                    quality: f.quality,
                    embedding: f.embedding.to_f32(),
                });
            }
        }
        Ok(out)
    }

    async fn snapshot(&self) -> Result<Option<Snapshot>> {
        let Some(hash) = self.people_snapshot_blob().await? else {
            return Ok(None);
        };
        if !self.node().blobs().has(&hash).await? {
            return Ok(None);
        }
        Ok(Snapshot::decode(&self.node().blobs().get(&hash).await?))
    }

    /// The device that groups faces: the indexer with the lowest device id among those that find
    /// faces with `model` (the same answer on every device). Always-on indexers come first; with
    /// none, a phone on its own does it ([`share_candidates`]).
    pub async fn grouping_device(&self, model: ModelId) -> Result<Option<EndpointId>> {
        let statuses = self.store().indexer_statuses().await?;
        Ok(share_candidates(&statuses, |s| s.models.contains(&model))
            .into_iter()
            .map(|c| c.device)
            .min())
    }

    /// Group the faces again if this device is the one that does it and the faces grew by 5%
    /// since the last snapshot (or `force`). Returns whether a snapshot was published.
    pub async fn regroup_people(&self, model: ModelId, force: bool) -> Result<bool> {
        if !self.people_on().await? {
            return Ok(false);
        }
        if !force && self.grouping_device(model).await? != Some(self.device()) {
            return Ok(false);
        }
        let faces = self.face_inputs(model).await?;
        let old = self.snapshot().await?;
        let before = old.as_ref().map_or(0, |s| s.faces.len());
        if faces.is_empty()
            || (!force && before > 0 && (faces.len() as f64) < before as f64 * REGROUP_GROWTH)
        {
            return Ok(false);
        }
        let labels = self.store().face_labels().await?;
        let persons: BTreeMap<PersonId, Person> = self
            .store()
            .persons()
            .await?
            .into_iter()
            .map(|p| (p.id, p))
            .collect();
        let snap =
            tokio::task::spawn_blocking(move || group(&faces, &labels, &persons, Some(model)))
                .await?;
        let bytes = snap.encode();
        let size = bytes.len() as u64;
        let hash = self
            .node()
            .blobs()
            .add_bytes(bytes, Protection::Index)
            .await?;
        let blob = BlobRef {
            hash,
            size,
            mime: "application/x-shoebox-people".into(),
        };
        self.publish_holdings(vec![(hash, BlobKind::Index)]).await?;
        self.store()
            .put_setting(PEOPLE_SNAPSHOT_KEY, postcard::to_stdvec(&blob)?)
            .await?;
        debug!("grouped {} faces", snap.faces.len());
        Ok(true)
    }

    /// Who is in the photos (epic S1 §8.3), from this device's face shards of `model`.
    pub async fn people_view(&self, model: ModelId) -> Result<PeopleView> {
        let faces = self.face_inputs(model).await?;
        let labels = self.store().face_labels().await?;
        let persons: BTreeMap<PersonId, Person> = self
            .store()
            .persons()
            .await?
            .into_iter()
            .map(|p| (p.id, p))
            .collect();
        let snap = self.snapshot().await?;
        let me = self
            .store()
            .setting(ME_SETTING)
            .await?
            .and_then(|v| <[u8; 16]>::try_from(v.as_slice()).ok())
            .map(PersonId);
        let who = resolve(&faces, &labels, &persons, snap.as_ref());
        Ok(PeopleView {
            faces: faces.into_iter().zip(who).collect(),
            persons,
            me,
        })
    }

    /// Names (and "me") per photo for the facts index, so people are found by name.
    pub(crate) async fn people_names(&self, model: ModelId) -> Result<Vec<(PhotoId, String)>> {
        if !self.people_on().await? {
            return Ok(vec![]);
        }
        let view = self.people_view(model).await?;
        let me = view
            .me
            .map(|m| shoebox_search::people::canonical(m, &view.persons));
        let mut by_photo: BTreeMap<PhotoId, Vec<String>> = BTreeMap::new();
        for (f, w) in &view.faces {
            let Who::Person(p) = w else { continue };
            let Some(person) = view.persons.get(p) else {
                continue;
            };
            if person.hidden.value || person.name.value.trim().is_empty() {
                continue;
            }
            let names = by_photo.entry(f.photo).or_default();
            names.push(person.name.value.clone());
            if Some(*p) == me {
                names.push("me".into());
            }
        }
        Ok(by_photo
            .into_iter()
            .map(|(p, mut n)| {
                n.sort();
                n.dedup();
                (p, n.join("\n"))
            })
            .collect())
    }

    /// The face key of a face, for labels.
    pub fn face_key(rect: [f32; 4]) -> FaceKey {
        FaceIn {
            photo: Blake3Hash([0; 32]),
            rect,
            quality: 0,
            embedding: vec![],
        }
        .key()
    }
}
