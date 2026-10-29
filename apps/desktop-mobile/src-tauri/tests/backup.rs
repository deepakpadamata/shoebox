//! M4-05/06: camera-roll backup with a fake photo source: incremental import, sync conditions,
//! failures, restarts.

use desktop_mobile::photos_source::{Conditions, FakePhotoSource, Permission, Picked};
use desktop_mobile::types::{BackupPolicy, BackupScope};
use desktop_mobile::{pause_reason, AppCore, AppOptions};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A phone gallery of 100 distinct photos, each larger than what the blob store keeps inline
/// (16 KiB), so referencing is really exercised: the sample JPEGs with a unique trailer.
fn corpus(dir: &Path) -> Vec<PathBuf> {
    let samples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/samples");
    let bases = ["photo-gps.jpg", "IMG_0001.jpg"].map(|f| std::fs::read(samples.join(f)).unwrap());
    let out = dir.join("DCIM").join("Camera");
    std::fs::create_dir_all(&out).unwrap();
    (0..100)
        .map(|i| {
            let mut bytes = bases[i % 2].clone();
            bytes.extend_from_slice(format!("shoebox test photo {i}").as_bytes());
            let p = out.join(format!("IMG_{i:04}.jpg"));
            std::fs::write(&p, bytes).unwrap();
            p
        })
        .collect()
}

fn policy(wifi: bool, charging: bool) -> BackupPolicy {
    BackupPolicy {
        enabled: true,
        wifi_only: wifi,
        charging_only: charging,
        min_battery: 20,
        scope: BackupScope::Camera,
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
        scope: BackupScope::Camera,
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
    let copy = r.files[2].with_file_name("copy-of-a2.jpg");
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

/// Without photo access nothing is checked; backup turned off still follows the gallery.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_gallery_is_followed_with_backup_off_but_not_without_access() {
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
    r.source.set_permission(Permission::Denied);
    r.core.gallery_check().await.unwrap();
    assert_eq!(photos(&r.core).await, 2);
    r.source.set_permission(Permission::Granted);
    r.core.gallery_check().await.unwrap();
    assert_eq!(photos(&r.core).await, 1);
    r.core.shutdown().await.unwrap();
}

/// Camera-only (the default) skips downloads and screenshots; turning it off picks them up
/// without waiting for them to change, and turning it back on keeps what is already backed up.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn camera_only_backs_up_what_the_camera_took() {
    let r = rig().await;
    let elsewhere = |folder: &str, from: &Path| {
        let dir = r._dir.path().join(folder);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(from.file_name().unwrap());
        std::fs::rename(from, &p).unwrap();
        p
    };
    let download = elsewhere("Download", &r.files[1]);
    let screenshot = elsewhere("DCIM/Screenshots", &r.files[2]);
    r.source.put("cam", &r.files[0], 1);
    r.source.put("dl", &download, 2);
    r.source.put("shot", &screenshot, 3);
    r.core.backup_configure(policy(false, false)).await.unwrap();
    let report = r.core.backup_run(true).await.unwrap();
    assert_eq!((report.imported, report.pending), (1, 0));

    let all = BackupPolicy {
        scope: BackupScope::All,
        ..policy(false, false)
    };
    r.core.backup_configure(all).await.unwrap();
    assert_eq!(r.core.backup_run(true).await.unwrap().imported, 2);
    assert_eq!(photos(&r.core).await, 3);

    // Back to camera-only: nothing already backed up goes, and a new download is skipped.
    r.core.backup_configure(policy(false, false)).await.unwrap();
    let later = elsewhere("Download", &r.files[3]);
    r.source.put("dl2", &later, 4);
    let report = r.core.backup_run(true).await.unwrap();
    assert_eq!((report.imported, report.pending), (0, 0));
    assert_eq!(photos(&r.core).await, 3);
    r.core.shutdown().await.unwrap();
}

