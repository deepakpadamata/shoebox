use futures::executor::block_on;
use shoebox_core::clock::ManualClock;
use shoebox_core::ids::EndpointId;
use shoebox_core::model::*;
use shoebox_core::store::*;
use shoebox_store_mem::{MemBackend, MemNetwork};
use shoebox_testkit::{assert_converged, assert_holders};
use std::panic::{catch_unwind, AssertUnwindSafe};

fn dev(n: u8) -> EndpointId {
    EndpointId([n; 32])
}

fn photo(n: u8) -> PhotoRecord {
    use shoebox_core::merge::Lww;
    use shoebox_core::{Blake3Hash, Hlc, Timestamp};
    let blob = |b: u8| BlobRef {
        hash: Blake3Hash([b; 32]),
        size: 1,
        mime: "image/jpeg".into(),
    };
    let h = Hlc {
        physical_ms: 1,
        counter: 0,
        device: dev(1),
    };
    PhotoRecord {
        id: Blake3Hash([n; 32]),
        original: blob(n),
        preview: blob(n + 100),
        thumb: blob(n + 200),
        taken_at: Timestamp(n as i64),
        exif: ExifSummary::default(),
        imported_by: dev(1),
        favorite: Lww::new(false, h),
        deleted_at: Lww::new(None, h),
    }
}

#[test]
fn assert_converged_passes_after_sync_and_fails_before() {
    block_on(async {
        let net = MemNetwork::new();
        let a = MemBackend::new(net.clone(), dev(1), ManualClock::new(10));
        let b = MemBackend::new(net.clone(), dev(2), ManualClock::new(10));
        let (la, cap) = a.create_library(&RootPublicKey([0; 32])).await.unwrap();
        let lb = b.open_library(&cap).await.unwrap();
        la.upsert_photo(photo(1)).await.unwrap();

        let diverged = catch_unwind(AssertUnwindSafe(|| block_on(assert_converged(&[&la, &lb]))));
        assert!(
            diverged.is_err(),
            "diverged libraries must fail the assertion"
        );

        lb.sync_with(&PeerAddr::id_only(dev(1))).await.unwrap();
        assert_converged(&[&la, &lb]).await;
    });
}

#[test]
fn assert_holders_checks_the_exact_device_set() {
    block_on(async {
        use shoebox_core::{Blake3Hash, Hlc};
        let net = MemNetwork::new();
        let a = MemBackend::new(net, dev(1), ManualClock::new(10));
        let (la, _) = a.create_library(&RootPublicKey([0; 32])).await.unwrap();
        let blob = Blake3Hash([7; 32]);
        la.put_holdings(
            dev(1),
            HoldingsDelta {
                hlc: Hlc {
                    physical_ms: 1,
                    counter: 0,
                    device: dev(1),
                },
                added: vec![(blob, BlobKind::Original)],
                removed: vec![],
            },
        )
        .await
        .unwrap();
        assert_holders(&la, &blob, &[dev(1)]).await;
        let wrong = catch_unwind(AssertUnwindSafe(|| {
            block_on(assert_holders(&la, &blob, &[dev(2)]))
        }));
        assert!(wrong.is_err());
    });
}
