//! Import from an export's archive parts (I1-04) with what Google Takeout's sidecars and folders say
//! (I1-05; the Google half of I1-07): dates, places, favourites and albums.
//!
//! Nothing is unpacked next to the archive. Each entry is streamed into a temporary file under
//! `<data dir>/imports/tmp`, analysed and copied into the blob store like any imported file, and
//! the temporary file is deleted; at most a few exist at a time. A journal
//! (`<data dir>/imports/<run>.journal`, one JSON line per finished entry) lets the same parts be
//! imported again after a crash or a closed laptop: finished entries are not read again, and a
//! second complete run adds nothing.

use crate::blobs::ImportMode;
use crate::import::{
    is_companion_clip, ImportOptions, ImportReport, HOLDINGS_BATCH, PROGRESS_EVERY,
};
use crate::library_node::LibraryNode;
use crate::node::NodeEvent;
use anyhow::{Context, Result};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use shoebox_core::hlc::Hlc;
use shoebox_core::ids::{Blake3Hash, Timestamp};
use shoebox_core::merge::{Lww, OrSet};
use shoebox_core::model::{AlbumId, AlbumMeta, BlobKind, Gps, PhotoId};
use shoebox_core::store::LibraryStore;
use shoebox_media::archive::ArchiveSet;
use shoebox_media::takeout::{self, Left, PlanItem, TakeoutPlan};
use shoebox_media::{analyze, AnalyzeError, ImportResult, TakenAtSource};
use std::collections::{BTreeSet, HashMap};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Room left on the disk after an import, on top of what the import needs.
const SPACE_MARGIN: u64 = 512 << 20;

/// The disk does not have room for the import.
#[derive(Debug, thiserror::Error)]
#[error(
    "not enough space: the import needs {needed} bytes (and {margin} to spare) but {available} are free at {at}",
    margin = SPACE_MARGIN,
    at = .at.display()
)]
pub struct NotEnoughSpace {
    pub needed: u64,
    pub available: u64,
    pub at: PathBuf,
}

#[derive(Clone, Debug, Default)]
pub struct ArchiveImportReport {
    /// Files in and out, as a folder import reports them; a path is `<part's file name>/<entry>`.
    pub import: ImportReport,
    /// Parts that could not be opened or listed (missing, truncated), with the reason.
    pub part_errors: Vec<(PathBuf, String)>,
    /// Entries an earlier run of these same parts had finished: not read again.
    pub resumed: usize,
    /// Photos left out because they are in the trash.
    pub trashed: usize,
    /// Copies of a photo in other folders (its albums), not read a second time.
    pub copies: usize,
    /// Files that are not part of Google Photos in a Takeout (other products, its own HTML page).
    pub not_photos: usize,
    /// Albums the export holds and how many photos each has in the library after this run.
    pub albums: Vec<(String, usize)>,
    /// New photos whose date came from the export (no date of their own, or re-dated by hand).
    pub export_dates: usize,
    /// New photos whose place came from the export.
    pub export_places: usize,
    /// Photos the export marks as favourites.
    pub favorites: usize,
    /// Media files with no sidecar, and those whose sidecar was claimed by several files.
    pub unmatched: usize,
    pub ambiguous: usize,
    /// Sidecars and other JSON files nothing used.
    pub unused_json: usize,
    /// The journal of this set of parts.
    pub journal: PathBuf,
}

/// One finished entry, as journalled.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
struct Line {
    part: String,
    path: String,
    size: u64,
    crc: Option<u32>,
    /// The photo it became (or, for a Live Photo's video, the photo it went with).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    photo: Option<String>,
    /// Why it was left out (not media this app understands).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    skipped: Option<String>,
}

type Key = (String, String, u64, Option<u32>);

fn part_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// The run's id: the same parts (by name and size, in any order) are the same run.
fn run_id(parts: &[PathBuf]) -> String {
    let mut named: Vec<(String, u64)> = parts
        .iter()
        .map(|p| {
            (
                part_name(p),
                std::fs::metadata(p).map(|m| m.len()).unwrap_or(0),
            )
        })
        .collect();
    named.sort();
    let mut h = blake3::Hasher::new();
    h.update(b"shoebox/import-run/v1");
    for (n, s) in named {
        h.update(n.as_bytes());
        h.update(&[0]);
        h.update(&s.to_le_bytes());
    }
    hex::encode(&h.finalize().as_bytes()[..12])
}

