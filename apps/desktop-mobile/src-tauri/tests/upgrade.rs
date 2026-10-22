//! M6-05: a library written before the format marker existed opens after an upgrade without
//! loss, and data from a newer layout is refused.

use desktop_mobile::{AppCore, AppOptions};
use std::path::{Path, PathBuf};

fn sample(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../fixtures/samples")
        .join(name)
        .display()
        .to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_older_library_is_upgraded_in_place_after_a_copy() {
    let dir = tempfile::tempdir().unwrap();
    {
        let core = AppCore::open(dir.path(), AppOptions::local_only()).await.unwrap();
        core.create_library("Laptop", "keeper").await.unwrap();
        core.import_paths(&[sample("photo-gps.jpg"), sample("screenshot.png")])
            .await
            .unwrap();
        let id = core.library_page(None, 10).await.unwrap().items[0].id.clone();
        core.set_favorite(&[id], true).await.unwrap();
        core.shutdown().await.unwrap();
    }
    // What an earlier build left behind: the same files, no marker.
    std::fs::remove_file(dir.path().join("format.json")).unwrap();

    let core = AppCore::open(dir.path(), AppOptions::local_only()).await.unwrap();
    let page = core.library_page(None, 10).await.unwrap();
    assert_eq!(page.items.len(), 2, "no photo lost");
    assert_eq!(page.items.iter().filter(|p| p.favorite).count(), 1, "edits kept");
    assert!(core.app_state().await.unwrap().has_library);
    core.shutdown().await.unwrap();

    let backups: Vec<PathBuf> = std::fs::read_dir(dir.path().join("backups"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(backups.len(), 1);
    assert!(backups[0].join("app.json").exists(), "the pre-upgrade copy has the app file");
    assert!(dir.path().join("format.json").exists());
    // Opening again does not copy again.
    AppCore::open(dir.path(), AppOptions::local_only()).await.unwrap().shutdown().await.unwrap();
    assert_eq!(std::fs::read_dir(dir.path().join("backups")).unwrap().count(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn data_from_a_newer_layout_is_refused_and_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("format.json"),
        br#"{"format": 99, "written_by": "9.9.9"}"#,
    )
    .unwrap();
    let err = AppCore::open(dir.path(), AppOptions::local_only())
        .await
        .err()
        .expect("refused");
    assert!(err.to_string().contains("newer version"), "{err}");
    assert!(!dir.path().join("node").exists(), "nothing was created or changed");
}
