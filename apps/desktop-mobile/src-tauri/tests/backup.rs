//! M4-05/06: camera-roll backup with a fake photo source: incremental import, sync conditions,
//! failures, restarts.

use desktop_mobile::photos_source::{Conditions, FakePhotoSource, Permission, Picked};
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
async fn only_new_and_changed_photos_are_imported_and_the_gallery_decides_what_stays() {
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

    // Nothing new: nothing imported again.
    let again = r.core.backup_run(true).await.unwrap();
    assert_eq!((again.imported, again.duplicates, again.pending), (0, 0, 0));

    // A new photo, and an edited one (new content, same id, later modification time): the
    // edit replaces the old version, which the gallery no longer has.
    r.source.put("b0", &r.files[6], 300);
    r.source.put("a1", &r.files[7], 400);
    let report = r.core.backup_run(true).await.unwrap();
    assert_eq!(report.imported, 2);
    assert_eq!(photos(&r.core).await, 7);
    assert_eq!(r.core.recently_deleted().await.unwrap().len(), 1);

    // Deleted in the gallery: deleted from the library too (Recently Deleted).
    r.source.remove("a0");
    std::fs::remove_file(&r.files[0]).unwrap();
    r.core.backup_run(true).await.unwrap();
    assert_eq!(photos(&r.core).await, 6);
    assert_eq!(r.core.recently_deleted().await.unwrap().len(), 2);

    // The same bytes under another asset id are recognised as a duplicate, and deleting one of
    // the two files keeps the photo.
    let copy = r._dir.path().join("copy-of-a2.jpg");
    std::fs::copy(&r.files[2], &copy).unwrap();
    r.source.put("copy", &copy, 500);
    let dup = r.core.backup_run(true).await.unwrap();
    assert_eq!((dup.imported, dup.duplicates), (0, 1));
    r.source.remove("a2");
    std::fs::remove_file(&r.files[2]).unwrap();
    r.core.backup_run(true).await.unwrap();
    assert_eq!(
        photos(&r.core).await,
        6,
        "the other gallery copy is still there"
    );
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
            .any(|f| f.contains("icloud") && f.contains("second copy")),
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

/// "Add photos": gallery photos are added in place like the camera roll; files from elsewhere
/// are copied (the phone has no other copy) in batches whose progress reads as one run, and the
/// temporary copies are gone afterwards. A gallery photo the app may not read in place is not
/// added. Cancelling imports nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn picked_photos_are_added_in_place_or_copied_once() {
    let r = rig().await;
    let gallery: Vec<Picked> = r.files[..5]
        .iter()
        .enumerate()
        .map(|(i, f)| Picked::InPlace(FakePhotoSource::asset(&format!("g{i}"), f, 1)))
        .collect();
    let elsewhere: Vec<PathBuf> = r.files[5..50].to_vec();
    let mut picks = gallery.clone();
    picks.extend(
        elsewhere
            .iter()
            .map(|p| Picked::Copy(p.display().to_string())),
    );
    picks.push(Picked::Unreadable("IMG_9999.jpg".into()));
    let mut events = r.core.events();
    r.source.set_picks(picks);
    let got = r.core.import_picked(true).await.unwrap();
    assert_eq!(got.imported + got.duplicates + got.skipped, 50, "{got:?}");
    assert_eq!(got.failed.len(), 1, "{got:?}");
    assert!(got.failed[0].contains("IMG_9999.jpg"));
    assert_eq!(photos(&r.core).await, got.imported as usize);
    assert_eq!(
        r.core.backup_status().await.unwrap().imported,
        5,
        "gallery picks are tracked like the camera roll"
    );

    // Events are forwarded by a task: read until the copies' last one arrives.
    let mut last = None;
    while last != Some(elsewhere.len() as u32) {
        let ev = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
            .await
            .expect("the last progress event arrives")
            .unwrap();
        if let desktop_mobile::types::AppEvent::ImportProgress { done, total } = ev {
            if total == elsewhere.len() as u32 {
                assert!(last.is_none_or(|l| done >= l), "progress never goes back");
                last = Some(done);
            }
        }
    }

    let leftovers = std::fs::read_dir(r._dir.path().join("scratch"))
        .map(|d| d.count())
        .unwrap_or(0);
    assert_eq!(leftovers, 0, "temporary copies are deleted");

    let storage = r.core.device_storage().await.unwrap();
    assert_eq!(storage.len(), 1);
    assert_eq!(storage[0].originals, got.imported);

    // Cancelled: nothing picked, nothing imported.
    let none = r.core.import_picked(true).await.unwrap();
    assert_eq!((none.imported, none.duplicates), (0, 0));
    assert_eq!(photos(&r.core).await, got.imported as usize);
}

