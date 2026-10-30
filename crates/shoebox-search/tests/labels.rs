//! Labels (S1-13): the list, its version and the vectors made for the shipped model.

use shoebox_search::labels::{parse_list, Labels, LABELS_VERSION};
use shoebox_search::models::CLIP_MODEL;
use std::collections::HashSet;

/// `labels.txt` as of LABELS_VERSION. Changing the list changes what text shards mean: bump
/// LABELS_VERSION, run `cargo xtask models`, then update this hash.
const LIST_BLAKE3: &str = "36fdac45e4ea8c901f4fabed7541d60863a905e75bf5a55b40d297c089c66f05";

#[test]
fn the_list_is_pinned_to_its_version() {
    let hash = blake3::hash(include_bytes!("../data/labels.txt"))
        .to_hex()
        .to_string();
    assert_eq!(
        (LABELS_VERSION, hash.as_str()),
        (1, LIST_BLAKE3),
        "labels.txt changed: bump LABELS_VERSION and regenerate labels.bin"
    );
}

#[test]
fn names_and_synonyms_are_unique() {
    let mut seen = HashSet::new();
    for (name, syn) in parse_list(include_str!("../data/labels.txt")) {
        for t in std::iter::once(&name).chain(&syn) {
            assert!(
                seen.insert(shoebox_search::text::fold(t)),
                "{t:?} is listed twice"
            );
        }
    }
    assert!(seen.len() > 1_000);
}

#[test]
fn vectors_are_for_the_shipped_model_and_this_list() {
    let l = Labels::bundled();
    assert!(l.len() > 900);
    assert_eq!(l.model(), Some(CLIP_MODEL), "run `cargo xtask models`");
    assert_eq!(l.lookup("puppy").and_then(|i| l.name(i)), Some("dog"));
    assert_eq!(l.lookup("Kitten").and_then(|i| l.name(i)), Some("cat"));
}

#[test]
fn relabelling_is_fast_enough() {
    let l = Labels::bundled();
    let n = 2_000u32;
    let start = std::time::Instant::now();
    for i in 0..n {
        let mut v: Vec<f32> = (0..512)
            .map(|d| ((i * 31 + d * 7) % 97) as f32 - 48.0)
            .collect();
        shoebox_search::encoder::normalize(&mut v);
        std::hint::black_box(l.assign(&v, shoebox_search::labels::MIN_PROBABILITY));
    }
    let per_100k = start.elapsed() * (100_000 / n);
    eprintln!("re-labelling 100k photos: {per_100k:?}");
    // S1-13: under 2 minutes for 100k on the Mac (release); debug builds are unoptimised.
    if !cfg!(debug_assertions) {
        assert!(per_100k.as_secs() < 120, "{per_100k:?}");
    }
}