/// The folders are listed camera first with what each holds; a chosen list backs up exactly
/// those folders, and un-choosing one keeps what it already backed up.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn chosen_folders_are_backed_up_and_nothing_else() {
    let r = rig().await;
    let downloads = r._dir.path().join("Download");
    let shots = r._dir.path().join("Pictures/Screenshots");
    for d in [&downloads, &shots] {
        std::fs::create_dir_all(d).unwrap();
    }
    let mv = |f: &PathBuf, to: &Path| {
        let p = to.join(f.file_name().unwrap());
        std::fs::rename(f, &p).unwrap();
        p
    };
    r.source.put("cam", &r.files[0], 1);
    r.source.put("dl1", mv(&r.files[1], &downloads), 2);
    r.source.put("dl2", mv(&r.files[2], &downloads), 3);
    r.source.put("shot", mv(&r.files[3], &shots), 4);
    r.core.backup_configure(policy(false, false)).await.unwrap();

    let folders = r.core.backup_folders().await.unwrap();
    let seen: Vec<_> = folders
        .iter()
        .map(|f| (f.name.as_str(), f.photos, f.camera, f.included))
        .collect();
    assert_eq!(
        seen,
        [
            ("Camera", 1, true, true),
            ("Download", 2, false, false),
            ("Screenshots", 1, false, false)
        ]
    );

    let chosen = |dirs: &[&Path]| BackupPolicy {
        scope: BackupScope::Folders(dirs.iter().map(|d| d.display().to_string()).collect()),
        ..policy(false, false)
    };
    let camera = r.files[0].parent().unwrap().to_path_buf();
    r.core
        .backup_configure(chosen(&[&camera, &downloads]))
        .await
        .unwrap();
    assert_eq!(r.core.backup_run(true).await.unwrap().imported, 3);
    let included: Vec<_> = r
        .core
        .backup_folders()
        .await
        .unwrap()
        .into_iter()
        .map(|f| f.included)
        .collect();
    assert_eq!(included, [true, true, false]);

    // Downloads un-chosen: what it backed up stays, a new download waits for nothing.
    r.core.backup_configure(chosen(&[&camera])).await.unwrap();
    r.source.put("dl3", mv(&r.files[4], &downloads), 5);
    let report = r.core.backup_run(true).await.unwrap();
    assert_eq!((report.imported, report.pending), (0, 0));
    assert_eq!(photos(&r.core).await, 3);
    r.core.shutdown().await.unwrap();
}

/// With only some photos shared, Android hides every other file, so a deleted file and an
/// unshared one look alike. A disappearance is a deletion, except across a change of access:
/// Shoebox's own "share more" prompt, or access narrowed from all photos to some. Those photos
/// were unshared and stay in the library.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn with_some_photos_shared_deletions_count_and_unshares_do_not() {
    let r = rig().await;
    r.source.set_permission(Permission::Limited);
    for i in 0..6 {
        r.source.put(&format!("a{i}"), &r.files[i], 1);
    }
    r.core.backup_configure(policy(false, false)).await.unwrap();
    r.core.backup_run(true).await.unwrap();
    let n = photos(&r.core).await;
    assert_eq!(n, 6);

    // Deleted in the gallery: deleted from the library.
    std::fs::remove_file(&r.files[0]).unwrap();
    r.core.backup_run(true).await.unwrap();
    assert_eq!(photos(&r.core).await, 5);

    // Unshared at Shoebox's own prompt: kept, no longer held here.
    r.source.answer_request(Permission::Limited);
    r.source.unshare_on_request(vec![r.files[1].clone()]);
    r.core.import_picked(false).await.unwrap();
    assert_eq!(photos(&r.core).await, 5);
    assert_eq!(r.core.backup_status().await.unwrap().imported, 4);
    assert_eq!(r.core.device_storage().await.unwrap()[0].originals, 4);

    // Full access, then narrowed in the system settings: what vanished was unshared.
    r.source.set_permission(Permission::Granted);
    r.core.backup_run(true).await.unwrap();
    r.source.set_permission(Permission::Limited);
    std::fs::remove_file(&r.files[2]).unwrap();
    r.core.backup_run(true).await.unwrap();
    assert_eq!(photos(&r.core).await, 5);
    assert_eq!(r.core.backup_status().await.unwrap().imported, 3);

    // Later disappearances are deletions again.
    std::fs::remove_file(&r.files[3]).unwrap();
    r.core.backup_run(true).await.unwrap();
    assert_eq!(photos(&r.core).await, 4);
    r.core.shutdown().await.unwrap();
}

/// The viewer asks for an original whose gallery file cannot be read (here: cut short). The
/// photo stays, this phone stops holding it, and the original is reported as not here so the
/// viewer gets it from another device instead of breaking.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_unreadable_gallery_file_is_let_go_not_served() {
    let r = rig().await;
    r.source.put("a", &r.files[0], 1);
    r.core.backup_configure(policy(false, false)).await.unwrap();
    r.core.backup_run(true).await.unwrap();
    let id = r.core.library_page(None, 10).await.unwrap().items[0]
        .id
        .clone();
    assert!(r.core.photo_detail(&id).await.unwrap().has_original);
    std::fs::write(&r.files[0], b"cut").unwrap();
    let got = desktop_mobile::blob::serve(&r.core, &id, "original", None)
        .await
        .unwrap();
    assert!(got.is_none(), "not served from a broken file");
    assert_eq!(photos(&r.core).await, 1, "the photo stays");
    assert!(!r.core.photo_detail(&id).await.unwrap().has_original);
    assert_eq!(r.core.backup_status().await.unwrap().imported, 0);
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
