//! I1-04 / I1-05 / I1-07 (Google half): a Takeout in zip parts imports with its dates, places,
//! favourites and albums; a run stopped half way continues; damaged parts are reported by name.

use shoebox_core::clock::ManualClock;
use shoebox_core::ids::Blake3Hash;
use shoebox_core::model::*;
use shoebox_core::store::*;
use shoebox_node::{
    imported_album_id, ArchiveImportReport, ImportOptions, LibraryNode, Node, NodeConfig, NodeEvent,
};
use shoebox_store_mem::{MemBackend, MemLibrary, MemNetwork};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use xtask::takeout::Truth;

struct Rig {
    ln: LibraryNode<MemLibrary>,
    dir: tempfile::TempDir,
}

async fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let clock = ManualClock::new(1_700_000_000_000);
    let node = Node::open(NodeConfig::local_only(dir.path().join("node"), "n"))
        .await
        .unwrap();
    let device = shoebox_core::ids::EndpointId(*node.id().as_bytes());
    let backend = MemBackend::new(MemNetwork::new(), device, clock.clone());
    let (lib, _) = backend
        .create_library(&RootPublicKey([1; 32]))
        .await
        .unwrap();
    Rig {
        ln: LibraryNode::new(node, Arc::new(lib), clock),
        dir,
    }
}

fn samples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/samples")
}

fn export(r: &Rig) -> (Vec<PathBuf>, Vec<Truth>) {
    let out = r.dir.path().join("export");
    let s = xtask::takeout::generate(&out, &samples(), 7, 3).unwrap();
    let truth = serde_json::from_slice(&std::fs::read(&s.truth).unwrap()).unwrap();
    (s.parts, truth)
}

fn hash(hex: &str) -> Blake3Hash {
    Blake3Hash(hex::decode(hex).unwrap().try_into().unwrap())
}

fn opts(concurrency: usize) -> ImportOptions {
    ImportOptions {
        concurrency,
        ..Default::default()
    }
}

/// The library holds exactly what the truth says.
async fn check_library(r: &Rig, truth: &[Truth]) {
    let store = r.ln.store();
    let photos: BTreeMap<PhotoId, PhotoRecord> = store
        .photos_by_time(TimeRange::ALL, Page::all())
        .await
        .unwrap()
        .into_iter()
        .map(|p| (p.id, p))
        .collect();
    let mut want: BTreeSet<PhotoId> = BTreeSet::new();
    for t in truth.iter().filter(|t| t.copy_of.is_none()) {
        let id = hash(&t.blake3);
        if t.trashed {
            assert!(
                !photos.contains_key(&id),
                "{} is trashed but imported",
                t.path
            );
            continue;
        }
        if let Some(still) = &t.live_photo_of {
            let still = truth.iter().find(|s| &s.path == still).unwrap();
            let m = store.motion(&hash(&still.blake3)).await.unwrap();
            assert_eq!(
                m.map(|m| m.video.hash),
                Some(id),
                "{} goes with its picture",
                t.path
            );
            continue;
        }
        want.insert(id);
        let p = photos
            .get(&id)
            .unwrap_or_else(|| panic!("{} not imported", t.path));
        if let Some(ms) = t.taken_at_ms {
            assert_eq!(p.taken_at.0, ms, "date of {}", t.path);
        }
        assert_eq!(p.favorite.value, t.favorite, "favourite of {}", t.path);
        if let Some((lat, lon)) = t.place {
            let gps = p
                .exif
                .gps
                .as_ref()
                .unwrap_or_else(|| panic!("no place for {}", t.path));
            assert_eq!(
                (gps.lat_e7, gps.lon_e7),
                ((lat * 1e7).round() as i32, (lon * 1e7).round() as i32)
            );
        }
    }
    assert_eq!(photos.keys().copied().collect::<BTreeSet<_>>(), want);

    // Albums: each title once, with exactly its photos.
    let mut albums: BTreeMap<String, BTreeSet<PhotoId>> = BTreeMap::new();
    for t in truth
        .iter()
        .filter(|t| t.copy_of.is_none() && !t.trashed && t.live_photo_of.is_none())
    {
        for a in &t.albums {
            albums.entry(a.clone()).or_default().insert(hash(&t.blake3));
        }
    }
    let summaries = store.albums().await.unwrap();
    assert_eq!(summaries.len(), albums.len(), "{summaries:?}");
    for (title, ids) in albums {
        let id = imported_album_id("google-takeout", &title);
        let s = summaries
            .iter()
            .find(|s| s.id == id)
            .unwrap_or_else(|| panic!("album {title}"));
        assert_eq!(s.name, title);
        let items: BTreeSet<PhotoId> = store
            .album_items(&id, Page::all())
            .await
            .unwrap()
            .into_iter()
            .collect();
        assert_eq!(items, ids, "photos of {title}");
    }
}

