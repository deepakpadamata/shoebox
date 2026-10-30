//! People (S1-18): grouping, and who each face is, the same on every device.

use proptest::prelude::*;
use shoebox_core::hlc::Hlc;
use shoebox_core::ids::{Blake3Hash, EndpointId};
use shoebox_core::library_state::{LibraryState, StateEntry};
use shoebox_core::merge::{Lww, OrSet};
use shoebox_core::model::{FaceLabel, Person, PersonId, PhotoId};
use shoebox_search::encoder::normalize;
use shoebox_search::people::{canonical, group, resolve, FaceIn, Who};
use std::collections::BTreeMap;

fn pid(i: u32) -> PhotoId {
    Blake3Hash::of_bytes(&i.to_le_bytes())
}

/// A face of person `who` (0, 1, 2…) in photo `i`: the person's direction plus a little noise.
fn face(who: u32, i: u32) -> FaceIn {
    let mut v: Vec<f32> = (0..128)
        .map(|d| {
            let base = if d % 3 == who as usize % 3 && d / 43 == who as usize / 3 {
                1.0
            } else {
                0.0
            };
            let h = i
                .wrapping_mul(2_654_435_761)
                .wrapping_add((d as u32).wrapping_mul(40_503));
            let noise = ((h >> 7) % 1000) as f32 / 1000.0 - 0.5;
            base + 0.25 * noise
        })
        .collect();
    normalize(&mut v);
    FaceIn {
        photo: pid(i),
        rect: [0.2, 0.2, 0.4, 0.5],
        quality: 200,
        embedding: v,
    }
}

fn hlc(ms: u64) -> Hlc {
    Hlc {
        physical_ms: ms,
        counter: 0,
        device: EndpointId([1; 32]),
    }
}

fn person(id: u8, name: &str, merged: Option<u8>) -> Person {
    Person {
        id: PersonId([id; 16]),
        name: Lww::new(name.into(), hlc(1)),
        hidden: Lww::new(false, hlc(1)),
        merged_into: Lww::new(merged.map(|m| PersonId([m; 16])), hlc(1)),
    }
}

fn label(photo: PhotoId, is: Option<u8>, not: &[u8]) -> FaceLabel {
    let mut is_not = OrSet::new();
    for n in not {
        is_not.add(PersonId([*n; 16]), hlc(2));
    }
    FaceLabel {
        photo,
        face: shoebox_search::faces::FaceKey::of([13_107, 13_107, 26_214, 32_768]).0,
        is: Lww::new(is.map(|p| PersonId([p; 16])), hlc(2)),
        is_not,
    }
}

fn people(list: &[Person]) -> BTreeMap<PersonId, Person> {
    list.iter().map(|p| (p.id, p.clone())).collect()
}

#[test]
fn faces_of_one_person_group_together() {
    let faces: Vec<FaceIn> = (0..30).map(|i| face(i % 3, i)).collect();
    let snap = group(&faces, &[], &BTreeMap::new(), None);
    let groups: std::collections::BTreeSet<u32> = snap.faces.iter().map(|f| f.2).collect();
    assert_eq!(groups.len(), 3, "{:?}", snap.faces);
    for who in 0..3 {
        let g: std::collections::BTreeSet<u32> = (0..30)
            .filter(|i| i % 3 == who)
            .map(|i| snap.group_of(&pid(i), faces[i as usize].rect).unwrap())
            .collect();
        assert_eq!(g.len(), 1);
    }
    let back = shoebox_search::people::Snapshot::decode(&snap.encode()).unwrap();
    assert_eq!(back, snap);
}

#[test]
fn naming_a_few_faces_names_the_group_and_not_ana_always_wins() {
    let faces: Vec<FaceIn> = (0..20).map(|i| face(i % 2, i)).collect();
    let ana = person(1, "Ana", None);
    let snap = group(&faces, &[], &BTreeMap::new(), None);
    // Two of person 0's faces are labelled Ana; one of person 0's faces is "not Ana".
    let labels = vec![
        label(pid(0), Some(1), &[]),
        label(pid(2), Some(1), &[]),
        label(pid(4), None, &[1]),
    ];
    let who = resolve(&faces, &labels, &people(&[ana]), Some(&snap));
    for (i, w) in who.iter().enumerate() {
        let want_ana = i % 2 == 0 && i != 4;
        assert_eq!(
            *w == Who::Person(PersonId([1; 16])),
            want_ana,
            "face {i}: {w:?}"
        );
    }
    assert!(matches!(who[4], Who::Group(_)));
    assert!(
        matches!(who[1], Who::Group(_)),
        "person 1's group stays unnamed"
    );
}

#[test]
fn new_faces_go_to_the_nearest_named_person() {
    let faces: Vec<FaceIn> = (0..10).map(|i| face(0, i)).collect();
    let snap = group(
        &faces,
        &[label(pid(0), Some(1), &[])],
        &BTreeMap::new(),
        None,
    );
    let mut later = faces.clone();
    later.push(face(0, 100));
    let who = resolve(
        &later,
        &[label(pid(0), Some(1), &[])],
        &people(&[person(1, "Ana", None)]),
        Some(&snap),
    );
    assert_eq!(who[10], Who::Person(PersonId([1; 16])));
}

#[test]
fn merges_are_followed_and_cycles_end_at_the_lowest_id() {
    let p = people(&[
        person(3, "", Some(2)),
        person(2, "", Some(5)),
        person(5, "Ana", None),
    ]);
    assert_eq!(canonical(PersonId([3; 16]), &p), PersonId([5; 16]));
    let cycle = people(&[
        person(7, "", Some(8)),
        person(8, "", Some(9)),
        person(9, "", Some(7)),
    ]);
    for start in [7, 8, 9] {
        assert_eq!(canonical(PersonId([start; 16]), &cycle), PersonId([7; 16]));
    }
}

proptest! {
    /// Two devices receiving the same people records in any order work out the same people.
    #[test]
    fn resolution_does_not_depend_on_arrival_order(order in Just((0..8usize).collect::<Vec<_>>()).prop_shuffle()) {
        let faces: Vec<FaceIn> = (0..16).map(|i| face(i % 2, i)).collect();
        let entries = [
            StateEntry::Person(person(1, "Ana", None)),
            StateEntry::Person(person(2, "Ben", Some(1))),
            StateEntry::Person(person(3, "Cat", None)),
            StateEntry::FaceLabel(label(pid(0), Some(1), &[])),
            StateEntry::FaceLabel(label(pid(1), Some(3), &[])),
            StateEntry::FaceLabel(label(pid(2), None, &[1])),
            StateEntry::FaceLabel(label(pid(3), Some(2), &[])),
            StateEntry::FaceLabel(label(pid(5), Some(3), &[])),
        ];
        let run = |idx: &[usize]| {
            let mut s = LibraryState::new(EndpointId([9; 32]));
            for &i in idx {
                s.apply_entry(&entries[i]);
            }
            let persons = people(&s.persons());
            let labels = s.face_labels();
            let snap = group(&faces, &labels, &persons, None);
            resolve(&faces, &labels, &persons, Some(&snap))
        };
        let base: Vec<usize> = (0..8).collect();
        prop_assert_eq!(run(&order), run(&base));
    }
}
