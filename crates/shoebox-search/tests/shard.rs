//! Index shards (S1-06): the blob format and which shards count.

use proptest::prelude::*;
use shoebox_core::hlc::Hlc;
use shoebox_core::ids::{Blake3Hash, EndpointId};
use shoebox_core::model::{BlobRef, IndexShard, ModelId, PhotoId};
use shoebox_search::shard::*;
use std::collections::BTreeSet;

const MODEL: ModelId = ModelId([7; 32]);

fn pid(i: u32) -> PhotoId {
    Blake3Hash::of_bytes(&i.to_le_bytes())
}

fn ids(n: u32) -> Vec<PhotoId> {
    (0..n).map(pid).collect()
}

fn vec8_shard(photos: &[PhotoId], replaces: Vec<Blake3Hash>) -> Shard {
    let rows = photos
        .iter()
        .map(|p| {
            Vec8::quantize(
                &p.0.iter()
                    .map(|b| f32::from(*b) - 128.0)
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    Shard::build(MODEL, 32, replaces, Rows::Vec8(rows), photos.to_vec()).unwrap()
}

#[test]
fn every_kind_round_trips() {
    let photos = ids(5);
    let shards = [
        vec8_shard(&photos, vec![Blake3Hash([1; 32])]),
        Shard::build(
            MODEL,
            32,
            vec![],
            Rows::Vec1(photos.iter().map(|p| p.0[..4].to_vec()).collect()),
            photos.clone(),
        )
        .unwrap(),
        Shard::build(
            MODEL,
            0,
            vec![],
            Rows::Text(
                (0..5)
                    .map(|i| TextRow {
                        labels: vec![(i, 200), (300, 17)],
                        ocr: if i % 2 == 0 {
                            format!("Café {i} ☕")
                        } else {
                            String::new()
                        },
                    })
                    .collect(),
            ),
            photos.clone(),
        )
        .unwrap(),
        Shard::build(
            MODEL,
            8,
            vec![],
            Rows::Faces(
                (0..5u16)
                    .map(|i| {
                        (0..i % 3)
                            .map(|k| Face {
                                bbox: [i, k, 1000, 2000],
                                landmarks: [k; 10],
                                quality: 99,
                                embedding: Vec8::quantize(&[
                                    0.5, -0.25, 0.0, 1.0, 0.1, 0.2, 0.3, -1.0,
                                ]),
                            })
                            .collect()
                    })
                    .collect(),
            ),
            photos.clone(),
        )
        .unwrap(),
    ];
    for s in shards {
        let bytes = s.encode();
        assert_eq!(Shard::decode(&bytes).unwrap(), s, "{:?}", s.kind());
        assert!(Shard::decode(&bytes[..bytes.len() - 1]).is_err());
        let mut longer = bytes.clone();
        longer.push(0);
        assert_eq!(Shard::decode(&longer), Err(ShardError::Length));
    }
}

#[test]
fn bad_shards_are_refused() {
    let s = vec8_shard(&ids(3), vec![]);
    let bytes = s.encode();
    let mut b = bytes.clone();
    b[0] = b'X';
    assert_eq!(Shard::decode(&b), Err(ShardError::BadMagic));
    let mut b = bytes.clone();
    b[4] = 2;
    assert_eq!(Shard::decode(&b), Err(ShardError::UnknownVersion(2)));
    let mut b = bytes.clone();
    b[5] = 9;
    assert_eq!(Shard::decode(&b), Err(ShardError::UnknownKind(9)));
    // Swap the first two ids.
    let mut b = bytes.clone();
    let at = 4 + 1 + 1 + 32 + 2 + 4 + 2;
    let (first, second) = (b[at..at + 32].to_vec(), b[at + 32..at + 64].to_vec());
    b[at..at + 32].copy_from_slice(&second);
    b[at + 32..at + 64].copy_from_slice(&first);
    assert_eq!(Shard::decode(&b), Err(ShardError::Unsorted));
    // A count larger than the rows.
    let mut b = bytes;
    b[40] = 4;
    assert!(Shard::decode(&b).is_err());
}

#[test]
fn build_sorts_and_keeps_the_later_row() {
    let (a, b) = (pid(1), pid(2));
    let rows = Rows::Text(vec![
        TextRow {
            labels: vec![],
            ocr: "b".into(),
        },
        TextRow {
            labels: vec![],
            ocr: "a old".into(),
        },
        TextRow {
            labels: vec![],
            ocr: "a new".into(),
        },
    ]);
    let s = Shard::build(MODEL, 0, vec![], rows, vec![b, a, a]).unwrap();
    let mut want = vec![a, b];
    want.sort();
    assert_eq!(s.ids, want);
    let Rows::Text(rows) = &s.rows else { panic!() };
    assert_eq!(rows[s.position(&a).unwrap()].ocr, "a new");
    assert!(Shard::build(
        MODEL,
        0,
        vec![],
        Rows::Text(vec![TextRow::default(); 1025]),
        ids(1025)
    )
    .is_err());
}

#[test]
fn quantisation_keeps_direction() {
    let x: Vec<f32> = (0..512)
        .map(|i| ((i * 37 % 101) as f32 - 50.0) / 50.0)
        .collect();
    let q = Vec8::quantize(&x).to_f32();
    let dot: f32 = x.iter().zip(&q).map(|(a, b)| a * b).sum();
    let (nx, nq) = (
        x.iter().map(|v| v * v).sum::<f32>().sqrt(),
        q.iter().map(|v| v * v).sum::<f32>().sqrt(),
    );
    assert!(dot / (nx * nq) > 0.999);
    assert_eq!(
        sign_bits(&[1.0, -1.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0, 3.0]),
        vec![0b1001, 1]
    );
}

// ---------- liveness ----------

fn record(s: &Shard, author: u8, ms: u64) -> (IndexShard, Shard) {
    let bytes = s.encode();
    (
        IndexShard {
            blob: BlobRef {
                hash: Blake3Hash::of_bytes(&bytes),
                size: bytes.len() as u64,
                mime: "application/x-shoebox-index".into(),
            },
            model: s.model,
            kind: s.kind(),
            count: s.ids.len() as u32,
            replaces: s.replaces.clone(),
            made_by: EndpointId([author; 32]),
            hlc: Hlc {
                physical_ms: ms,
                counter: 0,
                device: EndpointId([author; 32]),
            },
        },
        s.clone(),
    )
}

fn all_fetched(set: &[(IndexShard, Shard)]) -> impl Fn(&Blake3Hash) -> Fetched + '_ {
    move |h| {
        set.iter()
            .find(|(r, _)| r.blob.hash == *h)
            .map_or(Fetched::Unknown, |(_, s)| Fetched::Valid(s.ids.clone()))
    }
}

#[test]
fn compaction_replaces_and_duplicates_collapse() {
    let a = record(&vec8_shard(&ids(4), vec![]), 1, 1);
    let b = record(&vec8_shard(&ids(8)[4..], vec![]), 2, 2);
    // A second device indexed photos 2..6 too.
    let dup = record(&vec8_shard(&ids(6)[2..], vec![]), 3, 3);
    let set = vec![a.clone(), b.clone(), dup.clone()];
    let live = live_shards(set.iter().map(|(r, _)| r), all_fetched(&set));
    assert_eq!(live.len(), 3);
    let w = winners(set.iter().map(|(r, s)| (r.blob.hash, s.ids.as_slice())));
    assert_eq!(w.len(), 8);
    assert_eq!(w[&pid(3)], a.0.blob.hash.min(dup.0.blob.hash));

    // Compaction merges a and b, leaving photo 7 out (purged).
    let merged = record(
        &vec8_shard(&ids(7), vec![a.0.blob.hash, b.0.blob.hash]),
        1,
        9,
    );
    let mut set = set;
    set.push(merged.clone());
    let live = live_shards(set.iter().map(|(r, _)| r), all_fetched(&set));
    assert_eq!(live, BTreeSet::from([dup.0.blob.hash, merged.0.blob.hash]));

    // Not fetched yet: nothing is replaced.
    let unknown = |h: &Blake3Hash| {
        if *h == merged.0.blob.hash {
            Fetched::Unknown
        } else {
            all_fetched(&set)(h)
        }
    };
    assert_eq!(live_shards(set.iter().map(|(r, _)| r), unknown).len(), 4);

    // Fetched and bad: ignored, and replaces nothing.
    let bad = |h: &Blake3Hash| {
        if *h == merged.0.blob.hash {
            Fetched::Invalid
        } else {
            all_fetched(&set)(h)
        }
    };
    let live = live_shards(set.iter().map(|(r, _)| r), bad);
    assert_eq!(
        live,
        BTreeSet::from([a.0.blob.hash, b.0.blob.hash, dup.0.blob.hash])
    );
}

#[test]
fn a_replacement_must_stay_within_what_it_replaces() {
    let a = record(&vec8_shard(&ids(2), vec![]), 1, 1);
    let sneaky = record(&vec8_shard(&ids(5), vec![a.0.blob.hash]), 2, 2);
    let set = vec![a.clone(), sneaky.clone()];
    let live = live_shards(set.iter().map(|(r, _)| r), all_fetched(&set));
    assert!(
        live.contains(&a.0.blob.hash),
        "a photo outside the replaced shard"
    );
    // Another kind or model never replaces.
    let mut other = vec8_shard(&ids(2), vec![a.0.blob.hash]);
    other.model = ModelId([8; 32]);
    let other = record(&other, 2, 3);
    let set = vec![a.clone(), other];
    let live = live_shards(set.iter().map(|(r, _)| r), all_fetched(&set));
    assert_eq!(live.len(), 2);
}

#[test]
fn forged_cycles_stay_live() {
    let mut a = record(&vec8_shard(&ids(2), vec![]), 1, 1).0;
    let mut b = record(&vec8_shard(&ids(3)[1..], vec![]), 1, 2).0;
    a.replaces = vec![b.blob.hash];
    b.replaces = vec![a.blob.hash];
    let fetched = |_: &Blake3Hash| Fetched::Valid(ids(3));
    assert_eq!(live_shards([&a, &b], fetched).len(), 2);
}

proptest! {
    /// Live set and winners depend only on the set of records: not on arrival order or repeats.
    #[test]
    fn order_and_repetition_do_not_matter(
        plan in prop::collection::vec((0u32..40, 1u32..12, prop::collection::vec(any::<prop::sample::Index>(), 0..3), 0u8..4), 1..14),
        shuffle in prop::collection::vec(any::<prop::sample::Index>(), 0..40),
        fetched_mask in any::<u32>(),
    ) {
        // Build shards that replace earlier ones (as compaction does) with arbitrary coverage.
        let mut made: Vec<(IndexShard, Shard)> = vec![];
        for (i, (start, len, picks, author)) in plan.iter().enumerate() {
            let replaces: Vec<Blake3Hash> = if made.is_empty() {
                vec![]
            } else {
                picks.iter().map(|p| made[p.index(made.len())].0.blob.hash).collect()
            };
            let photos: Vec<PhotoId> = (*start..start + len).map(pid).collect();
            made.push(record(&vec8_shard(&photos, replaces), *author, i as u64));
        }
        let fetched = |h: &Blake3Hash| {
            let i = made.iter().position(|(r, _)| r.blob.hash == *h).unwrap();
            match (fetched_mask >> (2 * (i % 16))) & 3 {
                0 => Fetched::Unknown,
                1 => Fetched::Invalid,
                _ => Fetched::Valid(made[i].1.ids.clone()),
            }
        };
        let base: Vec<&IndexShard> = made.iter().map(|(r, _)| r).collect();
        let want = live_shards(base.iter().copied(), fetched);
        let mut mixed: Vec<&IndexShard> = base.clone();
        for s in &shuffle {
            let i = s.index(base.len());
            mixed.push(base[i]);
            let (j, last) = (s.index(mixed.len()), mixed.len() - 1);
            mixed.swap(j, last);
        }
        mixed.reverse();
        prop_assert_eq!(&live_shards(mixed.iter().copied(), fetched), &want);
        let rows = |order: &[&IndexShard]| {
            let v: Vec<(Blake3Hash, &[PhotoId])> = order
                .iter()
                .filter(|r| want.contains(&r.blob.hash))
                .map(|r| {
                    let s = &made.iter().find(|(m, _)| m.blob.hash == r.blob.hash).unwrap().1;
                    (r.blob.hash, s.ids.as_slice())
                })
                .collect();
            winners(v)
        };
        prop_assert_eq!(rows(&mixed), rows(&base));
    }
}
