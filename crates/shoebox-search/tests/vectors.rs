//! Vectors on a device (S1-12): the local file, scoring, the binary shortlist and fusion.

use shoebox_core::ids::Blake3Hash;
use shoebox_core::model::{ModelId, PhotoId};
use shoebox_search::encoder::normalize;
use shoebox_search::shard::{sign_bits, Vec8};
use shoebox_search::vectors::{cut, fuse, RowIn, VectorIndex};
use std::collections::HashSet;
use std::time::Instant;

const MODEL: ModelId = ModelId([5; 32]);

fn pid(i: u32) -> PhotoId {
    Blake3Hash::of_bytes(&i.to_le_bytes())
}

/// A pseudo-random unit vector.
fn unit(seed: u32, dims: usize) -> Vec<f32> {
    let mut x = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
    let mut v: Vec<f32> = (0..dims)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x as f32 / u32::MAX as f32) - 0.5
        })
        .collect();
    normalize(&mut v);
    v
}

#[test]
fn builds_reopens_and_ranks() {
    let dir = tempfile::tempdir().unwrap();
    let dims = 64;
    let vecs: Vec<Vec<f32>> = (0..50).map(|i| unit(i, dims)).collect();
    let q8: Vec<Vec8> = vecs.iter().map(|v| Vec8::quantize(v)).collect();
    let rows = (0..50u32).map(|i| RowIn {
        id: pid(i),
        vec8: Some(&q8[i as usize]),
        vec1: None,
    });
    let path = dir.path().join("vec.bin");
    let vx = VectorIndex::build(&path, MODEL, dims, rows).unwrap();
    assert_eq!(vx.len(), 50);
    // The photo itself is its own best match; scores are cosines.
    let hits = vx.score(&vecs[7], None).unwrap();
    assert_eq!(hits[0].0, pid(7));
    assert!((hits[0].1 - 1.0).abs() < 0.02);
    // Reopened, it gives the same.
    let again = VectorIndex::open(&path).unwrap();
    assert_eq!(again.score(&vecs[7], None).unwrap(), hits);
    assert!(again.vector(&pid(7)).unwrap().is_some());
    // Restricted to some photos.
    let allowed: HashSet<PhotoId> = [pid(1), pid(2), pid(3)].into_iter().collect();
    let some = vx.score(&vecs[2], Some(&allowed)).unwrap();
    assert_eq!(some.len(), 3);
    assert_eq!(some[0].0, pid(2));
    // A damaged file is not used.
    let bytes = std::fs::read(&path).unwrap();
    std::fs::write(&path, &bytes[..bytes.len() - 1]).unwrap();
    assert!(VectorIndex::open(&path).is_none());
}

#[test]
fn rows_with_binary_codes_only_are_still_found() {
    let dir = tempfile::tempdir().unwrap();
    let dims = 128;
    let vecs: Vec<Vec<f32>> = (0..20).map(|i| unit(i, dims)).collect();
    let bits: Vec<Vec<u8>> = vecs.iter().map(|v| sign_bits(v)).collect();
    let rows = (0..20u32).map(|i| RowIn {
        id: pid(i),
        vec8: None,
        vec1: Some(&bits[i as usize]),
    });
    let vx = VectorIndex::build(&dir.path().join("v.bin"), MODEL, dims, rows).unwrap();
    let hits = vx.score(&vecs[4], None).unwrap();
    assert_eq!(hits[0].0, pid(4));
}

#[test]
fn fusion_and_the_cut() {
    let (a, b, c) = (pid(1), pid(2), pid(3));
    assert_eq!(fuse(&[&[a, b], &[b, c]]), vec![b, a, c]);
    let scored = vec![(a, 0.30), (b, 0.25), (c, 0.10)];
    assert_eq!(cut(&scored, 0.08, 10), vec![a, b]);
    assert_eq!(cut(&scored, 0.5, 1), vec![a]);
    assert!(cut(&[], 0.1, 5).is_empty());
}

#[test]
fn a_hundred_thousand_photos() {
    let dir = tempfile::tempdir().unwrap();
    let (n, dims) = (100_000u32, 512);
    let q: Vec<Vec8> = (0..n).map(|i| Vec8::quantize(&unit(i, dims))).collect();
    let path = dir.path().join("vec.bin");
    let vx = VectorIndex::build(
        &path,
        MODEL,
        dims,
        (0..n).map(|i| RowIn {
            id: pid(i),
            vec8: Some(&q[i as usize]),
            vec1: None,
        }),
    )
    .unwrap();
    drop(q);
    let query = unit(99_999, dims);
    let t = Instant::now();
    let hits = vx.score(&query, None).unwrap();
    let took = t.elapsed();
    assert_eq!(hits[0].0, pid(99_999));
    // 50 ms on the Mac (S1-12) in release; our crates are unoptimised in debug builds.
    let budget = if cfg!(debug_assertions) { 3_000 } else { 50 };
    assert!(took.as_millis() < budget, "{took:?}");
    eprintln!("100k photos scored in {took:?}");
}