#[tokio::test]
async fn a_takeout_imports_with_its_dates_favourites_and_albums() {
    let r = rig().await;
    let (parts, truth) = export(&r);
    let report = r.ln.import_archives(&parts, opts(4)).await.unwrap();
    eprintln!("{report:#?}");
    assert!(report.part_errors.is_empty());
    assert!(
        report.import.failed.is_empty(),
        "{:?}",
        report.import.failed
    );
    assert_eq!(report.trashed, 2);
    assert_eq!(report.not_photos, 3, "archive_browser.html in each part");
    assert_eq!(report.unmatched + report.ambiguous, 0);
    assert!(report.copies >= 9);
    assert_eq!(
        report.export_dates, 2,
        "the screenshot and the re-dated scan"
    );
    assert_eq!(report.export_places, 1, "the screenshot");
    assert_eq!(report.import.with_video.len(), 1, "the Live Photo");
    check_library(&r, &truth).await;

    // Nothing is left in the temporary folder, and nothing was written next to the export.
    let tmp = r.dir.path().join("node/imports/tmp");
    assert_eq!(std::fs::read_dir(&tmp).unwrap().count(), 0);
    let export_files: BTreeSet<_> = std::fs::read_dir(r.dir.path().join("export"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert_eq!(
        export_files.len(),
        4,
        "three parts and truth.json: {export_files:?}"
    );

    // Again: everything is journalled, nothing is read, nothing changes.
    let again = r.ln.import_archives(&parts, opts(4)).await.unwrap();
    assert!(again.import.imported.is_empty() && again.import.duplicates.is_empty());
    assert_eq!(
        again.resumed,
        report.import.by_path.len() + report.import.skipped.len()
    );
    check_library(&r, &truth).await;
}

#[tokio::test]
async fn a_run_stopped_half_way_continues_where_it_stopped() {
    let r = rig().await;
    let (parts, truth) = export(&r);
    let mut events = r.ln.node().events();
    let ln = r.ln.clone();
    let p = parts.clone();
    let task = tokio::spawn(async move { ln.import_archives(&p, opts(1)).await });
    let mut stopped_at = 0;
    while let Ok(ev) = events.recv().await {
        if let NodeEvent::ImportProgress { done, .. } = ev {
            if done >= 10 {
                task.abort();
                stopped_at = done;
                break;
            }
        }
    }
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(stopped_at >= 10);

    let report: ArchiveImportReport = r.ln.import_archives(&parts, opts(4)).await.unwrap();
    assert!(
        report.resumed >= stopped_at,
        "{} resumed, stopped at {stopped_at}",
        report.resumed
    );
    let read =
        report.import.by_path.len() + report.import.skipped.len() + report.import.failed.len();
    let first = report.resumed;
    assert!(read > 0 && first > 0);
    eprintln!("stopped at {stopped_at}; resumed {first}, read {read}");
    check_library(&r, &truth).await;
}

#[tokio::test]
async fn missing_and_truncated_parts_are_named_and_the_rest_imports() {
    let r = rig().await;
    let (mut parts, _) = export(&r);
    let cut = parts[2].clone();
    let bytes = std::fs::read(&cut).unwrap();
    std::fs::write(&cut, &bytes[..bytes.len() / 2]).unwrap();
    parts.push(r.dir.path().join("export/takeout-20241027T033320Z-004.zip"));
    let report = r.ln.import_archives(&parts, opts(4)).await.unwrap();
    let named: Vec<String> = report
        .part_errors
        .iter()
        .map(|(p, _)| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        named,
        [
            "takeout-20241027T033320Z-003.zip",
            "takeout-20241027T033320Z-004.zip"
        ]
    );
    assert!(!report.import.imported.is_empty());
    assert!(
        report.import.failed.is_empty(),
        "{:?}",
        report.import.failed
    );
}

#[tokio::test]
async fn a_folder_without_sidecars_imports_as_plain_files() {
    let r = rig().await;
    let report = r.ln.import_archives(&[samples()], opts(4)).await.unwrap();
    assert_eq!(report.not_photos, 0);
    assert!(report.import.imported.len() >= 10, "{report:#?}");
    assert!(report.albums.is_empty());
}

#[tokio::test]
async fn the_plan_says_what_an_import_will_do_and_writes_nothing() {
    let r = rig().await;
    let (parts, _) = export(&r);
    let plan = r.ln.plan_archives(&parts).await.unwrap();
    assert!(plan.is_takeout);
    assert_eq!(plan.trashed, 2);
    assert_eq!(plan.not_photos, 3);
    assert_eq!(plan.already, 0);
    assert!(plan.photos > 0 && plan.copies >= 9);
    assert_eq!(plan.albums.len(), 3, "{:?}", plan.albums);
    assert!(plan.albums.iter().all(|(_, n)| *n > 0));
    assert!(plan.needed_bytes > 512 << 20);
    assert!(
        !r.dir.path().join("node/imports").exists(),
        "a plan writes nothing"
    );

    // After a full run everything is already done, and the plan says so.
    let report = r.ln.import_archives(&parts, opts(4)).await.unwrap();
    let after = r.ln.plan_archives(&parts).await.unwrap();
    assert_eq!(after.photos, 0);
    assert_eq!(after.already, plan.photos);
    assert_eq!(
        after.already,
        report.import.by_path.len() + report.import.skipped.len()
    );
}

#[tokio::test]
async fn a_plan_names_unreadable_parts_and_fails_when_none_can_be_read() {
    let r = rig().await;
    let (mut parts, _) = export(&r);
    parts.push(r.dir.path().join("export/missing.zip"));
    let plan = r.ln.plan_archives(&parts).await.unwrap();
    assert_eq!(plan.part_errors.len(), 1);
    let none = r.ln.plan_archives(&[r.dir.path().join("nope.zip")]).await;
    assert!(none.is_err());
}
