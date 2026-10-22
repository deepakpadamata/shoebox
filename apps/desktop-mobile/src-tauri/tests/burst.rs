//! M4-08/09: what a background wake-up does with a time budget.

use desktop_mobile::types::AppEvent;
use desktop_mobile::{AppCore, AppOptions};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

fn sample(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../fixtures/samples")
        .join(name)
        .display()
        .to_string()
}

async fn pair(a: &Arc<AppCore>, b: &Arc<AppCore>, role: &str, name: &str) {
    let start = a.pair_start(role).await.unwrap();
    for core in [a, b] {
        let mut ev = core.events();
        let core = core.clone();
        tokio::spawn(async move {
            while let Ok(e) = ev.recv().await {
                if matches!(e, AppEvent::PairingCode { .. }) {
                    let _ = core.confirm_pairing(true);
                    return;
                }
            }
        });
    }
    b.join(&start.ticket, name).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_burst_syncs_and_waits_until_the_keeper_has_taken_the_photos() {
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let phone = AppCore::open(da.path(), AppOptions::local_only())
        .await
        .unwrap();
    phone.create_library("Pixel", "phone").await.unwrap();
    phone
        .import_paths(&[
            sample("photo-gps.jpg"),
            sample("IMG_0001.jpg"),
            sample("screenshot.png"),
        ])
        .await
        .unwrap();
    let keeper = AppCore::open(db.path(), AppOptions::local_only())
        .await
        .unwrap();
    pair(&phone, &keeper, "keeper", "Pi").await;

    assert_eq!(phone.counts().await.unwrap().at_risk, 3, "one copy each");
    let report = phone.sync_burst(Duration::from_secs(60)).await.unwrap();
    assert_eq!(
        (report.peers_total, report.peers_reached),
        (1, 1),
        "{report:?}"
    );
    assert_eq!(
        (report.at_risk_before, report.at_risk_after),
        (3, 0),
        "{report:?}"
    );
    assert!(
        report.finished && !report.budget_exhausted && report.reason.is_none(),
        "{report:?}"
    );
    assert!(report.elapsed_ms < 60_000.0);
    phone.shutdown().await.unwrap();
    keeper.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_burst_with_nobody_reachable_says_so_quickly() {
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let phone = AppCore::open(da.path(), AppOptions::local_only())
        .await
        .unwrap();
    phone.create_library("Pixel", "phone").await.unwrap();
    phone
        .import_paths(&[sample("photo-gps.jpg")])
        .await
        .unwrap();
    let keeper = AppCore::open(db.path(), AppOptions::local_only())
        .await
        .unwrap();
    pair(&phone, &keeper, "keeper", "Pi").await;
    keeper.shutdown().await.unwrap(); // the keeper goes away

    let started = std::time::Instant::now();
    let report = phone.sync_burst(Duration::from_secs(30)).await.unwrap();
    assert_eq!(report.peers_reached, 0, "{report:?}");
    assert!(!report.finished && !report.budget_exhausted, "{report:?}");
    assert_eq!(
        report.reason.as_deref(),
        Some("none of your devices could be reached")
    );
    assert!(
        started.elapsed() < Duration::from_secs(25),
        "gives up within the peer timeout"
    );
    phone.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_burst_stops_at_its_deadline_when_the_work_does_not_finish() {
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let phone = AppCore::open(da.path(), AppOptions::local_only())
        .await
        .unwrap();
    phone.create_library("Pixel", "phone").await.unwrap();
    phone
        .import_paths(&[sample("photo-gps.jpg")])
        .await
        .unwrap();
    // The other device is a viewer: it never takes originals, so the photo stays at risk.
    let viewer = AppCore::open(db.path(), AppOptions::local_only())
        .await
        .unwrap();
    pair(&phone, &viewer, "viewer", "Tablet").await;

    let report = phone.sync_burst(Duration::from_secs(3)).await.unwrap();
    assert_eq!(report.peers_reached, 1);
    assert!(!report.finished && report.budget_exhausted, "{report:?}");
    assert_eq!(report.at_risk_after, 1);
    assert!(
        report.elapsed_ms >= 2_500.0 && report.elapsed_ms < 8_000.0,
        "{report:?}"
    );
    phone.shutdown().await.unwrap();
    viewer.shutdown().await.unwrap();
}
