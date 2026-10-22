//! Backend conformance suite (M0-07). One behavioural spec every [`StoreBackend`] must pass.
//!
//! Enabled by the `testing` feature. Implement [`Harness`] for your backend and call [`run`]
//! from any executor (the suite is runtime-agnostic):
//!
//! ```ignore
//! #[test]
//! fn passes_conformance() {
//!     futures::executor::block_on(shoebox_core::conformance::run(&MyHarness::default()));
//! }
//! ```

use crate::clock::ManualClock;
use crate::hlc::Hlc;
use crate::ids::{Blake3Hash, EndpointId, Timestamp};
use crate::merge::{Lww, OrSet};
use crate::model::*;
use crate::store::*;
use async_trait::async_trait;
use futures::StreamExt;

/// Adapter between the suite and a concrete backend.
#[async_trait]
pub trait Harness: Send + Sync + 'static {
    type Backend: StoreBackend;

    /// A fresh, isolated world: each scenario gets its own, so no state leaks between scenarios.
    fn fresh(&self) -> Self;

    /// A backend for `device`, reading time from `clock`. Backends created from the same world
    /// can reach each other via [`Harness::peer_addr`].
    async fn backend(&self, device: EndpointId, clock: ManualClock) -> Self::Backend;

    /// How to dial `device`'s replicas in this world.
    fn peer_addr(&self, device: EndpointId) -> PeerAddr;
}

/// Run every scenario, panicking (with the scenario name logged) on the first failure.
pub async fn run<H: Harness>(h: &H) {
    run_only(h, |_| true).await
}

macro_rules! scenario_table {
    ($($name:ident),* $(,)?) => {
        /// Names of all scenarios, in run order.
        pub const SCENARIOS: &[&str] = &[$(stringify!($name)),*];

        /// Run the scenarios whose name satisfies `include` (for backends that implement only
        /// part of the API, e.g. no shared albums yet). Names are listed in [`SCENARIOS`].
        pub async fn run_only<H: Harness>(h: &H, include: impl Fn(&str) -> bool) {
            $(
                if include(stringify!($name)) {
                    eprintln!("conformance: {}", stringify!($name));
                    $name(&h.fresh()).await;
                }
            )*
        }
    };
}

scenario_table!(
    upsert_is_idempotent_and_merges,
    paging_by_time,
    favorite_is_last_writer_wins,
    delete_restore_purge,
    album_add_wins,
    holdings_single_writer_and_at_risk,
    change_stream_events,
    concurrent_edits_converge,
    delete_races_favorite,
    device_list_is_signed_and_order_independent,
    library_settings_are_last_writer_wins,
    read_only_capability,
    shared_album_flow,
);

// ---------- helpers ----------

type Lib<H> = <<H as Harness>::Backend as StoreBackend>::Library;

struct Node<H: Harness> {
    /// Kept alive for the node's lifetime (some backends tie the store to it).
    #[allow(dead_code)]
    backend: H::Backend,
    store: Lib<H>,
    dev: EndpointId,
    clock: ManualClock,
}

fn dev(n: u8) -> EndpointId {
    EndpointId([n; 32])
}
fn hash(n: u8) -> Blake3Hash {
    Blake3Hash([n; 32])
}
fn hlc(ms: u64, d: u8) -> Hlc {
    Hlc {
        physical_ms: ms,
        counter: 0,
        device: dev(d),
    }
}
const ROOT: RootPublicKey = RootPublicKey([42; 32]);

fn blob(n: u8) -> BlobRef {
    BlobRef {
        hash: hash(n),
        size: 100 + n as u64,
        mime: "image/jpeg".into(),
    }
}

/// A photo whose id is `n` and whose registers were written at HLC `(1, importer)`.
fn photo(n: u8, taken_ms: i64, importer: u8) -> PhotoRecord {
    PhotoRecord {
        id: hash(n),
        original: blob(n),
        preview: blob(n.wrapping_add(100)),
        thumb: blob(n.wrapping_add(200)),
        taken_at: Timestamp(taken_ms),
        exif: ExifSummary::default(),
        imported_by: dev(importer),
        favorite: Lww::new(false, hlc(1, importer)),
        deleted_at: Lww::new(None, hlc(1, importer)),
    }
}

fn album_meta(n: u8, name: &str) -> AlbumMeta {
    AlbumMeta {
        id: AlbumId([n; 16]),
        name: Lww::new(name.to_string(), hlc(1, 1)),
        cover: Lww::new(None, hlc(1, 1)),
        items: OrSet::new(),
    }
}

