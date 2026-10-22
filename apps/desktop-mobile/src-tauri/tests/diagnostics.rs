//! M6-07: the diagnostics bundle of a real library says how things are, not what is in it.

use desktop_mobile::{AppCore, AppOptions};
use std::path::Path;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_bundle_has_the_numbers_and_none_of_the_content() {
    desktop_mobile::diagnostics::init_logging();
    let dir = tempfile::tempdir().unwrap();
    let core = AppCore::open(dir.path(), AppOptions::local_only()).await.unwrap();
    core.create_library("Ana's laptop", "keeper").await.unwrap();
    let samples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/samples");
    let photo = samples.join("photo-gps.jpg").display().to_string();
    core.import_paths(&[photo.clone()]).await.unwrap();
    core.create_album("Secret Party").await.unwrap();
    core.shared_create("Surprise Trip").await.unwrap();
    tracing::warn!("could not read {photo} for Surprise Trip at 38.722300, -9.139300 (Ana's laptop)");
    tracing::info!("sync finished in 12ms");

    let bundle = core.diagnostics().await.unwrap();
    let t = &bundle.text;
    assert!(bundle.file_name.starts_with("shoebox-diagnostics-") && bundle.file_name.ends_with(".txt"));
    assert!(t.contains("library: 1 photos"), "{t}");
    assert!(t.contains("role=keeper") && t.contains("shared albums: 1"), "{t}");
    assert!(t.contains("sync finished in 12ms"), "ordinary log lines stay");
    for secret in ["Secret Party", "Surprise Trip", "Ana's laptop", "photo-gps", "fixtures", "38.7223", "9.1393", "Pictures"] {
        assert!(!t.contains(secret), "`{secret}` is in the bundle:\n{t}");
    }
    core.shutdown().await.unwrap();
}
