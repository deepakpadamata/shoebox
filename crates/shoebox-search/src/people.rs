//! People (S1-18): faces grouped by one device, named and corrected by people, and worked out
//! on every device the same way (epic S1 §8).
//!
//! - [`group`]: clusters face embeddings (greedy leader clustering on cosine similarity, best
//!   faces first), keeping faces people labelled as one person together. One device runs it and
//!   publishes the result as a [`Snapshot`] blob.
//! - [`resolve`]: on every device, who each face is: an explicit label, else its group's name
//!   (the person most of the group's labelled faces belong to), else the nearest named person,
//!   else unknown. "Not Ana" always wins over all three.

use crate::faces::{iou, FaceKey};
use crate::shard::Vec8;
use serde::{Deserialize, Serialize};
use shoebox_core::model::{FaceLabel, ModelId, Person, PersonId, PhotoId};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Faces at least this similar to a group's centroid join it.
pub const GROUP_SIMILARITY: f32 = 0.45;
/// A face this close to a named person's centroid is taken to be them (rule 3).
pub const NEAREST_SIMILARITY: f32 = 0.5;
/// Unnamed groups are shown once they hold this many faces.
pub const MIN_GROUP: usize = 5;
pub const SNAPSHOT_VERSION: u8 = 1;

/// One face as the indexer recorded it.
#[derive(Clone, Debug, PartialEq)]
pub struct FaceIn {
    pub photo: PhotoId,
    /// Box as fractions of the photo.
    pub rect: [f32; 4],
    pub quality: u8,
    pub embedding: Vec<f32>,
}

impl FaceIn {
    pub fn key(&self) -> FaceKey {
        FaceKey::of(
            self.rect
                .map(|v| (v.clamp(0.0, 1.0) * 65_535.0).round() as u16),
        )
    }
}

/// The grouping result (`settings/people.snapshot` → blob).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub version: u8,
    /// The face model the embeddings came from.
    pub model: Option<ModelId>,
    /// Faces grouped, `(photo, face key, group)`, sorted.
    pub faces: Vec<(PhotoId, FaceKey, u32)>,
    /// Each group's centroid, by group.
    pub centroids: Vec<(u32, Vec8)>,
}

impl Snapshot {
    pub fn encode(&self) -> Vec<u8> {
        postcard::to_stdvec(self).expect("encodes")
    }

    pub fn decode(bytes: &[u8]) -> Option<Snapshot> {
        postcard::from_bytes::<Snapshot>(bytes)
            .ok()
            .filter(|s| s.version == SNAPSHOT_VERSION)
    }

    /// The group of a face found in `photo` at `rect` (by overlap with the grouped box).
    pub fn group_of(&self, photo: &PhotoId, rect: [f32; 4]) -> Option<u32> {
        let start = self.faces.partition_point(|(p, _, _)| p < photo);
        self.faces[start..]
            .iter()
            .take_while(|(p, _, _)| p == photo)
            .map(|(_, k, g)| (iou(k.rect(), rect), *g))
            .filter(|(o, _)| *o > 0.5)
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, g)| g)
    }
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn normalized(mut v: Vec<f32>) -> Vec<f32> {
    crate::encoder::normalize(&mut v);
    v
}

/// The end of a person's `merged_into` chain. Within a cycle, the lowest id is the end, so
/// every device agrees whatever order the merges arrived in.
pub fn canonical(id: PersonId, persons: &BTreeMap<PersonId, Person>) -> PersonId {
    let mut seen = vec![id];
    let mut at = id;
    while let Some(next) = persons.get(&at).and_then(|p| p.merged_into.value) {
        if let Some(i) = seen.iter().position(|s| *s == next) {
            return *seen[i..].iter().min().expect("non-empty");
        }
        seen.push(next);
        at = next;
    }
    at
}

/// The label people gave the face at `rect` in its photo, by overlap with the label's key.
fn label_for<'a>(
    labels: &'a HashMap<PhotoId, Vec<&'a FaceLabel>>,
    photo: &PhotoId,
    rect: [f32; 4],
) -> Option<&'a FaceLabel> {
    labels
        .get(photo)?
        .iter()
        .map(|l| (iou(FaceKey(l.face).rect(), rect), *l))
        .filter(|(o, _)| *o > 0.5)
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, l)| l)
}

fn by_photo(labels: &[FaceLabel]) -> HashMap<PhotoId, Vec<&FaceLabel>> {
    let mut m: HashMap<PhotoId, Vec<&FaceLabel>> = HashMap::new();
    for l in labels {
        m.entry(l.photo).or_default().push(l);
    }
    m
}