/// One node, library created.
async fn solo<H: Harness>(h: &H, d: u8, start_ms: u64) -> (Node<H>, LibraryCapability) {
    let clock = ManualClock::new(start_ms);
    let backend = h.backend(dev(d), clock.clone()).await;
    let (store, cap) = backend.create_library(&ROOT).await.expect("create_library");
    (
        Node {
            backend,
            store,
            dev: dev(d),
            clock,
        },
        cap,
    )
}

/// Two nodes (devices 1 and 2) replicating one library; clocks start at 100 and 200.
async fn pair<H: Harness>(h: &H) -> (Node<H>, Node<H>) {
    let (a, cap) = solo(h, 1, 100).await;
    let clock = ManualClock::new(200);
    let backend = h.backend(dev(2), clock.clone()).await;
    let store = backend.open_library(&cap).await.expect("open_library");
    (
        a,
        Node {
            backend,
            store,
            dev: dev(2),
            clock,
        },
    )
}

async fn sync_both<H: Harness>(h: &H, a: &Node<H>, b: &Node<H>) {
    a.store
        .sync_with(&h.peer_addr(b.dev))
        .await
        .expect("a syncs with b");
    b.store
        .sync_with(&h.peer_addr(a.dev))
        .await
        .expect("b syncs with a");
}

async fn timeline<L: LibraryStore>(s: &L) -> Vec<PhotoId> {
    s.photos_by_time(TimeRange::ALL, Page::all())
        .await
        .unwrap()
        .into_iter()
        .map(|p| p.id)
        .collect()
}

// ---------- scenarios ----------

async fn upsert_is_idempotent_and_merges<H: Harness>(h: &H) {
    let (n, _) = solo(h, 1, 100).await;
    let s = &n.store;
    let p = photo(1, 1000, 1);
    s.upsert_photo(p.clone()).await.unwrap();
    s.upsert_photo(p.clone()).await.unwrap();
    assert_eq!(s.photo(&p.id).await.unwrap(), Some(p.clone()));
    assert_eq!(timeline(s).await, vec![p.id]);

    // A newer favorite write merges in; an older one is ignored.
    let mut newer = p.clone();
    newer.favorite = Lww::new(true, hlc(50, 1));
    s.upsert_photo(newer.clone()).await.unwrap();
    let mut older = p.clone();
    older.favorite = Lww::new(false, hlc(10, 1));
    s.upsert_photo(older).await.unwrap();
    assert!(s.photo(&p.id).await.unwrap().unwrap().favorite.value);
    assert_eq!(s.photo(&hash(99)).await.unwrap(), None);
}

async fn paging_by_time<H: Harness>(h: &H) {
    let (n, _) = solo(h, 1, 100).await;
    let s = &n.store;
    // taken_at: 5@500, 3@300, 4@300 (tie → id asc), 1@100, 2@200
    for (id, t) in [(1, 100), (2, 200), (3, 300), (4, 300), (5, 500)] {
        s.upsert_photo(photo(id, t, 1)).await.unwrap();
    }
    let expected = vec![hash(5), hash(3), hash(4), hash(2), hash(1)];
    assert_eq!(timeline(s).await, expected);

    // Page two at a time using cursors.
    let mut seen = vec![];
    let mut page = Page::first(2);
    loop {
        let batch = s.photos_by_time(TimeRange::ALL, page).await.unwrap();
        if batch.is_empty() {
            break;
        }
        assert!(batch.len() <= 2);
        page.after = Some(Cursor::after_photo(batch.last().unwrap()));
        seen.extend(batch.into_iter().map(|p| p.id));
    }
    assert_eq!(seen, expected, "paging neither repeats nor skips");

    // Half-open range [200, 500).
    let range = TimeRange {
        start: Some(Timestamp(200)),
        end: Some(Timestamp(500)),
    };
    let ids: Vec<_> = s
        .photos_by_time(range, Page::all())
        .await
        .unwrap()
        .into_iter()
        .map(|p| p.id)
        .collect();
    assert_eq!(ids, vec![hash(3), hash(4), hash(2)]);

    // Deleted photos leave the timeline.
    s.mark_deleted(&hash(5)).await.unwrap();
    assert_eq!(timeline(s).await, vec![hash(3), hash(4), hash(2), hash(1)]);
}

