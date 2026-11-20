//! Importing a Google Takeout or an iCloud Photos export on the keeper (A-148): the same import the
//! desktop app runs (`LibraryNode::plan_archives` / `import_archives`), inside the daemon, so it
//! uses the library the daemon has open and goes on when the command that started it exits.
//!
//! One import runs at a time. The files it was given are kept in `<storage>/import.json` until a
//! run ends with nothing left to retry, and a daemon that starts with that file continues the
//! import on its own (a NAS rebooted half way through needs nobody). The import's own journal
//! (`<storage>/imports/`) makes continuing read only what is not done yet.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use shoebox_node::{ArchiveImportReport, ArchivePlanSummary, ImportOptions, LibraryNode};
use shoebox_store::Library;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::task::JoinHandle;

/// The import the daemon is running, or ran last.
#[derive(Default)]
pub(crate) struct Run {
    task: Option<JoinHandle<()>>,
    /// Files done and the run's total, from the import's progress events.
    progress: Option<(usize, usize)>,
    /// The last finished run's report (JSON, as `report_json` makes it).
    report: Option<Value>,
    /// Why the last run ended early.
    error: Option<String>,
}

pub(crate) type Shared = Arc<Mutex<Run>>;

#[derive(Default, Serialize, Deserialize)]
struct Pending {
    paths: Vec<PathBuf>,
}

fn pending_path(storage: &Path) -> PathBuf {
    storage.join("import.json")
}

fn load_pending(storage: &Path) -> Vec<PathBuf> {
    std::fs::read(pending_path(storage))
        .ok()
        .and_then(|b| serde_json::from_slice::<Pending>(&b).ok())
        .map(|p| p.paths)
        .unwrap_or_default()
}

fn save_pending(storage: &Path, paths: &[PathBuf]) -> anyhow::Result<()> {
    let path = pending_path(storage);
    if paths.is_empty() {
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
            _ => return Ok(()),
        }
    }
    let bytes = serde_json::to_vec_pretty(&Pending {
        paths: paths.to_vec(),
    })?;
    std::fs::write(&path, bytes)?;
    Ok(())
}

fn source_name(s: Option<shoebox_media::takeout::Source>) -> &'static str {
    match s {
        Some(shoebox_media::takeout::Source::GoogleTakeout) => "google",
        Some(shoebox_media::takeout::Source::Icloud) => "icloud",
        None => "files",
    }
}

fn errors(list: &[(PathBuf, String)]) -> Vec<String> {
    list.iter()
        .map(|(p, e)| format!("{}: {e}", p.display()))
        .collect()
}

fn albums(list: &[(String, usize)]) -> Vec<Value> {
    list.iter()
        .map(|(title, photos)| json!({"title": title, "photos": photos}))
        .collect()
}

pub(crate) fn plan_json(p: &ArchivePlanSummary) -> Value {
    json!({
        "source": source_name(p.source),
        "photos": p.photos,
        "already": p.already,
        "copies": p.copies,
        "trashed": p.trashed,
        "not_photos": p.not_photos,
        "albums": albums(&p.albums),
        "with_export_data": p.with_export_data,
        "unmatched": p.unmatched + p.ambiguous,
        "missing": p.missing,
        "part_errors": errors(&p.part_errors),
        "needed_bytes": p.needed_bytes,
        "free_bytes": p.free_bytes,
    })
}

fn report_json(r: &ArchiveImportReport) -> Value {
    json!({
        "imported": r.import.imported.len(),
        "duplicates": r.import.duplicates.len(),
        "skipped": r.import.skipped.len(),
        "failed": errors(&r.import.failed),
        "resumed": r.resumed,
        "copies": r.copies,
        "albums": albums(&r.albums),
        "export_dates": r.export_dates,
        "export_places": r.export_places,
        "favorites": r.favorites,
        "trashed": r.trashed,
        "not_photos": r.not_photos,
        "unmatched": r.unmatched + r.ambiguous,
        "missing": r.missing,
        "part_errors": errors(&r.part_errors),
    })
}