/// The id of an album an import makes: the same export's album (by kind and title) is filled
/// again by a second run or a second export of the same account, not made twice.
pub fn imported_album_id(kind: &str, title: &str) -> AlbumId {
    let mut h = blake3::Hasher::new();
    h.update(b"shoebox/import-album/v1");
    h.update(kind.as_bytes());
    h.update(&[0]);
    h.update(title.as_bytes());
    let mut id = [0u8; 16];
    id.copy_from_slice(&h.finalize().as_bytes()[..16]);
    AlbumId(id)
}

fn read_journal(path: &Path) -> HashMap<Key, Line> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return HashMap::new();
    };
    // A line cut short by a crash is ignored: that entry is simply done again.
    text.lines()
        .filter_map(|l| serde_json::from_str::<Line>(l).ok())
        .map(|l| ((l.part.clone(), l.path.clone(), l.size, l.crc), l))
        .collect()
}

fn gps_of((lat, lon): (f64, f64)) -> Gps {
    Gps {
        lat_e7: (lat * 1e7).round() as i32,
        lon_e7: (lon * 1e7).round() as i32,
    }
}

/// Bytes free for this process on the disk that holds `path`; `None` where this build cannot
/// tell (Windows, for now: the import then goes ahead and a full disk fails it like any write).
pub fn available_space(path: &Path) -> std::io::Result<Option<u64>> {
    #[cfg(unix)]
    {
        let st = rustix::fs::statvfs(path)?;
        Ok(Some(st.f_bavail.saturating_mul(st.f_frsize)))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(None)
    }
}

/// The date and place the export gives a photo that is new to the library (owner's decisions:
/// the file's own date wins unless the export's differs by more than a time zone; the export's
/// place fills in only where the file has none). Returns whether each was used.
fn apply_export(r: &mut ImportResult, item: &PlanItem) -> (bool, bool) {
    let own = item.sidecar.is_some_and(|(_, rule)| rule.is_own());
    let export = item.meta.taken_at_ms;
    let file = (r.taken_at_source == TakenAtSource::Exif).then_some(r.taken_at.0);
    // A date borrowed from another file (an edit's original, a Live Photo's picture) only fills
    // in; it never overrides the file's own.
    let date = if own {
        takeout::resolve_taken_at(file, export)
    } else {
        file.or(export)
    };
    let mut used_date = false;
    if let Some(ms) = date {
        if Some(ms) != file {
            r.taken_at = Timestamp(ms);
            r.taken_at_source = TakenAtSource::Exif;
            used_date = true;
        }
    }
    let mut used_place = false;
    if r.exif.gps.is_none() && own {
        if let Some(place) = item.meta.place {
            r.exif.gps = Some(gps_of(place));
            used_place = true;
        }
    }
    (used_date, used_place)
}

struct Run {
    set: Arc<ArchiveSet>,
    plan: TakeoutPlan,
    part_names: Vec<String>,
    journal_path: PathBuf,
    journal: std::fs::File,
    done: HashMap<Key, Line>,
    tmp: PathBuf,
}

impl Run {
    fn key(&self, entry: usize) -> Key {
        let e = &self.set.entries()[entry].entry;
        (
            self.part_names[e.part].clone(),
            e.path.clone(),
            e.size,
            e.crc32,
        )
    }

    fn label(&self, entry: usize) -> PathBuf {
        let e = &self.set.entries()[entry].entry;
        PathBuf::from(&self.part_names[e.part]).join(&e.path)
    }

    fn write(
        &mut self,
        entry: usize,
        photo: Option<PhotoId>,
        skipped: Option<String>,
    ) -> Result<()> {
        let (part, path, size, crc) = self.key(entry);
        let line = Line {
            part,
            path,
            size,
            crc,
            photo: photo.map(|p| hex::encode(p.0)),
            skipped,
        };
        let mut text = serde_json::to_string(&line)?;
        text.push('\n');
        self.journal
            .write_all(text.as_bytes())
            .with_context(|| format!("writing {}", self.journal_path.display()))?;
        self.done.insert(self.key(entry), line);
        Ok(())
    }