async fn favorite_is_last_writer_wins<H: Harness>(h: &H) {
    let (n, _) = solo(h, 1, 100).await;
    let s = &n.store;
    s.upsert_photo(photo(1, 10, 1)).await.unwrap();
    s.set_favorite(&hash(1), true).await.unwrap();
    assert!(s.photo(&hash(1)).await.unwrap().unwrap().favorite.value);
    s.set_favorite(&hash(1), false).await.unwrap();
    assert!(!s.photo(&hash(1)).await.unwrap().unwrap().favorite.value);
    // Same-millisecond writes still order (HLC counter).
    s.set_favorite(&hash(1), true).await.unwrap();
    s.set_favorite(&hash(1), false).await.unwrap();
    s.set_favorite(&hash(1), true).await.unwrap();
    assert!(s.photo(&hash(1)).await.unwrap().unwrap().favorite.value);
    assert_eq!(
        s.set_favorite(&hash(77), true).await,
        Err(StoreError::NotFound)
    );
}

async fn delete_restore_purge<H: Harness>(h: &H) {
    let (n, _) = solo(h, 1, 1_000).await;
    let s = &n.store;
    s.upsert_photo(photo(1, 10, 1)).await.unwrap();
    s.upsert_photo(photo(2, 20, 1)).await.unwrap();

    s.mark_deleted(&hash(1)).await.unwrap();
    s.mark_deleted(&hash(1)).await.unwrap(); // idempotent in effect
    let rec = s.photo(&hash(1)).await.unwrap().unwrap();
    assert!(rec.is_deleted());
    assert_eq!(rec.deleted_at.value, Some(Timestamp(1_000)));
    assert_eq!(timeline(s).await, vec![hash(2)]);
    let deleted: Vec<_> = s
        .deleted_photos()
        .await
        .unwrap()
        .into_iter()
        .map(|p| p.id)
        .collect();
    assert_eq!(deleted, vec![hash(1)]);

    // Restore brings it back; delete again.
    n.clock.advance(10);
    s.restore_photo(&hash(1)).await.unwrap();
    assert!(!s.photo(&hash(1)).await.unwrap().unwrap().is_deleted());
    assert_eq!(timeline(s).await, vec![hash(2), hash(1)]);
    n.clock.advance(10);
    s.mark_deleted(&hash(1)).await.unwrap();
    let deleted_at = s
        .photo(&hash(1))
        .await
        .unwrap()
        .unwrap()
        .deleted_at
        .value
        .unwrap();

    // Nothing is purged before the retention window ends; live photos are never purged.
    let almost = Timestamp(deleted_at.0 + RETENTION_MS - 1);
    assert_eq!(s.purge_expired(almost).await.unwrap(), vec![]);
    let due = Timestamp(deleted_at.0 + RETENTION_MS);
    n.clock.set(due.0 as u64);
    assert_eq!(s.purge_expired(due).await.unwrap(), vec![hash(1)]);
    assert_eq!(s.photo(&hash(1)).await.unwrap(), None);
    assert!(s.photo(&hash(2)).await.unwrap().is_some());
    assert_eq!(
        s.purge_expired(due).await.unwrap(),
        vec![],
        "second purge is a no-op"
    );

    // A stale replay of the purged record cannot resurrect it...
    let mut stale = photo(1, 10, 1);
    stale.deleted_at = Lww::new(Some(deleted_at), hlc(due.0 as u64 - 5, 1));
    s.upsert_photo(stale).await.unwrap();
    assert_eq!(s.photo(&hash(1)).await.unwrap(), None);
    // ...but importing the same file again later is a fresh record and survives.
    let mut reimport = photo(1, 10, 1);
    let after = due.0 as u64 + 10;
    reimport.favorite = Lww::new(false, hlc(after, 1));
    reimport.deleted_at = Lww::new(None, hlc(after, 1));
    s.upsert_photo(reimport).await.unwrap();
    assert!(!s.photo(&hash(1)).await.unwrap().unwrap().is_deleted());
}