/// With only some photos shared, "from the gallery" asks Android to share more and adds what it
/// then sees, in place. Refusing access is an error with a next step.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn adding_from_the_gallery_with_limited_access_shares_more() {
    let r = rig().await;
    r.source.set_permission(Permission::Limited);
    r.source.put("shared", &r.files[0], 1);
    r.source.put("newly-shared", &r.files[1], 2);
    r.source.answer_request(Permission::Limited);
    let got = r.core.import_picked(false).await.unwrap();
    assert_eq!(got.imported, 2, "{got:?}");
    assert_eq!(r.core.backup_status().await.unwrap().imported, 2);

    r.source.set_permission(Permission::Denied);
    r.source.answer_request(Permission::Denied);
    let err = r.core.import_picked(false).await.unwrap_err();
    assert!(format!("{err:#}").contains("access"), "{err:#}");
}

/// Gallery photos are referenced, not copied: the store grows by previews only.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn backed_up_gallery_photos_are_not_copied() {
    let r = rig().await;
    let blobs = r.core.data_dir().join("node").join("blobs");
    let before = dir_bytes(&blobs);
    let mut originals = 0;
    for (i, f) in r.files.iter().take(20).enumerate() {
        r.source.put(&format!("a{i}"), f, 1);
        originals += f.metadata().unwrap().len();
    }
    r.core.backup_configure(policy(false, false)).await.unwrap();
    assert_eq!(r.core.backup_run(true).await.unwrap().imported, 20);
    let grew = dir_bytes(&blobs) - before;
    assert!(
        grew < originals / 2,
        "the store grew by {grew} bytes for {originals} bytes of originals"
    );
    // Nothing to free on a phone: the originals are the gallery's.
    assert_eq!(r.core.free_up_space(true).await.unwrap().photos, 0);
    r.core.shutdown().await.unwrap();
}

/// A storage glitch (a card swapped out, a broken index) must not empty the library: many
/// photos gone at once wait for the person, who may keep them or delete them.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn many_photos_gone_at_once_wait_for_the_person() {
    let r = rig().await;
    for (i, f) in r.files.iter().take(80).enumerate() {
        r.source.put(&format!("a{i}"), f, 1);
    }
    r.core.backup_configure(policy(false, false)).await.unwrap();
    r.core.backup_run(true).await.unwrap();
    // The corpus has a few files that are not photos, or the same photo twice.
    let n = photos(&r.core).await;
    let tracked = r.core.backup_status().await.unwrap().imported;
    assert!(n > 70);
    for f in &r.files[..60] {
        std::fs::remove_file(f).unwrap();
    }
    r.core.backup_run(true).await.unwrap();
    assert_eq!(photos(&r.core).await, n, "held, not applied");
    let held = r.core.backup_status().await.unwrap().gallery_deletions_held;
    assert!(held > 50, "{held}");
    // Kept: the photos stay in the library, this phone no longer holds them.
    r.core.answer_gallery_deletions(false).await.unwrap();
    assert_eq!(photos(&r.core).await, n);
    let st = r.core.backup_status().await.unwrap();
    assert_eq!(
        (st.gallery_deletions_held, st.imported),
        (0, tracked - held)
    );
    let n = photos(&r.core).await;

    // A few more go: below the threshold, deleted at once.
    for f in &r.files[60..63] {
        std::fs::remove_file(f).unwrap();
    }
    r.core.backup_run(true).await.unwrap();
    assert_eq!(photos(&r.core).await, n - 3);
    r.core.shutdown().await.unwrap();
}