fn running(run: &Run) -> bool {
    run.task.as_ref().is_some_and(|t| !t.is_finished())
}

/// Start (or continue) importing `paths`. Fails if an import is already running.
pub(crate) fn start(
    run: &Shared,
    storage: &Path,
    ln: LibraryNode<Library>,
    paths: Vec<PathBuf>,
) -> anyhow::Result<()> {
    anyhow::ensure!(!paths.is_empty(), "no files given");
    let mut guard = run.lock().unwrap();
    anyhow::ensure!(
        !running(&guard),
        "an import is already running (`shoebox-keeper import` shows it)"
    );
    save_pending(storage, &paths)?;
    guard.error = None;
    guard.report = None;
    guard.progress = None;
    let shared = run.clone();
    let storage = storage.to_path_buf();
    // Progress comes as node events; only this import sends them on a keeper.
    let mut events = ln.node().events();
    let watcher = {
        let shared = shared.clone();
        tokio::spawn(async move {
            loop {
                match events.recv().await {
                    Ok(shoebox_node::NodeEvent::ImportProgress { done, total, .. }) => {
                        shared.lock().unwrap().progress = Some((done, total));
                    }
                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(_) => break,
                }
            }
        })
    };
    guard.task = Some(tokio::spawn(async move {
        tracing::info!("importing {} file(s) of an export", paths.len());
        let result = ln.import_archives(&paths, ImportOptions::default()).await;
        watcher.abort();
        let mut run = shared.lock().unwrap();
        match result {
            Ok(r) => {
                tracing::info!(
                    "import finished: {} new, {} already in the library",
                    r.import.imported.len(),
                    r.import.duplicates.len()
                );
                // Parts that could not be read, or files that failed, leave it open to try again.
                if r.part_errors.is_empty() && r.import.failed.is_empty() {
                    if let Err(e) = save_pending(&storage, &[]) {
                        tracing::warn!("forgetting the finished import: {e:#}");
                    }
                }
                run.report = Some(report_json(&r));
            }
            Err(e) => {
                tracing::warn!("import stopped: {e:#}");
                run.error = Some(format!("{e:#}"));
            }
        }
    }));
    Ok(())
}

/// Continue an import a previous run of the daemon did not finish.
pub(crate) fn resume(run: &Shared, storage: &Path, ln: LibraryNode<Library>) {
    let paths = load_pending(storage);
    if paths.is_empty() {
        return;
    }
    tracing::info!("continuing the unfinished import");
    if let Err(e) = start(run, storage, ln, paths) {
        tracing::warn!("could not continue the import: {e:#}");
    }
}

/// Stop the running import; what it finished stays, and starting it again continues.
pub(crate) fn stop(run: &Shared) -> bool {
    let mut guard = run.lock().unwrap();
    let was = running(&guard);
    if let Some(t) = guard.task.take() {
        t.abort();
    }
    guard.progress = None;
    was
}

/// Stop it if it runs, and forget it: the files, the last report and its error. The journal
/// stays on disk, so importing the same files later still skips what was done.
pub(crate) fn forget(run: &Shared, storage: &Path) -> anyhow::Result<()> {
    stop(run);
    let mut guard = run.lock().unwrap();
    guard.report = None;
    guard.error = None;
    save_pending(storage, &[])
}

/// `idle`, `running`, `stopped` (unfinished: start it again to continue) or `done`, with the
/// files, the progress and the last report.
pub(crate) fn status(run: &Shared, storage: &Path) -> Value {
    let paths = load_pending(storage);
    let guard = run.lock().unwrap();
    let state = if running(&guard) {
        "running"
    } else if !paths.is_empty() {
        "stopped"
    } else if guard.report.is_some() {
        "done"
    } else {
        "idle"
    };
    let (done, total) = match (state, guard.progress) {
        ("running", Some(p)) => (Some(p.0), Some(p.1)),
        _ => (None, None),
    };
    json!({
        "state": state,
        "paths": paths,
        "done": done,
        "total": total,
        "report": guard.report,
        "error": guard.error,
    })
}