    fn photo_of(&self, entry: usize) -> Option<PhotoId> {
        let hex = self.done.get(&self.key(entry))?.photo.as_ref()?;
        let bytes: [u8; 32] = hex::decode(hex).ok()?.try_into().ok()?;
        Some(Blake3Hash(bytes))
    }
}

/// Copy entries of one part into temporary files, in the part's order, handing each over as it
/// is written; `tx` is bounded, so only a few exist at a time.
fn extract_part(
    set: &ArchiveSet,
    part: usize,
    wanted: &BTreeSet<usize>,
    tmp: &Path,
    tx: tokio::sync::mpsc::Sender<(usize, std::io::Result<PathBuf>)>,
) -> std::io::Result<()> {
    set.visit(part, wanted, |i, reader| {
        let e = &set.entries()[i];
        let ext = e
            .entry
            .path
            .rsplit_once('.')
            .map(|(_, x)| x.to_ascii_lowercase())
            .filter(|x| x.len() <= 8 && x.chars().all(|c| c.is_ascii_alphanumeric()))
            .unwrap_or_default();
        let path = tmp.join(format!("{i}.{ext}"));
        let written = (|| {
            let mut file = std::fs::File::create(&path)?;
            std::io::copy(reader, &mut file)?;
            if let Some(ms) = e.modified_ms.and_then(|ms| u64::try_from(ms).ok()) {
                // The file's date if it has none of its own: the archive's, not the time of import.
                let _ =
                    file.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_millis(ms));
            }
            file.sync_data()?;
            Ok(path.clone())
        })();
        if written.is_err() {
            let _ = std::fs::remove_file(&path);
        }
        // The receiver is gone when the import stopped: stop reading too.
        tx.blocking_send((i, written))
            .map_err(|_| std::io::Error::other("import stopped"))
    })
}

/// Open and list the parts and plan the import over them: the set, the plan and the parts that
/// could not be read.
async fn open_and_plan(
    parts: &[PathBuf],
) -> Result<(ArchiveSet, TakeoutPlan, Vec<(PathBuf, String)>)> {
    let owned: Vec<PathBuf> = parts.to_vec();
    let (set, plan, errors) = tokio::task::spawn_blocking(move || {
        let (set, errors) = ArchiveSet::open(&owned);
        let entries = set.takeout_entries();
        let json: BTreeSet<usize> = (0..entries.len())
            .filter(|&i| entries[i].path.to_lowercase().ends_with(".json"))
            .collect();
        let bytes = set.read_small(&json);
        let plan = takeout::plan(&entries, |i| bytes.get(&i).cloned());
        (set, plan, errors)
    })
    .await?;
    Ok((
        set,
        plan,
        errors.into_iter().map(|e| (e.part, e.error)).collect(),
    ))
}

/// What an import of these parts would do, from reading their listings and sidecars only.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ArchivePlanSummary {
    /// Photos and videos still to read.
    pub photos: usize,
    /// Finished by an earlier run of these parts, not read again.
    pub already: usize,
    /// Copies in other folders, not read a second time.
    pub copies: usize,
    /// Photos left out because they are in the trash.
    pub trashed: usize,
    /// Files that are not photos or videos (other products, the export's own page).
    pub not_photos: usize,
    /// Albums, with the photos each holds.
    pub albums: Vec<(String, usize)>,
    /// Photos with a sidecar that gives them a date, place or favourite.
    pub with_export_data: usize,
    pub unmatched: usize,
    pub ambiguous: usize,
    /// Parts that could not be opened or listed, with the reason.
    pub part_errors: Vec<(PathBuf, String)>,
    /// What reading them needs on disk, and what is free there (`None`: not known here).
    pub needed_bytes: u64,
    pub free_bytes: Option<u64>,
    /// The parts have the layout of a Google Takeout (sidecars or album folders).
    pub is_takeout: bool,
}

