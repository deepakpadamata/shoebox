//! M4-05/06: camera-roll backup with a fake photo source: incremental import, sync conditions,
//! failures, restarts.

use desktop_mobile::photos_source::{Conditions, FakePhotoSource, Permission};
use desktop_mobile::types::BackupPolicy;
use desktop_mobile::{pause_reason, AppCore, AppOptions};
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn corpus(dir: &Path) -> Vec<PathBuf> {
    let out = dir.join("corpus");
    xtask::corpus::generate(&xtask::corpus::Options {
        preset: xtask::corpus::Preset::Small,
        out_dir: out.clone(),
        seed: 11,
        metadata_only: None,
    })
    .unwrap();
    fn walk(d: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else {
                out.push(p);
            }
        }
    }
    let mut files = vec![];
    walk(&out.join("photos"), &mut files);
    files.sort();
    files
}

fn policy(wifi: bool, charging: bool) -> BackupPolicy {
    BackupPolicy {
        enabled: true,
        wifi_only: wifi,
        charging_only: charging,
        min_battery: 20,
    }
}

struct Rig {
    _dir: tempfile::TempDir,
    core: Arc<AppCore>,
    source: Arc<FakePhotoSource>,
    files: Vec<PathBuf>,
}

async fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let files = corpus(dir.path());
    let core = AppCore::open(dir.path().join("app"), AppOptions::local_only())
        .await
        .unwrap();
    core.create_library("Pixel", "phone").await.unwrap();
    let source = Arc::new(FakePhotoSource::new(dir.path().join("scratch")));
    core.set_photo_source(source.clone());
    Rig {
        _dir: dir,
        core,
        source,
        files,
    }
}

async fn photos(core: &Arc<AppCore>) -> usize {
    core.library_page(None, 2000).await.unwrap().items.len()
}