/// With only some photos shared, a file the app cannot see may just be one no longer shared:
/// nothing is deleted. Backup turned off still follows the gallery.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn only_full_access_lets_the_gallery_delete() {
    let r = rig().await;
    r.source.put("a", &r.files[0], 1);
    r.source.put("b", &r.files[1], 2);
    r.core.backup_configure(policy(false, false)).await.unwrap();
    r.core.backup_run(true).await.unwrap();
    r.core
        .backup_configure(BackupPolicy {
            enabled: false,
            ..policy(false, false)
        })
        .await
        .unwrap();
    std::fs::remove_file(&r.files[0]).unwrap();
    r.source.set_permission(Permission::Limited);
    r.core.gallery_check().await.unwrap();
    assert_eq!(photos(&r.core).await, 2);
    r.source.set_permission(Permission::Granted);
    r.core.gallery_check().await.unwrap();
    assert_eq!(photos(&r.core).await, 1);
    r.core.shutdown().await.unwrap();
}

/// An earlier build copied camera-roll photos into the store. They are found again, tracked, and
/// each copy is replaced by a reference to its gallery file: the store shrinks, the photos stay.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn copies_from_an_earlier_build_become_references() {
    let dir = tempfile::tempdir().unwrap();
    let files = corpus(dir.path());
    let opts = || AppOptions {
        gc_interval: Some(std::time::Duration::from_millis(200)),
        ..AppOptions::local_only()
    };
    let app = dir.path().join("app");
    let core = AppCore::open(&app, opts()).await.unwrap();
    core.create_library("Pixel", "phone").await.unwrap();
    let paths: Vec<String> = files[..10]
        .iter()
        .map(|p| p.display().to_string())
        .collect();
    assert_eq!(core.import_paths(&paths).await.unwrap().imported, 10);
    core.shutdown().await.unwrap();
    // What that build left in backup.json.
    let imported: serde_json::Map<String, serde_json::Value> = (0..10)
        .map(|i| (format!("a{i}"), serde_json::json!(1)))
        .collect();
    std::fs::write(
        app.join("backup.json"),
        serde_json::to_vec(&serde_json::json!({
            "policy": policy(false, false),
            "token": "11",
            "pending": {},
            "imported": imported,
            "last_scan_ms": 0,
            "last_error": null,
        }))
        .unwrap(),
    )
    .unwrap();

    let core = AppCore::open(&app, opts()).await.unwrap();
    let source = Arc::new(FakePhotoSource::new(dir.path().join("scratch")));
    for (i, f) in files[..10].iter().enumerate() {
        source.put(&format!("a{i}"), f, 1);
    }
    core.set_photo_source(source.clone());
    let blobs = app.join("node").join("blobs");
    let with_copies = dir_bytes(&blobs);
    let originals: u64 = files[..10]
        .iter()
        .map(|f| f.metadata().unwrap().len())
        .sum();
    let first = core.backup_run(true).await.unwrap();
    assert_eq!((first.imported, first.duplicates), (0, 10));
    assert_eq!(core.backup_status().await.unwrap().imported, 10);
    let mut shrunk = false;
    for _ in 0..50 {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        core.backup_run(true).await.unwrap();
        if dir_bytes(&blobs) + originals / 2 < with_copies {
            shrunk = true;
            break;
        }
    }
    assert!(shrunk, "the copies are replaced by references");
    assert_eq!(photos(&core).await, 10);
    assert_eq!(core.device_storage().await.unwrap()[0].originals, 10);
    core.shutdown().await.unwrap();
}

fn dir_bytes(d: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(d) else {
        return 0;
    };
    entries
        .flatten()
        .map(|e| {
            let p = e.path();
            if p.is_dir() {
                dir_bytes(&p)
            } else {
                e.metadata().map(|m| m.len()).unwrap_or(0)
            }
        })
        .sum()
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
