//! People in the app (S1-19): the People screen, a person's photos, naming and correcting.
//! Everything is worked out on this device (`LibraryNode::people_view`); changes are library
//! records that reach the other devices by sync.

use crate::core::{parse_hash, AppCore};
use crate::types::{FaceOut, PeopleList, PersonCard, PhotoSummary};
use anyhow::{anyhow, bail, Result};
use shoebox_core::model::{PersonId, PhotoId};
use shoebox_core::store::{FaceEdit, LibraryStore, PersonEdit};
use shoebox_node::{PeopleView, ME_SETTING};
use shoebox_search::people::{canonical, Who, MIN_GROUP};
use std::collections::{BTreeMap, BTreeSet};

fn person_id(s: &str) -> Result<PersonId> {
    let b = hex::decode(s)?;
    Ok(PersonId(
        b.try_into().map_err(|_| anyhow!("bad person id"))?,
    ))
}

fn new_person_id() -> PersonId {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).expect("random");
    PersonId(b)
}

impl AppCore {
    async fn view(&self) -> Result<PeopleView> {
        let ln = self.ln().await?;
        ln.people_view(ln.face_model()).await
    }

    async fn face_out(
        &self,
        f: &shoebox_search::people::FaceIn,
        who: &Who,
        v: &PeopleView,
    ) -> Result<FaceOut> {
        let ln = self.ln().await?;
        let rec = ln.store().photo(&f.photo).await?;
        let (person, name) = match who {
            Who::Person(p) => (
                Some(hex::encode(p.0)),
                v.persons.get(p).map(|x| x.name.value.clone()),
            ),
            _ => (None, None),
        };
        Ok(FaceOut {
            photo: f.photo.to_string(),
            rect: f.rect.to_vec(),
            width: rec.as_ref().map_or(0, |r| r.exif.width),
            height: rec.as_ref().map_or(0, |r| r.exif.height),
            person,
            name,
            group: match who {
                Who::Group(g) => Some(*g),
                _ => None,
            },
        })
    }

    /// Named people (most photos first), then unnamed groups with at least five faces.
    pub async fn people(&self) -> Result<PeopleList> {
        let ln = self.ln().await?;
        let on = ln.people_on().await?;
        if !on {
            return Ok(PeopleList {
                on,
                people: vec![],
                groups: vec![],
            });
        }
        let v = self.view().await?;
        let me = v.me.map(|m| canonical(m, &v.persons));
        let mut by: BTreeMap<Who, Vec<usize>> = BTreeMap::new();
        for (i, (_, w)) in v.faces.iter().enumerate() {
            by.entry(*w).or_default().push(i);
        }
        let (mut people, mut groups) = (vec![], vec![]);
        for (who, faces) in by {
            let photos: BTreeSet<PhotoId> = faces.iter().map(|i| v.faces[*i].0.photo).collect();
            // The cover: the best face.
            let best = *faces
                .iter()
                .max_by_key(|i| v.faces[**i].0.quality)
                .expect("non-empty");
            let cover = self.face_out(&v.faces[best].0, &who, &v).await?;
            match who {
                Who::Person(p) => {
                    let person = v.persons.get(&p);
                    people.push(PersonCard {
                        id: hex::encode(p.0),
                        name: person.map(|x| x.name.value.clone()).unwrap_or_default(),
                        count: photos.len() as u32,
                        cover,
                        me: Some(p) == me,
                        hidden: person.is_some_and(|x| x.hidden.value),
                    });
                }
                Who::Group(g) if faces.len() >= MIN_GROUP => groups.push(PersonCard {
                    id: format!("group:{g}"),
                    name: String::new(),
                    count: photos.len() as u32,
                    cover,
                    me: false,
                    hidden: false,
                }),
                _ => {}
            }
        }
        people.retain(|p| !p.hidden);
        people.sort_by(|a, b| b.count.cmp(&a.count).then(a.name.cmp(&b.name)));
        groups.sort_by_key(|g| std::cmp::Reverse(g.count));
        Ok(PeopleList { on, people, groups })
    }

    fn who_of(id: &str) -> Result<Who> {
        Ok(match id.strip_prefix("group:") {
            Some(g) => Who::Group(g.parse()?),
            None => Who::Person(person_id(id)?),
        })
    }

    /// The photos a person (or an unnamed group) is in, newest first.
    pub async fn person_photos(&self, id: &str) -> Result<Vec<PhotoSummary>> {
        let who = Self::who_of(id)?;
        let v = self.view().await?;
        let photos: BTreeSet<PhotoId> = v
            .faces
            .iter()
            .filter(|(_, w)| *w == who)
            .map(|(f, _)| f.photo)
            .collect();
        let ln = self.ln().await?;
        let mut recs = vec![];
        for p in photos {
            if let Some(r) = ln.store().photo(&p).await? {
                if !r.is_deleted() {
                    recs.push(r);
                }
            }
        }
        recs.sort_by_key(|r| std::cmp::Reverse(r.taken_at));
        self.summaries(&ln, &recs).await
    }