impl<L: LibraryStore> LibraryNode<L> {
    /// What importing these parts would do, without writing anything: the counts, the albums,
    /// and the space needed against the space free.
    pub async fn plan_archives(&self, parts: &[PathBuf]) -> Result<ArchivePlanSummary> {
        let (set, plan, part_errors) = open_and_plan(parts).await?;
        let listed: Vec<PathBuf> = set.parts().map(Path::to_path_buf).collect();
        anyhow::ensure!(
            !listed.is_empty(),
            "none of the parts could be read: {}",
            part_errors
                .iter()
                .map(|(p, e)| format!("{}: {e}", p.display()))
                .collect::<Vec<_>>()
                .join("; ")
        );
        let data_dir = self.node().config().data_dir;
        let journal = read_journal(
            &data_dir
                .join("imports")
                .join(format!("{}.journal", run_id(&listed))),
        );
        let names: Vec<String> = listed.iter().map(|p| part_name(p)).collect();
        let mut s = ArchivePlanSummary {
            part_errors,
            unmatched: plan.unmatched.len(),
            ambiguous: plan.ambiguous.len(),
            copies: plan.items.iter().map(|i| i.entries.len() - 1).sum(),
            is_takeout: !plan.albums.is_empty() || plan.items.iter().any(|i| i.sidecar.is_some()),
            ..Default::default()
        };
        for (_, why) in &plan.left {
            match why {
                Left::Trashed => s.trashed += 1,
                Left::NotPhotos => s.not_photos += 1,
                Left::UnreadableJson(_) | Left::UnusedSidecar => {}
            }
        }
        let mut sizes = 0u64;
        for item in &plan.items {
            let first = &set.entries()[item.entries[0]].entry;
            let key = (
                names[first.part].clone(),
                first.path.clone(),
                first.size,
                first.crc32,
            );
            if journal.contains_key(&key) {
                s.already += 1;
            } else {
                s.photos += 1;
                sizes += first.size;
            }
            if item.sidecar.is_some() {
                s.with_export_data += 1;
            }
        }
        s.needed_bytes = sizes.saturating_add(SPACE_MARGIN);
        s.free_bytes = available_space(&data_dir).ok().flatten();
        let mut counts = vec![0usize; plan.albums.len()];
        for item in &plan.items {
            for &a in &item.albums {
                counts[a] += 1;
            }
        }
        s.albums = plan
            .albums
            .iter()
            .map(|a| a.title.clone())
            .zip(counts)
            .collect();
        Ok(s)
    }