async fn album_add_wins<H: Harness>(h: &H) {
    let (a, b) = pair(h).await;
    a.store.upsert_album(album_meta(1, "Trip")).await.unwrap();
    for id in [1, 2, 3] {
        a.store.upsert_photo(photo(id, id as i64, 1)).await.unwrap();
    }
    assert_eq!(
        a.store.album_add(&AlbumId([9; 16]), &[hash(1)]).await,
        Err(StoreError::NotFound)
    );
    let album = AlbumId([1; 16]);
    a.store
        .album_add(&album, &[hash(1), hash(2), hash(3)])
        .await
        .unwrap();
    a.store.album_add(&album, &[hash(1)]).await.unwrap(); // re-add is harmless
    a.store.album_remove(&album, &[hash(3)]).await.unwrap();
    a.store
        .album_remove(&album, &[hash(3), hash(99)])
        .await
        .unwrap(); // absent: no-op
    assert_eq!(
        a.store.album_items(&album, Page::all()).await.unwrap(),
        vec![hash(1), hash(2)]
    );
    let summaries = a.store.albums().await.unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(
        (summaries[0].name.as_str(), summaries[0].item_count),
        ("Trip", 2)
    );

    // Paging by id.
    let first = a.store.album_items(&album, Page::first(1)).await.unwrap();
    assert_eq!(first, vec![hash(1)]);
    let next = Page {
        after: Some(Cursor::after_id(hash(1))),
        limit: 1,
    };
    assert_eq!(
        a.store.album_items(&album, next).await.unwrap(),
        vec![hash(2)]
    );

    // Concurrent: A removes photo 2 while B (which has it) adds it again → add wins.
    sync_both(h, &a, &b).await;
    assert_eq!(
        b.store.album_items(&album, Page::all()).await.unwrap(),
        vec![hash(1), hash(2)]
    );
    a.store.album_remove(&album, &[hash(2)]).await.unwrap();
    b.store.album_add(&album, &[hash(2)]).await.unwrap();
    sync_both(h, &a, &b).await;
    for n in [&a, &b] {
        assert_eq!(
            n.store.album_items(&album, Page::all()).await.unwrap(),
            vec![hash(1), hash(2)]
        );
    }
    // A removal that has observed every add really removes, everywhere.
    a.store.album_remove(&album, &[hash(2)]).await.unwrap();
    sync_both(h, &a, &b).await;
    for n in [&a, &b] {
        assert_eq!(
            n.store.album_items(&album, Page::all()).await.unwrap(),
            vec![hash(1)]
        );
    }
}

async fn holdings_single_writer_and_at_risk<H: Harness>(h: &H) {
    let (a, b) = pair(h).await;
    a.store.upsert_photo(photo(1, 1, 1)).await.unwrap();
    a.store.upsert_photo(photo(2, 2, 1)).await.unwrap();
    let put = |ms, added: Vec<(Blake3Hash, BlobKind)>, removed: Vec<Blake3Hash>, d| HoldingsDelta {
        hlc: hlc(ms, d),
        added,
        removed,
    };

    // Single writer: only my own device.
    let err = a
        .store
        .put_holdings(
            dev(2),
            put(10, vec![(hash(1), BlobKind::Original)], vec![], 2),
        )
        .await;
    assert!(matches!(err, Err(StoreError::Invalid(_))), "got {err:?}");

    let delta = put(
        10,
        vec![(hash(1), BlobKind::Original), (hash(2), BlobKind::Original)],
        vec![],
        1,
    );
    a.store.put_holdings(dev(1), delta.clone()).await.unwrap();
    a.store.put_holdings(dev(1), delta).await.unwrap(); // replay is idempotent
    let holders = a.store.holders(&hash(1)).await.unwrap();
    assert_eq!(holders.len(), 1);
    assert_eq!(
        (holders[0].device, holders[0].kind, holders[0].since),
        (dev(1), BlobKind::Original, hlc(10, 1))
    );

    // Everything has 1 copy: at risk below 2, safe at 1.
    assert_eq!(
        a.store.at_risk(2, Page::all()).await.unwrap(),
        vec![hash(1), hash(2)]
    );
    assert_eq!(a.store.at_risk(1, Page::all()).await.unwrap(), vec![]);
    let page = Page {
        after: Some(Cursor::after_id(hash(1))),
        limit: 10,
    };
    assert_eq!(a.store.at_risk(2, page).await.unwrap(), vec![hash(2)]);

    // B holds photo 1's original too (visible after sync); 2 is still at risk.
    sync_both(h, &a, &b).await;
    b.store
        .put_holdings(
            dev(2),
            put(
                20,
                vec![(hash(1), BlobKind::Original), (hash(2), BlobKind::Thumb)],
                vec![],
                2,
            ),
        )
        .await
        .unwrap();
    sync_both(h, &a, &b).await;
    assert_eq!(a.store.holders(&hash(1)).await.unwrap().len(), 2);
    assert_eq!(
        a.store.at_risk(2, Page::all()).await.unwrap(),
        vec![hash(2)],
        "a thumb is not a copy"
    );
    assert_eq!(
        b.store.at_risk(2, Page::all()).await.unwrap(),
        vec![hash(2)]
    );

    // Removal wins over the earlier add; a stale (older) re-add does not bring it back.
    a.store
        .put_holdings(dev(1), put(30, vec![], vec![hash(1)], 1))
        .await
        .unwrap();
    a.store
        .put_holdings(
            dev(1),
            put(5, vec![(hash(1), BlobKind::Original)], vec![], 1),
        )
        .await
        .unwrap();
    let holders = a.store.holders(&hash(1)).await.unwrap();
    assert_eq!(
        holders.iter().map(|x| x.device).collect::<Vec<_>>(),
        vec![dev(2)]
    );

    // Deleted photos are not "at risk".
    a.store.mark_deleted(&hash(2)).await.unwrap();
    assert_eq!(
        a.store.at_risk(5, Page::all()).await.unwrap(),
        vec![hash(1)]
    );
}

