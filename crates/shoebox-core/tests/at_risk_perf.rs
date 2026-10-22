//! M1-11 acceptance: `at_risk` on a 100k-photo library answers in under 200 ms.

mod fixtures;

use fixtures::{dev, hlc};
use shoebox_core::library_state::LibraryState;
use shoebox_core::merge::Lww;
use shoebox_core::model::*;
use shoebox_core::store::*;
use shoebox_core::{Blake3Hash, Timestamp};
use std::time::Instant;

#[test]
fn at_risk_on_100k_photos_is_fast() {
    // Debug builds are ~40x slower to *build* the library, so they check 10k photos; the full
    // 100k acceptance run is `cargo test -p shoebox-core --release --test at_risk_perf`.
    let n: u32 = if cfg!(debug_assertions) {
        10_000
    } else {
        100_000
    };
    let mut s = LibraryState::new(dev(1));
    let blob = |h: Blake3Hash| BlobRef {
        hash: h,
        size: 1,
        mime: "image/jpeg".into(),
    };
    let mut held = vec![];
    for i in 0..n {
        let id = Blake3Hash::of_bytes(&i.to_le_bytes());
        s.upsert_photo(&PhotoRecord {
            id,
            original: blob(id),
            preview: blob(id),
            thumb: blob(id),
            taken_at: Timestamp(i as i64),
            exif: ExifSummary::default(),
            imported_by: dev(1),
            favorite: Lww::new(false, hlc(1, 0, 1)),
            deleted_at: Lww::new(None, hlc(1, 0, 1)),
        });
        // Two thirds of the photos are held by both devices, the rest by one.
        if i % 3 != 0 {
            held.push((id, BlobKind::Original));
        }
    }
    let two_copies = held.len();
    s.put_holdings(
        dev(1),
        &HoldingsDelta {
            hlc: hlc(5, 0, 1),
            added: held.clone(),
            removed: vec![],
        },
    )
    .unwrap();
    let mut other = LibraryState::new(dev(2));
    other
        .put_holdings(
            dev(2),
            &HoldingsDelta {
                hlc: hlc(6, 0, 2),
                added: held,
                removed: vec![],
            },
        )
        .unwrap();
    s.merge_from(&other);

    let start = Instant::now();
    let risky = s.at_risk(2, Page::all());
    let elapsed = start.elapsed();
    assert_eq!(
        risky.len(),
        n as usize - two_copies,
        "photos with fewer than 2 copies"
    );
    eprintln!("at_risk over {n} photos: {elapsed:?}");
    // The 200 ms budget is for the shipped (optimised) build; debug builds get generous slack.
    let budget = if cfg!(debug_assertions) { 3_000 } else { 200 };
    assert!(
        elapsed.as_millis() < budget,
        "{elapsed:?} exceeds {budget} ms"
    );
}
