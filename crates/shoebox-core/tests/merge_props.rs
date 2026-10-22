//! Property tests: `merge` is commutative, associative and idempotent for every mergeable type.
//! Generators draw from deliberately small domains so replicas frequently collide.

mod fixtures;

use proptest::prelude::*;
use shoebox_core::merge::{Lww, OrSet};
use shoebox_core::model::*;
use shoebox_core::{Blake3Hash, EndpointId, Hlc, Timestamp};

fn arb_hlc() -> impl Strategy<Value = Hlc> {
    (0u64..6, 0u32..3, 0u8..3).prop_map(|(p, c, d)| Hlc {
        physical_ms: p,
        counter: c,
        device: EndpointId([d; 32]),
    })
}

fn arb_lww<T: std::fmt::Debug + Clone + 'static>(
    value: impl Strategy<Value = T> + 'static,
) -> impl Strategy<Value = Lww<T>> {
    (value, arb_hlc()).prop_map(|(value, hlc)| Lww { value, hlc })
}

fn arb_hash() -> impl Strategy<Value = Blake3Hash> {
    (0u8..4).prop_map(|b| Blake3Hash([b; 32]))
}

fn arb_orset() -> impl Strategy<Value = OrSet<Blake3Hash>> {
    let tags = || proptest::collection::btree_set(arb_hlc(), 0..4);
    let side = || proptest::collection::btree_map(arb_hash(), tags(), 0..4);
    (side(), side()).prop_map(|(adds, removes)| OrSet { adds, removes })
}

fn arb_blob() -> impl Strategy<Value = BlobRef> {
    (0u8..3).prop_map(|b| BlobRef {
        hash: Blake3Hash([b; 32]),
        size: b as u64,
        mime: "image/jpeg".into(),
    })
}

fn arb_photo() -> impl Strategy<Value = PhotoRecord> {
    (
        (arb_blob(), arb_blob(), arb_blob()),
        (0i64..3, 0u8..3),
        arb_lww(any::<bool>()),
        arb_lww(proptest::option::of((0i64..3).prop_map(Timestamp))),
    )
        .prop_map(
            |((original, preview, thumb), (taken, importer), favorite, deleted_at)| PhotoRecord {
                id: Blake3Hash([1; 32]),
                original,
                preview,
                thumb,
                taken_at: Timestamp(taken),
                exif: ExifSummary::default(),
                imported_by: EndpointId([importer; 32]),
                favorite,
                deleted_at,
            },
        )
}

fn arb_album() -> impl Strategy<Value = AlbumMeta> {
    (
        arb_lww(proptest::sample::select(vec![
            "a".to_string(),
            "b".to_string(),
        ])),
        arb_lww(proptest::option::of(arb_hash())),
        arb_orset(),
    )
        .prop_map(|(name, cover, items)| AlbumMeta {
            id: AlbumId([3; 16]),
            name,
            cover,
            items,
        })
}

mod lww {
    use super::*;
    proptest! {
        #[test]
        fn commutative(a in arb_lww(0u8..3), b in arb_lww(0u8..3)) {
            prop_assert_eq!(a.merge(&b), b.merge(&a));
        }
        #[test]
        fn associative(a in arb_lww(0u8..3), b in arb_lww(0u8..3), c in arb_lww(0u8..3)) {
            prop_assert_eq!(a.merge(&b).merge(&c), a.merge(&b.merge(&c)));
        }
        #[test]
        fn idempotent(a in arb_lww(0u8..3)) {
            prop_assert_eq!(a.merge(&a), a);
        }
    }
}

mod orset {
    use super::*;
    proptest! {
        #[test]
        fn commutative(a in arb_orset(), b in arb_orset()) {
            prop_assert_eq!(a.merge(&b), b.merge(&a));
        }
        #[test]
        fn associative(a in arb_orset(), b in arb_orset(), c in arb_orset()) {
            prop_assert_eq!(a.merge(&b).merge(&c), a.merge(&b.merge(&c)));
        }
        #[test]
        fn idempotent(a in arb_orset()) {
            prop_assert_eq!(a.merge(&a), a.clone());
        }
        #[test]
        fn adds_unobserved_by_a_remove_always_survive(
            adds in proptest::collection::btree_set(arb_hlc(), 1..4), item in arb_hash()
        ) {
            // A replica that never saw `fresh` removes `item`; merging keeps `item` present.
            let fresh = Hlc { physical_ms: 99, counter: 0, device: EndpointId([9; 32]) };
            let mut remover = OrSet::new();
            for t in &adds { remover.add(item, *t); }
            remover.remove(&item);
            let mut adder = OrSet::new();
            adder.add(item, fresh);
            prop_assert!(remover.merge(&adder).contains(&item));
            prop_assert!(adder.merge(&remover).contains(&item));
        }
    }
}

mod photo {
    use super::*;
    proptest! {
        #[test]
        fn commutative(a in arb_photo(), b in arb_photo()) {
            prop_assert_eq!(a.merge(&b).unwrap(), b.merge(&a).unwrap());
        }
        #[test]
        fn associative(a in arb_photo(), b in arb_photo(), c in arb_photo()) {
            prop_assert_eq!(
                a.merge(&b).unwrap().merge(&c).unwrap(),
                a.merge(&b.merge(&c).unwrap()).unwrap()
            );
        }
        #[test]
        fn idempotent(a in arb_photo()) {
            prop_assert_eq!(a.merge(&a).unwrap(), a);
        }
    }
}

mod album {
    use super::*;
    proptest! {
        #[test]
        fn commutative(a in arb_album(), b in arb_album()) {
            prop_assert_eq!(a.merge(&b).unwrap(), b.merge(&a).unwrap());
        }
        #[test]
        fn associative(a in arb_album(), b in arb_album(), c in arb_album()) {
            prop_assert_eq!(
                a.merge(&b).unwrap().merge(&c).unwrap(),
                a.merge(&b.merge(&c).unwrap()).unwrap()
            );
        }
        #[test]
        fn idempotent(a in arb_album()) {
            prop_assert_eq!(a.merge(&a).unwrap(), a);
        }
    }
}