/// Group faces: best quality first, each joins the most similar group above
/// [`GROUP_SIMILARITY`] or starts one. Faces labelled as the same person are put together
/// first, and a face is never put into a group holding a face labelled as someone it is not.
pub fn group(
    faces: &[FaceIn],
    labels: &[FaceLabel],
    persons: &BTreeMap<PersonId, Person>,
    model: Option<ModelId>,
) -> Snapshot {
    let labels = by_photo(labels);
    let label = |f: &FaceIn| label_for(&labels, &f.photo, f.rect);
    let mut order: Vec<usize> = (0..faces.len()).collect();
    // Labelled faces seed the groups; then the best faces; ties broken by photo and position.
    order.sort_by(|&a, &b| {
        let la = label(&faces[a]).and_then(|l| l.is.value).is_some();
        let lb = label(&faces[b]).and_then(|l| l.is.value).is_some();
        lb.cmp(&la)
            .then(faces[b].quality.cmp(&faces[a].quality))
            .then(faces[a].photo.cmp(&faces[b].photo))
            .then(faces[a].key().cmp(&faces[b].key()))
    });
    struct Group {
        sum: Vec<f32>,
        centroid: Vec<f32>,
        people: BTreeSet<PersonId>,
    }
    let mut groups: Vec<Group> = vec![];
    let mut person_group: HashMap<PersonId, usize> = HashMap::new();
    let mut assigned: Vec<(usize, usize)> = Vec::with_capacity(faces.len());
    for &i in &order {
        let f = &faces[i];
        let l = label(f);
        let is = l
            .and_then(|l| l.is.value)
            .map(|p| canonical(p, persons))
            .filter(|p| !l.is_some_and(|l| l.is_not.contains(p)));
        let not: BTreeSet<PersonId> = l
            .map(|l| {
                l.is_not
                    .items()
                    .into_iter()
                    .map(|p| canonical(p, persons))
                    .collect()
            })
            .unwrap_or_default();
        let target = match is.and_then(|p| person_group.get(&p).copied()) {
            Some(g) => Some(g),
            None => groups
                .iter()
                .enumerate()
                .filter(|(_, g)| g.people.is_disjoint(&not))
                .filter(|(_, g)| is.is_none_or(|p| g.people.is_empty() || g.people.contains(&p)))
                .map(|(gi, g)| (gi, dot(&g.centroid, &f.embedding)))
                .filter(|(_, s)| *s >= GROUP_SIMILARITY)
                .max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0)))
                .map(|(gi, _)| gi),
        };
        let gi = match target {
            Some(gi) => {
                let g = &mut groups[gi];
                for (s, v) in g.sum.iter_mut().zip(&f.embedding) {
                    *s += v;
                }
                g.centroid = normalized(g.sum.clone());
                gi
            }
            None => {
                groups.push(Group {
                    sum: f.embedding.clone(),
                    centroid: normalized(f.embedding.clone()),
                    people: BTreeSet::new(),
                });
                groups.len() - 1
            }
        };
        if let Some(p) = is {
            groups[gi].people.insert(p);
            person_group.entry(p).or_insert(gi);
        }
        assigned.push((i, gi));
    }
    let mut out: Vec<(PhotoId, FaceKey, u32)> = assigned
        .into_iter()
        .map(|(i, g)| (faces[i].photo, faces[i].key(), g as u32))
        .collect();
    out.sort();
    out.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1);
    Snapshot {
        version: SNAPSHOT_VERSION,
        model,
        faces: out,
        centroids: groups
            .iter()
            .enumerate()
            .map(|(i, g)| (i as u32, Vec8::quantize(&g.centroid)))
            .collect(),
    }
}

/// Who a face is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Who {
    Person(PersonId),
    /// An unnamed group of the snapshot.
    Group(u32),
    Unknown,
}

/// Who every face is (epic S1 §8.3), the same on every device for the same records, snapshot
/// and faces, whatever order they arrived in.
pub fn resolve(
    faces: &[FaceIn],
    labels: &[FaceLabel],
    persons: &BTreeMap<PersonId, Person>,
    snapshot: Option<&Snapshot>,
) -> Vec<Who> {
    let labels_by = by_photo(labels);
    let label = |f: &FaceIn| label_for(&labels_by, &f.photo, f.rect);
    let not_of = |f: &FaceIn| -> BTreeSet<PersonId> {
        label(f)
            .map(|l| {
                l.is_not
                    .items()
                    .into_iter()
                    .map(|p| canonical(p, persons))
                    .collect()
            })
            .unwrap_or_default()
    };
    let explicit = |f: &FaceIn| -> Option<PersonId> {
        let l = label(f)?;
        let p = canonical(l.is.value?, persons);
        (!not_of(f).contains(&p)).then_some(p)
    };
    let group_of = |f: &FaceIn| snapshot.and_then(|s| s.group_of(&f.photo, f.rect));
    // Each group's name: the person most of its labelled faces are (ties: the lowest id).
    let mut votes: BTreeMap<u32, BTreeMap<PersonId, usize>> = BTreeMap::new();
    // Each named person's centroid, from their labelled faces.
    let mut sums: BTreeMap<PersonId, Vec<f32>> = BTreeMap::new();
    for f in faces {
        if let Some(p) = explicit(f) {
            if let Some(g) = group_of(f) {
                *votes.entry(g).or_default().entry(p).or_default() += 1;
            }
            let s = sums
                .entry(p)
                .or_insert_with(|| vec![0.0; f.embedding.len()]);
            for (a, b) in s.iter_mut().zip(&f.embedding) {
                *a += b;
            }
        }
    }
    let group_name: BTreeMap<u32, PersonId> = votes
        .into_iter()
        .filter_map(|(g, v)| {
            v.into_iter()
                .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
                .map(|(p, _)| (g, p))
        })
        .collect();
    let centroids: Vec<(PersonId, Vec<f32>)> =
        sums.into_iter().map(|(p, s)| (p, normalized(s))).collect();
    faces
        .iter()
        .map(|f| {
            if let Some(p) = explicit(f) {
                return Who::Person(p);
            }
            let not = not_of(f);
            let group = group_of(f);
            if let Some(p) = group.and_then(|g| group_name.get(&g)) {
                if !not.contains(p) {
                    return Who::Person(*p);
                }
            }
            // Rule 3 is for faces the snapshot does not know yet.
            if group.is_none() {
                let near = centroids
                    .iter()
                    .filter(|(p, _)| !not.contains(p))
                    .map(|(p, c)| (*p, dot(c, &f.embedding)))
                    .filter(|(_, s)| *s >= NEAREST_SIMILARITY)
                    .max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0)));
                if let Some((p, _)) = near {
                    return Who::Person(p);
                }
            }
            match group {
                Some(g) => Who::Group(g),
                None => Who::Unknown,
            }
        })
        .collect()
}