    /// Name a person, or an unnamed group (which becomes a person: its best faces are labelled
    /// with the new person, and the group follows them).
    pub async fn name_person(&self, id: &str, name: &str) -> Result<String> {
        let name = name.trim();
        if name.is_empty() {
            bail!("a name is needed");
        }
        let ln = self.ln().await?;
        match Self::who_of(id)? {
            Who::Person(p) => {
                ln.store()
                    .edit_person(
                        p,
                        PersonEdit {
                            name: Some(name.into()),
                            ..Default::default()
                        },
                    )
                    .await?;
                Ok(hex::encode(p.0))
            }
            Who::Group(g) => {
                let v = self.view().await?;
                let p = new_person_id();
                ln.store()
                    .edit_person(
                        p,
                        PersonEdit {
                            name: Some(name.into()),
                            ..Default::default()
                        },
                    )
                    .await?;
                let mut faces: Vec<&shoebox_search::people::FaceIn> = v
                    .faces
                    .iter()
                    .filter(|(_, w)| *w == Who::Group(g))
                    .map(|(f, _)| f)
                    .collect();
                faces.sort_by_key(|f| std::cmp::Reverse(f.quality));
                for f in faces.into_iter().take(5) {
                    ln.store()
                        .label_face(
                            f.photo,
                            f.key().0,
                            FaceEdit {
                                is: Some(Some(p)),
                                ..Default::default()
                            },
                        )
                        .await?;
                }
                Ok(hex::encode(p.0))
            }
            Who::Unknown => bail!("nobody to name"),
        }
    }

    /// Merge `from` into `into` (both people; a group is named first).
    pub async fn merge_people(&self, from: &str, into: &str) -> Result<()> {
        let ln = self.ln().await?;
        let (Who::Person(a), Who::Person(b)) = (Self::who_of(from)?, Self::who_of(into)?) else {
            bail!("name a group before merging it");
        };
        ln.store()
            .edit_person(
                a,
                PersonEdit {
                    merged_into: Some(Some(b)),
                    ..Default::default()
                },
            )
            .await?;
        Ok(())
    }

    pub async fn hide_person(&self, id: &str, hidden: bool) -> Result<()> {
        let ln = self.ln().await?;
        let Who::Person(p) = Self::who_of(id)? else {
            bail!("not a person")
        };
        ln.store()
            .edit_person(
                p,
                PersonEdit {
                    hidden: Some(hidden),
                    ..Default::default()
                },
            )
            .await?;
        Ok(())
    }

    /// This person is me ("photos of me").
    pub async fn set_me(&self, id: &str) -> Result<()> {
        let ln = self.ln().await?;
        let Who::Person(p) = Self::who_of(id)? else {
            bail!("not a person")
        };
        ln.store().put_setting(ME_SETTING, p.0.to_vec()).await?;
        Ok(())
    }

    /// Faces in a photo, with who they are.
    pub async fn photo_faces(&self, photo: &str) -> Result<Vec<FaceOut>> {
        let id = parse_hash(photo)?;
        let v = self.view().await?;
        let mut out = vec![];
        for (f, w) in v.faces.iter().filter(|(f, _)| f.photo == id) {
            out.push(self.face_out(f, w, &v).await?);
        }
        Ok(out)
    }

    /// Correct a face: it is `person` (an id, or a new name), or it is not `not`.
    pub async fn label_face(
        &self,
        photo: &str,
        rect: &[f32],
        person: Option<&str>,
        new_name: Option<&str>,
        not: Option<&str>,
    ) -> Result<()> {
        let ln = self.ln().await?;
        let photo = parse_hash(photo)?;
        let r: [f32; 4] = rect
            .try_into()
            .map_err(|_| anyhow!("a face box has four numbers"))?;
        let key = shoebox_node::LibraryNode::<shoebox_store::Library>::face_key(r).0;
        let is = match (person, new_name) {
            (Some(p), _) => Some(Some(person_id(p)?)),
            (None, Some(n)) if !n.trim().is_empty() => {
                let p = new_person_id();
                ln.store()
                    .edit_person(
                        p,
                        PersonEdit {
                            name: Some(n.trim().into()),
                            ..Default::default()
                        },
                    )
                    .await?;
                Some(Some(p))
            }
            _ => None,
        };
        let is_not = not.map(person_id).transpose()?;
        ln.store()
            .label_face(photo, key, FaceEdit { is, is_not })
            .await?;
        Ok(())
    }

    /// Turn people on or off for the whole library.
    pub async fn set_people(&self, on: bool) -> Result<()> {
        self.ln().await?.set_people(on).await
    }
}
