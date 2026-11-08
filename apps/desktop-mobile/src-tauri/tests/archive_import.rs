//! I1-10: a Google Takeout imported from the app core: plan, start, stop, continue, report.

use desktop_mobile::types::*;
use desktop_mobile::{AppCore, AppOptions};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

fn samples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/samples")
}

fn export(dir: &Path) -> Vec<String> {
    xtask::takeout::generate(&dir.join("export"), &samples(), 7, 3)
        .unwrap()
        .parts
        .iter()
        .map(|p| p.display().to_string())
        .collect()
}

async fn wait_for(core: &Arc<AppCore>, state: &str) -> ArchiveImportInfo {
    for _ in 0..400 {
        let s = core.archive_import_status();
        if s.state == state {
            return s;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!(
        "never reached `{state}`: {:?}",
        core.archive_import_status()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_takeout_is_planned_imported_stopped_and_continued() {
    let dir = tempfile::tempdir().unwrap();
    let core = AppCore::open(dir.path().join("app"), AppOptions::local_only())
        .await
        .unwrap();
    core.create_library("MacBook", "keeper").await.unwrap();
    let parts = export(dir.path());

    assert_eq!(core.archive_import_status().state, "idle");
    let plan = core.plan_archive_import(&parts).await.unwrap();
    assert!(plan.is_takeout && plan.photos > 0 && plan.trashed == 2);
    assert_eq!(plan.albums.len(), 3);
    assert_eq!(
        core.archive_import_status().state,
        "idle",
        "a plan starts nothing"
    );

    // Start, stop at once: the files are remembered and the state is `stopped`.
    core.start_archive_import(&parts).await.unwrap();
    assert!(
        core.start_archive_import(&parts).await.is_err(),
        "one at a time"
    );
    core.stop_archive_import();
    let s = core.archive_import_status();
    assert_eq!(s.state, "stopped");
    assert_eq!(s.paths, parts);

    // Continue to the end.
    core.start_archive_import(&s.paths).await.unwrap();
    let done = wait_for(&core, "done").await;
    let r = done.report.unwrap();
    assert!(r.imported > 0 && r.failed.is_empty(), "{r:?}");
    assert_eq!(r.albums.len(), 3);
    assert_eq!(r.trashed, 2);
    assert!(done.paths.is_empty());
    let albums = core.albums().await.unwrap();
    assert_eq!(albums.len(), 3);

    core.forget_archive_import().unwrap();
    assert_eq!(core.archive_import_status().state, "idle");
    core.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_unfinished_import_is_offered_again_after_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("app");
    let core = AppCore::open(&app, AppOptions::local_only()).await.unwrap();
    core.create_library("MacBook", "keeper").await.unwrap();
    let parts = export(dir.path());
    core.start_archive_import(&parts).await.unwrap();
    core.stop_archive_import();
    core.shutdown().await.unwrap();

    let core = AppCore::open(&app, AppOptions::local_only()).await.unwrap();
    let s = core.archive_import_status();
    assert_eq!((s.state.as_str(), s.paths.len()), ("stopped", parts.len()));
    core.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_missing_file_ends_the_import_with_a_reason() {
    let dir = tempfile::tempdir().unwrap();
    let core = AppCore::open(dir.path().join("app"), AppOptions::local_only())
        .await
        .unwrap();
    core.create_library("MacBook", "keeper").await.unwrap();
    let missing = vec![dir.path().join("nope.zip").display().to_string()];
    assert!(core.plan_archive_import(&missing).await.is_err());
    core.start_archive_import(&missing).await.unwrap();
    let s = wait_for(&core, "stopped").await;
    assert!(s.error.is_some_and(|e| e.contains("none of the parts")));
    core.forget_archive_import().unwrap();
    core.shutdown().await.unwrap();
}