/// Wait for the next change, skipping nothing: the suite asserts the exact sequence.
async fn next<S: futures::Stream<Item = Change> + Unpin>(s: &mut S) -> Change {
    s.next().await.expect("change stream ended")
}

async fn change_stream_events<H: Harness>(h: &H) {
    let (a, b) = pair(h).await;
    let mut stream = a.store.subscribe();
    let mut remote = b.store.subscribe();

    a.store.upsert_photo(photo(1, 1, 1)).await.unwrap();
    assert_eq!(next(&mut stream).await, Change::PhotoUpserted(hash(1)));
    a.store.mark_deleted(&hash(1)).await.unwrap();
    assert_eq!(next(&mut stream).await, Change::PhotoDeleted(hash(1)));
    a.store.upsert_album(album_meta(1, "X")).await.unwrap();
    assert_eq!(
        next(&mut stream).await,
        Change::AlbumChanged(AlbumId([1; 16]))
    );
    a.store
        .apply_device_change(SignedDeviceChange::Device(device_cert(2, 1)))
        .await
        .unwrap();
    assert_eq!(next(&mut stream).await, Change::DeviceListChanged);
    a.store
        .put_holdings(
            dev(1),
            HoldingsDelta {
                hlc: hlc(5, 1),
                added: vec![(hash(1), BlobKind::Thumb)],
                removed: vec![],
            },
        )
        .await
        .unwrap();
    assert_eq!(
        next(&mut stream).await,
        Change::HoldingsChanged { device: dev(1) }
    );

    // A new subscriber sees only later changes; remote changes arrive via sync.
    let mut late = a.store.subscribe();
    a.store.upsert_photo(photo(2, 2, 1)).await.unwrap();
    assert_eq!(next(&mut late).await, Change::PhotoUpserted(hash(2)));
    sync_both(h, &a, &b).await;
    // The order in which a sync reports what changed is up to the backend: look through the
    // first few events (the library holds a handful of records) for the one we caused.
    let mut got = vec![];
    for _ in 0..8 {
        let change = next(&mut remote).await;
        got.push(change.clone());
        if change == Change::PhotoUpserted(hash(2)) {
            return;
        }
    }
    panic!(
        "no PhotoUpserted({}) among the first events after sync: {got:?}",
        hash(2)
    );
}

