//! Assertions over replicated state.

use shoebox_core::ids::{Blake3Hash, EndpointId};
use shoebox_core::store::{LibraryStore, Page, TimeRange};

/// Everything observable about a library that must be equal on converged replicas.
async fn snapshot<L: LibraryStore>(store: &L) -> String {
    let live = store
        .photos_by_time(TimeRange::ALL, Page::all())
        .await
        .expect("list photos");
    let mut deleted = store.deleted_photos().await.expect("list deleted");
    deleted.sort_by_key(|p| p.id);
    let albums = store.albums().await.expect("list albums");
    let devices = store.device_list().await.expect("device list");
    format!("live={live:#?}\ndeleted={deleted:#?}\nalbums={albums:#?}\ndevices={devices:#?}")
}

/// Panics unless every store exposes identical photos (live and deleted), albums and devices.
pub async fn assert_converged<L: LibraryStore>(stores: &[&L]) {
    let first = match stores.first() {
        Some(s) => snapshot(*s).await,
        None => return,
    };
    for (i, store) in stores.iter().enumerate().skip(1) {
        let other = snapshot(*store).await;
        assert!(
            first == other,
            "replica {i} diverged from replica 0\n--- 0 ---\n{first}\n--- {i} ---\n{other}"
        );
    }
}

/// Panics unless exactly `expected` devices (any order) currently hold `blob`.
pub async fn assert_holders<L: LibraryStore>(
    store: &L,
    blob: &Blake3Hash,
    expected: &[EndpointId],
) {
    let mut actual: Vec<EndpointId> = store
        .holders(blob)
        .await
        .expect("holders")
        .into_iter()
        .map(|h| h.device)
        .collect();
    let mut want = expected.to_vec();
    actual.sort();
    want.sort();
    assert_eq!(actual, want, "holders of {blob}");
}