    /// Import an export: zip or tgz parts, or unpacked folders. A Google Takeout brings its
    /// dates, places, favourites and albums; any other archive imports as its files. Running it
    /// again with the same parts continues where the last run stopped.
    pub async fn import_archives(
        &self,
        parts: &[PathBuf],
        opts: ImportOptions,
    ) -> Result<ArchiveImportReport> {
        let opts = ImportOptions {
            // An archive's entries have no file to reference: always copied.
            mode: ImportMode::Copy,
            ..opts
        };
        let data_dir = self.node().config().data_dir;
        let imports = data_dir.join("imports");
        let tmp = imports.join("tmp");
        std::fs::create_dir_all(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
        // Leftovers of a run that was killed.
        for e in std::fs::read_dir(&tmp)?.flatten() {
            let _ = std::fs::remove_file(e.path());
        }

        let mut report = ArchiveImportReport::default();
        let (set, plan, errors) = open_and_plan(parts).await?;
        report.part_errors = errors;
        let listed: Vec<PathBuf> = set.parts().map(Path::to_path_buf).collect();
        anyhow::ensure!(
            !listed.is_empty(),
            "none of the parts could be read: {}",
            report
                .part_errors
                .iter()
                .map(|(p, e)| format!("{}: {e}", p.display()))
                .collect::<Vec<_>>()
                .join("; ")
        );
        let journal_path = imports.join(format!("{}.journal", run_id(&listed)));
        report.journal = journal_path.clone();
        let mut run = Run {
            part_names: listed.iter().map(|p| part_name(p)).collect(),
            set: Arc::new(set),
            plan,
            done: read_journal(&journal_path),
            journal: std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&journal_path)
                .with_context(|| format!("opening {}", journal_path.display()))?,
            journal_path,
            tmp,
        };

        for (_, why) in &run.plan.left {
            match why {
                Left::Trashed => report.trashed += 1,
                Left::NotPhotos => report.not_photos += 1,
                Left::UnreadableJson(_) | Left::UnusedSidecar => report.unused_json += 1,
            }
        }
        report.unmatched = run.plan.unmatched.len();
        report.ambiguous = run.plan.ambiguous.len();
        report.copies = run.plan.items.iter().map(|i| i.entries.len() - 1).sum();

        // What is left to read: the first copy of each item not journalled. Live Photo videos go
        // last, once their pictures are in.
        let mut stills = vec![];
        let mut clips = vec![];
        for (n, item) in run.plan.items.iter().enumerate() {
            let first = item.entries[0];
            if run.done.contains_key(&run.key(first)) {
                report.resumed += 1;
            } else if item.live_photo_of.is_some() {
                clips.push(n);
            } else {
                stills.push(n);
            }
        }
        let needed: u64 = stills
            .iter()
            .chain(&clips)
            .map(|&n| run.set.entries()[run.plan.items[n].entries[0]].entry.size)
            .sum();
        let available = available_space(&data_dir)
            .with_context(|| format!("reading the free space at {}", data_dir.display()))?;
        if let Some(available) = available.filter(|&a| needed.saturating_add(SPACE_MARGIN) > a) {
            return Err(NotEnoughSpace {
                needed,
                available,
                at: data_dir,
            }
            .into());
        }

        let total = stills.len() + clips.len();
        let mut progress = 0usize;
        let mut held: Vec<(Blake3Hash, BlobKind)> = vec![];
        for (phase, items) in [stills, clips].into_iter().enumerate() {
            for (part, part_path) in listed.iter().enumerate() {
                let by_entry: HashMap<usize, usize> = items
                    .iter()
                    .map(|&n| (run.plan.items[n].entries[0], n))
                    .filter(|(e, _)| run.set.entries()[*e].entry.part == part)
                    .collect();
                if by_entry.is_empty() {
                    continue;
                }
                let wanted: BTreeSet<usize> = by_entry.keys().copied().collect();
                let concurrency = opts.concurrency.max(1);
                let (tx, mut rx) = tokio::sync::mpsc::channel(concurrency);
                let (set, dir) = (run.set.clone(), run.tmp.clone());
                let reader = tokio::task::spawn_blocking(move || {
                    extract_part(&set, part, &wanted, &dir, tx)
                });
                let mut analysed = futures::stream::poll_fn(move |cx| rx.poll_recv(cx))
                    .map(|(entry, written)| async move {
                        let result = match written {
                            Ok(path) => {
                                let p = path.clone();
                                let r = tokio::task::spawn_blocking(move || analyze(&p))
                                    .await
                                    .unwrap_or_else(|e| {
                                        Err(AnalyzeError::Decode(format!(
                                            "analysis task failed: {e}"
                                        )))
                                    });
                                (Some(path), r)
                            }
                            Err(e) => (None, Err(AnalyzeError::Io(e))),
                        };
                        (entry, result)
                    })
                    .buffered(concurrency);
                while let Some((entry, (temp, result))) = analysed.next().await {
                    let n = by_entry[&entry];
                    let label = run.label(entry);
                    let outcome = self
                        .take_entry(
                            &mut run,
                            n,
                            entry,
                            &label,
                            temp.as_deref(),
                            result,
                            phase,
                            &opts,
                            &mut held,
                            &mut report,
                        )
                        .await;
                    if let Some(temp) = temp {
                        let _ = std::fs::remove_file(temp);
                    }
                    outcome?;
                    progress += 1;
                    if held.len() >= HOLDINGS_BATCH {
                        self.publish_holdings(std::mem::take(&mut held)).await?;
                    }
                    if progress.is_multiple_of(PROGRESS_EVERY) || progress == total {
                        let r = &report.import;
                        self.node().emit(NodeEvent::ImportProgress {
                            done: progress,
                            total,
                            imported: r.imported.len(),
                            duplicates: r.duplicates.len(),
                            skipped: r.skipped.len(),
                            failed: r.failed.len(),
                        });
                    }
                }
                match reader.await {
                    Ok(Ok(())) => {}
                    // The part could be listed but not read through (cut short, damaged): what
                    // was read is in; the rest is reported and a later run tries it again.
                    Ok(Err(e)) => report
                        .part_errors
                        .push((part_path.clone(), format!("reading stopped: {e}"))),
                    Err(e) => report
                        .part_errors
                        .push((part_path.clone(), format!("reading stopped: {e}"))),
                }
            }
        }
        self.publish_holdings(held).await?;

        // Albums, with every photo of theirs that is in the library (this run's and earlier ones').
        let mut members: Vec<Vec<PhotoId>> = vec![vec![]; run.plan.albums.len()];
        for item in &run.plan.items {
            if item.live_photo_of.is_some() {
                continue;
            }
            if let Some(photo) = run.photo_of(item.entries[0]) {
                for &a in &item.albums {
                    members[a].push(photo);
                }
            }
        }
        let store = self.store();
        for (album, mut ids) in run.plan.albums.iter().zip(members) {
            ids.sort_unstable();
            ids.dedup();
            if ids.is_empty() {
                report.albums.push((album.title.clone(), 0));
                continue;
            }
            let id = imported_album_id("google-takeout", &album.title);
            // Registers at GENESIS: a name the user gave the album since wins.
            store
                .upsert_album(AlbumMeta {
                    id,
                    name: Lww::new(album.title.clone(), Hlc::GENESIS),
                    cover: Lww::new(None, Hlc::GENESIS),
                    items: OrSet::new(),
                })
                .await?;
            store.album_add(&id, &ids).await?;
            report.albums.push((album.title.clone(), ids.len()));
        }
        if !report.import.imported.is_empty() {
            let _ = self
                .announce_new_items(report.import.imported.len() as u32)
                .await;
        }
        Ok(report)
    }

