//! Where a synced Loro replica's memory goes (M4-11): one document of photo-sized and
//! holding-sized records, exported and imported into a second document the way a phone receives
//! a library, measured in live heap bytes after each step.
//!
//!     cargo run --release -p store-bench --bin loro-memory -- 100000
//!     cargo run --release -p store-bench --bin loro-memory -- --dir <replicas>

#[path = "../counting.rs"]
mod counting;

use counting::{live, reset_peak, PEAK};
use loro::{ExportMode, LoroDoc};
use std::sync::atomic::Ordering;

/// Encoded and signed sizes seen in the production backend (≈ 435 B a record on disk).
const PHOTO_VALUE: usize = 480;
const HOLDING_VALUE: usize = 150;

fn key(kind: &str, i: usize) -> String {
    format!(
        "0011223344556677/{kind}/{}",
        blake3::hash(&i.to_le_bytes()).to_hex()
    )
}

/// Incompressible bytes, as signatures and hashes are.
fn value(i: usize, len: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(len + 32);
    let mut k = 0u64;
    while v.len() < len {
        v.extend_from_slice(blake3::hash(&[i as u64, k].map(u64::to_le_bytes).concat()).as_bytes());
        k += 1;
    }
    v.truncate(len);
    v
}

fn mb(b: u64) -> f64 {
    b as f64 / 1e6
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--dir") {
        let dir = std::env::args().nth(2).expect("a directory");
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            if e.path().is_dir() {
                load_dir(&e.path());
            }
        }
        return;
    }
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(100_000);
    let per = |b: u64| b as f64 / n as f64;

    let base = live();
    let a = LoroDoc::new();
    let map = a.get_map("entries");
    for i in 0..n {
        map.insert(&key("p", i), value(i, PHOTO_VALUE)).unwrap();
        map.insert(&key("h", i), value(i, HOLDING_VALUE)).unwrap();
        if i % 5_000 == 4_999 {
            a.commit();
        }
    }
    a.commit();
    let a_live = live() - base;
    let snapshot = a.export(ExportMode::Snapshot).unwrap();
    let updates = a.export(ExportMode::all_updates()).unwrap();
    drop(a);
    println!(
        "writer doc: {:.0} B/photo; snapshot {:.1} MB, all updates {:.1} MB",
        per(a_live),
        mb(snapshot.len() as u64),
        mb(updates.len() as u64)
    );

    for (what, bytes) in [("snapshot", &snapshot), ("updates", &updates)] {
        let base = live();
        reset_peak();
        let b = LoroDoc::new();
        let before = b.state_frontiers();
        b.import(bytes).unwrap();
        let imported = live() - base;
        let peak_import = PEAK.load(Ordering::Relaxed) - base;
        let after = b.state_frontiers();
        reset_peak();
        let diff = b.diff(&before, &after).unwrap();
        let mut keys = 0usize;
        for (_, d) in diff.iter() {
            if let loro::event::Diff::Map(m) = d {
                keys += m.updated.len();
            }
        }
        let peak_diff = PEAK.load(Ordering::Relaxed) - base;
        drop(diff);
        let diffed = live() - base;
        b.free_diff_calculator();
        b.free_history_cache();
        let freed = live() - base;
        let upd = b.export(ExportMode::all_updates()).unwrap();
        let after_updates_export = live() - base - upd.len() as u64;
        drop(upd);
        println!(
            "  after exporting all updates {:.0} B/photo",
            per(after_updates_export)
        );
        // What a replica opened from its own snapshot holds (the restart case); the export
        // itself is what compaction does after a sync.
        let snap = b.export(ExportMode::Snapshot).unwrap();
        let exported = live() - base - snap.len() as u64;
        b.compact_change_store();
        let compacted = live() - base - snap.len() as u64;
        // Opening from the update form instead.
        let all = b.export(ExportMode::all_updates()).unwrap();
        drop(b);
        let base3 = live();
        let t = std::time::Instant::now();
        let u = LoroDoc::new();
        u.import(&all).unwrap();
        let _ = u.get_map("entries").len();
        let from_updates = live() - base3;
        let t_updates = t.elapsed();
        drop(u);
        drop(all);
        let base2 = live();
        let t = std::time::Instant::now();
        let c = LoroDoc::new();
        c.import(&snap).unwrap();
        let reopened = live() - base2;
        let _ = c.get_map("entries").len();
        let touched = live() - base2;
        let t_snap = t.elapsed();
        drop(c);
        println!(
            "from {what}: {keys} keys; imported {:.0} B/photo (peak {:.0} MB), after diff {:.0} (peak {:.0} MB), \
             caches freed {:.0}, after a snapshot export {:.0}, change store compacted {:.0}; \
             reopened from snapshot {:.0}, after reading the map {:.0} ({t_snap:.1?}); \
             reopened from all updates {:.0} ({t_updates:.1?})",
            per(imported),
            mb(peak_import),
            per(diffed),
            mb(peak_diff),
            per(freed),
            per(exported),
            per(compacted),
            per(reopened),
            per(touched),
            per(from_updates),
        );
    }
}

/// `loro-memory --dir <dir>`: what a replica's own files cost once loaded (each subdirectory of
/// `dir` is one replica; `SB_KEEP_B=<dir> store-bench --backend loro-net` leaves one there).
fn load_dir(dir: &std::path::Path) {
    let base = live();
    let doc = LoroDoc::new();
    let snap = std::fs::read(dir.join("snapshot.loro")).unwrap_or_default();
    if !snap.is_empty() {
        doc.import(&snap).unwrap();
    }
    let after_snap = live() - base;
    let raw = std::fs::read(dir.join("updates.log")).unwrap_or_default();
    let (mut at, mut frames) = (0usize, 0usize);
    while raw.len() - at >= 12 {
        let len = u32::from_le_bytes(raw[at..at + 4].try_into().unwrap()) as usize;
        doc.import(&raw[at + 12..at + 12 + len]).unwrap();
        at += 12 + len;
        frames += 1;
    }
    let after_log = live() - base;
    let map = doc.get_map("entries");
    let mut n = 0;
    map.for_each(|_, _| n += 1);
    let after_read = live() - base;
    println!(
        "snapshot {:.1} MB, log {:.1} MB in {frames} frames, {n} keys; live after snapshot {:.1} MB, after log {:.1} MB, after reading {:.1} MB",
        mb(snap.len() as u64),
        mb(raw.len() as u64),
        mb(after_snap),
        mb(after_log),
        mb(after_read)
    );
}