async fn concurrent_edits_converge<H: Harness>(h: &H) {
    let (a, b) = pair(h).await;
    a.store.upsert_photo(photo(1, 10, 1)).await.unwrap();
    sync_both(h, &a, &b).await;
    assert!(b.store.photo(&hash(1)).await.unwrap().is_some());

    // Offline edits: A favorites at t=300, B unfavorites at t=400 (later → wins everywhere).
    a.clock.set(300);
    a.store.set_favorite(&hash(1), true).await.unwrap();
    b.clock.set(400);
    b.store.set_favorite(&hash(1), false).await.unwrap();
    b.store.set_favorite(&hash(1), true).await.unwrap(); // B's latest intent: favorite

    // Disjoint adds on both sides.
    a.store.upsert_photo(photo(2, 20, 1)).await.unwrap();
    b.store.upsert_photo(photo(3, 30, 2)).await.unwrap();
    // Same photo imported on both sides converges to one entry (content addressing).
    a.store.upsert_photo(photo(4, 40, 1)).await.unwrap();
    b.store.upsert_photo(photo(4, 40, 2)).await.unwrap();

    sync_both(h, &a, &b).await;
    sync_both(h, &a, &b).await; // syncing again changes nothing
    let (pa, pb) = (timeline(&a.store).await, timeline(&b.store).await);
    assert_eq!(pa, pb);
    assert_eq!(pa, vec![hash(4), hash(3), hash(2), hash(1)]);
    for id in [1, 2, 3, 4] {
        assert_eq!(
            a.store.photo(&hash(id)).await.unwrap(),
            b.store.photo(&hash(id)).await.unwrap(),
            "photo {id} converged"
        );
    }
    assert!(
        a.store
            .photo(&hash(1))
            .await
            .unwrap()
            .unwrap()
            .favorite
            .value
    );

    // Offline B edits a *later* favorite than A's even though A syncs last: HLC, not arrival, decides.
    a.clock.set(1_000);
    a.store.set_favorite(&hash(1), false).await.unwrap();
    b.clock.set(500);
    b.store.set_favorite(&hash(1), true).await.unwrap();
    sync_both(h, &b, &a).await;
    assert!(
        !b.store
            .photo(&hash(1))
            .await
            .unwrap()
            .unwrap()
            .favorite
            .value
    );
    assert!(
        !a.store
            .photo(&hash(1))
            .await
            .unwrap()
            .unwrap()
            .favorite
            .value
    );

    // Purge replicates.
    a.clock.set(2_000);
    a.store.mark_deleted(&hash(3)).await.unwrap();
    sync_both(h, &a, &b).await;
    assert!(b.store.photo(&hash(3)).await.unwrap().unwrap().is_deleted());
    let due = Timestamp(2_000 + RETENTION_MS);
    a.clock.set(due.0 as u64);
    assert_eq!(a.store.purge_expired(due).await.unwrap(), vec![hash(3)]);
    sync_both(h, &a, &b).await;
    assert_eq!(b.store.photo(&hash(3)).await.unwrap(), None);
    assert_eq!(
        b.store.purge_expired(due).await.unwrap(),
        vec![],
        "already purged by sync"
    );
}

async fn delete_races_favorite<H: Harness>(h: &H) {
    let (a, b) = pair(h).await;
    a.store.upsert_photo(photo(1, 10, 1)).await.unwrap();
    sync_both(h, &a, &b).await;
    a.clock.set(500);
    a.store.mark_deleted(&hash(1)).await.unwrap();
    b.clock.set(900); // B's edit is HLC-later than the delete and concurrent with it
    b.store.set_favorite(&hash(1), true).await.unwrap();
    sync_both(h, &a, &b).await;
    for n in [&a, &b] {
        let rec = n.store.photo(&hash(1)).await.unwrap().unwrap();
        assert!(rec.is_deleted(), "an edit never undeletes");
        assert!(rec.favorite.value, "and the edit is not lost");
        assert_eq!(timeline(&n.store).await, vec![]);
    }
    // Restore is an explicit later write and wins.
    b.clock.set(1_000);
    b.store.restore_photo(&hash(1)).await.unwrap();
    sync_both(h, &b, &a).await;
    for n in [&a, &b] {
        assert!(!n.store.photo(&hash(1)).await.unwrap().unwrap().is_deleted());
    }
}

fn device_cert(device: u8, hlc_ms: u64) -> DeviceCert {
    DeviceCert {
        device: dev(device),
        role: Role::Keeper,
        name: format!("device-{device}"),
        added_by: dev(1),
        hlc: hlc(hlc_ms, 1),
        revoked: false,
        signature: vec![1, 2, 3],
    }
}

async fn device_list_is_signed_and_order_independent<H: Harness>(h: &H) {
    let (a, b) = pair(h).await;
    let mut unsigned = device_cert(3, 5);
    unsigned.signature.clear();
    assert_eq!(
        a.store
            .apply_device_change(SignedDeviceChange::Device(unsigned))
            .await,
        Err(StoreError::BadSignature)
    );
    assert_eq!(
        a.store.device_list().await.unwrap(),
        SignedDeviceList::default()
    );

    // Revocation (later HLC) wins even if it is applied before the original cert.
    let cert = device_cert(2, 10);
    let mut revoked = device_cert(2, 20);
    revoked.revoked = true;
    a.store
        .apply_device_change(SignedDeviceChange::Device(revoked.clone()))
        .await
        .unwrap();
    a.store
        .apply_device_change(SignedDeviceChange::Device(cert.clone()))
        .await
        .unwrap();
    a.store
        .apply_device_change(SignedDeviceChange::Device(cert.clone()))
        .await
        .unwrap(); // idempotent
    b.store
        .apply_device_change(SignedDeviceChange::Device(cert))
        .await
        .unwrap();
    b.store
        .apply_device_change(SignedDeviceChange::Device(device_cert(1, 3)))
        .await
        .unwrap();
    let admin = AdminCert {
        admin: dev(1),
        hlc: hlc(1, 1),
        signature: vec![9],
    };
    b.store
        .apply_device_change(SignedDeviceChange::Admin(admin.clone()))
        .await
        .unwrap();

    sync_both(h, &a, &b).await;
    let (la, lb) = (
        a.store.device_list().await.unwrap(),
        b.store.device_list().await.unwrap(),
    );
    assert_eq!(la, lb);
    assert_eq!(
        la.devices.iter().map(|d| d.device).collect::<Vec<_>>(),
        vec![dev(1), dev(2)]
    );
    assert!(la.devices[1].revoked && la.devices[1].hlc == revoked.hlc);
    assert_eq!(la.admins, vec![admin]);
}