    /// Bring one analysed entry in, apply what the export says about it, and journal it.
    #[allow(clippy::too_many_arguments)]
    async fn take_entry(
        &self,
        run: &mut Run,
        n: usize,
        entry: usize,
        label: &Path,
        temp: Option<&Path>,
        result: Result<ImportResult, AnalyzeError>,
        phase: usize,
        opts: &ImportOptions,
        held: &mut Vec<(Blake3Hash, BlobKind)>,
        report: &mut ArchiveImportReport,
    ) -> Result<()> {
        let item = run.plan.items[n].clone();
        let mut r = match (temp, result) {
            (_, Err(AnalyzeError::Unsupported(why))) => {
                *report.import.skipped_why.entry(why.clone()).or_default() += 1;
                report.import.skipped.push(label.to_path_buf());
                return run.write(entry, None, Some(why));
            }
            // Not journalled: a later run tries it again.
            (_, Err(e)) => {
                report
                    .import
                    .failed
                    .push((label.to_path_buf(), e.to_string()));
                return Ok(());
            }
            (None, Ok(_)) => unreachable!("an analysed entry has its file"),
            (Some(_), Ok(r)) => r,
        };
        let temp = temp.expect("checked above");
        // A Live Photo's video goes with its picture when it is short enough to be one.
        if phase == 1 {
            let still = item
                .live_photo_of
                .and_then(|s| run.photo_of(run.plan.items[s].entries[0]));
            if let (Some(photo), true) = (still, is_companion_clip(&r)) {
                match self.attach_clip(temp, &r, photo, opts, held).await {
                    Ok(added) => {
                        if added {
                            report.import.with_video.push(photo);
                        }
                        report.import.by_path.push((label.to_path_buf(), photo));
                        return run.write(entry, Some(photo), None);
                    }
                    Err(e) => {
                        report
                            .import
                            .failed
                            .push((label.to_path_buf(), format!("{e:#}")));
                        return Ok(());
                    }
                }
            }
        }
        let (date, place) = apply_export(&mut r, &item);
        let Some((photo, new)) = self
            .ingest_as(temp, label, &r, opts, held, &mut report.import)
            .await
        else {
            return Ok(());
        };
        if new {
            report.export_dates += usize::from(date);
            report.export_places += usize::from(place);
        }
        if item.meta.favorite {
            self.store().set_favorite(&photo, true).await?;
            report.favorites += 1;
        }
        run.write(entry, Some(photo), None)
    }
}