#[test]
fn the_pause_rules() {
    let c = |wifi, charging, battery_pct| Conditions {
        on_wifi: wifi,
        charging,
        battery_pct,
    };
    let p = |w, ch| BackupPolicy {
        enabled: true,
        wifi_only: w,
        charging_only: ch,
        min_battery: 20,
    };
    assert_eq!(pause_reason(&p(true, false), &c(true, false, 80)), None);
    assert_eq!(
        pause_reason(&p(true, false), &c(false, true, 80)),
        Some("Waiting for Wi-Fi")
    );
    assert_eq!(
        pause_reason(&p(false, false), &c(false, false, 80)),
        None,
        "mobile data allowed"
    );
    assert_eq!(
        pause_reason(&p(false, true), &c(true, false, 80)),
        Some("Waiting for the charger")
    );
    assert_eq!(
        pause_reason(&p(false, false), &c(true, false, 19)),
        Some("Battery is low")
    );
    assert_eq!(
        pause_reason(&p(false, false), &c(true, true, 5)),
        None,
        "charging beats a low battery"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn only_new_and_changed_photos_are_imported_and_removed_ones_stay_in_the_library() {
    let r = rig().await;
    for (i, f) in r.files.iter().take(6).enumerate() {
        r.source.put(&format!("a{i}"), f, 100 + i as i64);
    }
    // Looking first (before turning backup on) only counts; nothing is imported.
    let found = r.core.backup_run(false).await.unwrap();
    assert_eq!(
        (found.imported, found.pending),
        (0, 6),
        "first-run 'found 6 photos'"
    );
    assert_eq!(photos(&r.core).await, 0);

    r.core.backup_configure(policy(true, false)).await.unwrap();
    let report = r.core.backup_run(true).await.unwrap();
    assert_eq!(
        report.pending,
        0,
        "{report:?} {:?}",
        r.core.backup_status().await.unwrap()
    );
    assert_eq!(photos(&r.core).await, 6);
    let reads = r.source.reads().len();

    // Nothing new: nothing read.
    let again = r.core.backup_run(true).await.unwrap();
    assert_eq!((again.imported, again.pending), (0, 0));
    assert_eq!(
        r.source.reads().len(),
        reads,
        "unchanged photos are not read again"
    );

    // A new photo and an edited one (new content, same id, later modification time).
    r.source.put("b0", &r.files[6], 300);
    r.source.put("a1", &r.files[7], 400);
    let report = r.core.backup_run(true).await.unwrap();
    assert_eq!(report.imported, 2);
    assert_eq!(photos(&r.core).await, 8);

    // Deleted from the roll: the library keeps its copy.
    r.source.remove("a0");
    r.core.backup_run(true).await.unwrap();
    assert_eq!(photos(&r.core).await, 8);

    // The same bytes under another asset id are recognised as a duplicate.
    r.source.put("copy", &r.files[2], 500);
    let dup = r.core.backup_run(true).await.unwrap();
    assert_eq!((dup.imported, dup.duplicates), (0, 1));
    r.core.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sync_conditions_hold_back_the_import_and_the_fetcher_until_they_are_met() {
    let r = rig().await;
    r.source.put("a", &r.files[0], 1);
    r.source.put("b", &r.files[1], 2);
    r.core.backup_configure(policy(true, true)).await.unwrap();

    r.source.set_conditions(Conditions {
        on_wifi: false,
        charging: true,
        battery_pct: 90,
    });
    let held = r.core.backup_run(true).await.unwrap();
    assert_eq!((held.imported, held.pending), (0, 2));
    let st = r.core.backup_status().await.unwrap();
    assert_eq!(st.paused.as_deref(), Some("Waiting for Wi-Fi"));
    assert_eq!(st.pending, 2);

    r.source.set_conditions(Conditions {
        on_wifi: true,
        charging: false,
        battery_pct: 90,
    });
    assert_eq!(
        r.core.backup_run(true).await.unwrap().imported,
        0,
        "charging is required too"
    );
    assert_eq!(
        r.core.backup_status().await.unwrap().paused.as_deref(),
        Some("Waiting for the charger")
    );

    r.source.set_conditions(Conditions {
        on_wifi: true,
        charging: true,
        battery_pct: 90,
    });
    let ok = r.core.backup_run(true).await.unwrap();
    assert_eq!((ok.imported, ok.pending), (2, 0));
    assert_eq!(r.core.backup_status().await.unwrap().paused, None);

    // Backup switched off: nothing is reported as paused and a manual look only counts.
    r.core
        .backup_configure(BackupPolicy {
            enabled: false,
            ..policy(true, true)
        })
        .await
        .unwrap();
    r.source.put("c", &r.files[2], 3);
    assert_eq!(r.core.backup_run(false).await.unwrap().pending, 1);
    r.core.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unreadable_photos_are_reported_and_retried_without_blocking_the_rest() {
    let r = rig().await;
    r.source.put("ok1", &r.files[0], 1);
    r.source.put("icloud", &r.files[1], 2);
    r.source.put("ok2", &r.files[2], 3);
    r.source.make_unreadable("icloud");
    r.core.backup_configure(policy(false, false)).await.unwrap();
    let report = r.core.backup_run(true).await.unwrap();
    assert_eq!(report.imported, 2);
    assert_eq!(report.pending, 1);
    assert!(
        report
            .failed
            .iter()
            .any(|f| f.contains("icloud") && f.contains("iCloud")),
        "{:?}",
        report.failed
    );
    assert_eq!(photos(&r.core).await, 2);
    assert!(r.core.backup_status().await.unwrap().last_error.is_some());
    r.core.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn denied_access_is_an_error_with_a_next_step_and_state_survives_a_restart() {
    let r = rig().await;
    r.source.put("a", &r.files[0], 1);
    r.source.set_permission(Permission::Denied);
    let err = r
        .core
        .backup_configure(policy(true, false))
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("system settings"), "{err:#}");
    assert!(
        !r.core.backup_status().await.unwrap().policy.enabled,
        "not enabled without access"
    );

    r.source.set_permission(Permission::Limited);
    r.source.set_conditions(Conditions {
        on_wifi: false,
        charging: false,
        battery_pct: 90,
    });
    r.core.backup_configure(policy(true, false)).await.unwrap();
    assert_eq!(
        r.core.backup_run(true).await.unwrap().pending,
        1,
        "held back: not on Wi-Fi"
    );
    let dir = r.core.data_dir().to_path_buf();
    r.core.shutdown().await.unwrap();

    // After a restart the policy and the waiting photo are still there.
    let core = AppCore::open(&dir, AppOptions::local_only()).await.unwrap();
    let source = Arc::new(FakePhotoSource::new(dir.join("scratch2")));
    source.put("a", &r.files[0], 1);
    core.set_photo_source(source.clone());
    let st = core.backup_status().await.unwrap();
    assert!(st.policy.enabled && st.policy.wifi_only);
    assert_eq!(st.pending, 1);
    source.set_conditions(Conditions {
        on_wifi: true,
        charging: false,
        battery_pct: 90,
    });
    assert_eq!(core.backup_run(true).await.unwrap().pending, 0);
    assert_eq!(photos(&core).await, 1);
    core.shutdown().await.unwrap();
}

/// "Add photos": the person picks files the backup does not see. They are imported in batches,
/// progress reads as one run, the temporary copies are gone afterwards, and the device's storage
/// counts them. Cancelling the picker imports nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn picked_photos_are_imported_in_batches_and_their_copies_removed() {
    let r = rig().await;
    let picks: Vec<PathBuf> = r.files.iter().take(45).cloned().collect();
    assert!(picks.len() > 20, "more than one batch");
    let mut events = r.core.events();
    r.source.set_picks(picks.clone());
    let got = r.core.import_picked(false).await.unwrap();
    assert_eq!(
        got.imported + got.duplicates + got.skipped,
        picks.len() as u32,
        "{got:?}"
    );
    assert!(got.failed.is_empty(), "{got:?}");
    assert!(got.imported > 20);
    assert_eq!(photos(&r.core).await, got.imported as usize);

    // Events are forwarded by a task: read until the run's last one arrives.
    let mut last = None;
    while last != Some(picks.len() as u32) {
        let ev = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
            .await
            .expect("the last progress event arrives")
            .unwrap();
        if let desktop_mobile::types::AppEvent::ImportProgress { done, total } = ev {
            assert_eq!(total, picks.len() as u32, "one run, not one per batch");
            assert!(last.is_none_or(|l| done >= l), "progress never goes back");
            last = Some(done);
        }
    }

    let leftovers = std::fs::read_dir(r._dir.path().join("scratch"))
        .map(|d| d.count())
        .unwrap_or(0);
    assert_eq!(leftovers, 0, "temporary copies are deleted");

    let storage = r.core.device_storage().await.unwrap();
    assert_eq!(storage.len(), 1);
    assert_eq!(storage[0].originals, got.imported);
    assert!(storage[0].original_bytes > 0.0 && storage[0].other_bytes > 0.0);

    // Cancelled: nothing picked, nothing imported.
    let none = r.core.import_picked(true).await.unwrap();
    assert_eq!((none.imported, none.duplicates), (0, 0));
    assert_eq!(photos(&r.core).await, got.imported as usize);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_device_without_a_camera_roll_cannot_pick() {
    let dir = tempfile::tempdir().unwrap();
    let core = AppCore::open(dir.path().join("app"), AppOptions::local_only())
        .await
        .unwrap();
    core.create_library("Mac", "keeper").await.unwrap();
    assert!(core.import_picked(false).await.is_err());
}