async fn library_settings_are_last_writer_wins<H: Harness>(h: &H) {
    let (a, b) = pair(h).await;
    assert_eq!(a.store.setting("redundancy.target").await.unwrap(), None);
    a.store
        .put_setting("redundancy.target", vec![3])
        .await
        .unwrap();
    assert_eq!(
        a.store.setting("redundancy.target").await.unwrap(),
        Some(vec![3])
    );
    sync_both(h, &a, &b).await;
    assert_eq!(
        b.store.setting("redundancy.target").await.unwrap(),
        Some(vec![3])
    );

    // Concurrent offline writes: the later HLC wins on both sides, whoever syncs first.
    a.clock.set(1_000);
    a.store
        .put_setting("redundancy.target", vec![4])
        .await
        .unwrap();
    b.clock.set(2_000);
    b.store
        .put_setting("redundancy.target", vec![5])
        .await
        .unwrap();
    b.store
        .put_setting("other.key", b"x".to_vec())
        .await
        .unwrap();
    sync_both(h, &a, &b).await;
    for n in [&a, &b] {
        assert_eq!(
            n.store.setting("redundancy.target").await.unwrap(),
            Some(vec![5])
        );
        assert_eq!(
            n.store.setting("other.key").await.unwrap(),
            Some(b"x".to_vec())
        );
    }
    // Keys are restricted to a safe alphabet; anything else is rejected before it is stored.
    for bad in ["", "Has Space", "slash/inside", "ünïcode", &"k".repeat(200)] {
        assert!(
            matches!(
                a.store.put_setting(bad, vec![1]).await,
                Err(StoreError::Invalid(_))
            ),
            "{bad:?}"
        );
    }
}

async fn read_only_capability<H: Harness>(h: &H) {
    let (a, cap) = solo(h, 1, 100).await;
    assert_eq!(a.store.capability(), Capability::Write);
    a.store.upsert_photo(photo(1, 1, 1)).await.unwrap();

    let clock = ManualClock::new(300);
    let viewer = h.backend(dev(5), clock).await;
    let ro = viewer
        .open_library(&a.backend.downgrade(&cap))
        .await
        .unwrap();
    assert_eq!(ro.capability(), Capability::Read);
    assert_eq!(
        ro.upsert_photo(photo(2, 2, 5)).await,
        Err(StoreError::ReadOnly)
    );
    assert_eq!(
        ro.set_favorite(&hash(1), true).await,
        Err(StoreError::ReadOnly)
    );
    assert_eq!(
        ro.put_setting("redundancy.target", vec![1]).await,
        Err(StoreError::ReadOnly)
    );
    assert_eq!(ro.mark_deleted(&hash(1)).await, Err(StoreError::ReadOnly));
    assert_eq!(
        ro.upsert_album(album_meta(1, "x")).await,
        Err(StoreError::ReadOnly)
    );
    assert_eq!(
        ro.put_holdings(
            dev(5),
            HoldingsDelta {
                hlc: hlc(1, 5),
                added: vec![],
                removed: vec![]
            }
        )
        .await,
        Err(StoreError::ReadOnly)
    );
    // It can still read what syncs in.
    ro.sync_with(&h.peer_addr(a.dev)).await.unwrap();
    assert_eq!(timeline(&ro).await, vec![hash(1)]);
}

async fn shared_album_flow<H: Harness>(h: &H) {
    let owner_clock = ManualClock::new(1_000);
    let owner = h.backend(dev(1), owner_clock.clone()).await;
    let meta = SharedAlbumMeta {
        id: AlbumId([7; 16]),
        name: "Wedding".into(),
        cover: None,
        created_by: dev(1),
    };
    let album = owner.create_album(meta.clone()).await.unwrap();
    assert_eq!(album.meta().await.unwrap(), meta);
    let members = album.members().await.unwrap();
    assert_eq!(
        members.iter().map(|m| (m.key, m.role)).collect::<Vec<_>>(),
        vec![(dev(1), MemberRole::Owner)]
    );

    let item = |n: u8, added_ms: i64| SharedItem {
        id: hash(n),
        original: blob(n),
        shared_hash: None,
        preview: blob(n.wrapping_add(100)),
        thumb: blob(n.wrapping_add(200)),
        added_by: dev(1),
        added_at: Timestamp(added_ms),
        taken_at: Timestamp(0),
    };
    album
        .add_items(vec![item(1, 10), item(2, 20)])
        .await
        .unwrap();
    album
        .add_items(vec![item(2, 20), item(3, 30)])
        .await
        .unwrap(); // 2 again: idempotent
    let ids = |v: Vec<SharedItem>| v.into_iter().map(|i| i.id).collect::<Vec<_>>();
    assert_eq!(
        ids(album.items(Page::all()).await.unwrap()),
        vec![hash(3), hash(2), hash(1)]
    );
    assert_eq!(
        ids(album.items(Page::first(2)).await.unwrap()),
        vec![hash(3), hash(2)]
    );
    let page = Page {
        after: Some(Cursor::after_id(hash(2))),
        limit: 5,
    };
    assert_eq!(ids(album.items(page).await.unwrap()), vec![hash(1)]);
    let events = album.events(None).await.unwrap();
    let added: u32 = events
        .iter()
        .map(|e| match e.kind {
            AlbumEventKind::ItemsAdded { count } => count,
            _ => 0,
        })
        .sum();
    assert_eq!(added, 3, "only genuinely new items are logged");

    // Invites: expired tickets are refused.
    let old = album
        .invite(MemberRole::Contributor, Expiry::At(Timestamp(500)))
        .await
        .unwrap();
    let guest_clock = ManualClock::new(2_000);
    let guest = h.backend(dev(2), guest_clock.clone()).await;
    assert_eq!(
        guest.join_album(&old).await.err(),
        Some(StoreError::Expired)
    );

    // A valid ticket joins, receives state, contributes, and the owner sees it after a sync.
    let ticket = album
        .invite(MemberRole::Contributor, Expiry::Never)
        .await
        .unwrap();
    let joined = guest.join_album(&ticket).await.unwrap();
    assert_eq!(joined.meta().await.unwrap().name, "Wedding");
    assert_eq!(ids(joined.items(Page::all()).await.unwrap()).len(), 3);
    let mut mine = item(9, 2_100);
    mine.added_by = dev(2);
    joined.add_items(vec![mine]).await.unwrap();
    joined.sync_with(&h.peer_addr(dev(1))).await.unwrap();
    album.sync_with(&h.peer_addr(dev(2))).await.unwrap();
    assert_eq!(
        ids(album.items(Page::first(1)).await.unwrap()),
        vec![hash(9)]
    );
    let roles: Vec<_> = album
        .members()
        .await
        .unwrap()
        .into_iter()
        .map(|m| (m.key, m.role))
        .collect();
    assert_eq!(
        roles,
        vec![
            (dev(1), MemberRole::Owner),
            (dev(2), MemberRole::Contributor)
        ]
    );
    let kinds: Vec<_> = album
        .events(None)
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.kind)
        .collect();
    assert!(kinds.contains(&AlbumEventKind::MemberJoined));

    // A viewer can read but not add or invite.
    let viewer_ticket = album
        .invite(MemberRole::Viewer, Expiry::Never)
        .await
        .unwrap();
    let viewer = h.backend(dev(3), ManualClock::new(3_000)).await;
    let v = viewer.join_album(&viewer_ticket).await.unwrap();
    assert_eq!(
        v.add_items(vec![item(5, 1)]).await,
        Err(StoreError::ReadOnly)
    );
    assert_eq!(
        v.invite(MemberRole::Viewer, Expiry::Never).await.err(),
        Some(StoreError::ReadOnly)
    );
    // Contributors cannot invite either.
    assert_eq!(
        joined.invite(MemberRole::Viewer, Expiry::Never).await.err(),
        Some(StoreError::ReadOnly)
    );
}
