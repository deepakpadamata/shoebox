//! Crash-safe storage of a Loro document: `snapshot.loro` plus an append-only `updates.log`.
//!
//! * Every local commit and every import appends the exported update to the log (framed with a
//!   length and a checksum and flushed to disk) before the caller sees success.
//! * Opening imports the snapshot, then replays the log up to the first damaged frame (a torn
//!   tail from a crash is truncated away).
//! * Compaction writes a fresh full snapshot to a temp file, renames it over the old one and
//!   truncates the log. A crash between those two steps just replays updates the snapshot
//!   already contains, which Loro ignores.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const SNAPSHOT: &str = "snapshot.loro";
const LOG: &str = "updates.log";
/// Compact when the log exceeds this and is bigger than the snapshot.
const COMPACT_MIN_LOG: u64 = 8 * 1024 * 1024;
/// Longest a logged update may wait for its `fsync`.
pub(crate) const FLUSH_INTERVAL: std::time::Duration = std::time::Duration::from_millis(200);

/// What opening a store yields: the store, its snapshot (if any) and the logged updates.
pub(crate) type Opened = (Persist, Option<Vec<u8>>, Vec<Vec<u8>>);

pub(crate) struct Persist {
    dir: PathBuf,
    log: File,
    log_len: u64,
    snapshot_len: u64,
    /// Bytes were appended since the last `fsync`.
    unsynced: bool,
    last_sync: std::time::Instant,
}

fn frame_sum(bytes: &[u8]) -> [u8; 8] {
    blake3::hash(bytes).as_bytes()[..8]
        .try_into()
        .expect("8 bytes")
}

impl Persist {
    /// Open (creating) the store in `dir`; returns what to load: the snapshot, then the updates.
    pub(crate) fn open(dir: &Path) -> io::Result<Opened> {
        std::fs::create_dir_all(dir)?;
        let snapshot = match std::fs::read(dir.join(SNAPSHOT)) {
            Ok(b) => Some(b),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(e),
        };
        let mut log = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(dir.join(LOG))?;
        let mut raw = vec![];
        log.seek(SeekFrom::Start(0))?;
        log.read_to_end(&mut raw)?;
        let mut updates = vec![];
        let mut at = 0usize;
        while raw.len() - at >= 12 {
            let len = u32::from_le_bytes(raw[at..at + 4].try_into().expect("4")) as usize;
            let end = at + 12 + len;
            if end > raw.len() || raw[at + 4..at + 12] != frame_sum(&raw[at + 12..end]) {
                break;
            }
            updates.push(raw[at + 12..end].to_vec());
            at = end;
        }
        if at < raw.len() {
            // A torn or damaged tail: drop it so new frames follow valid ones.
            log.set_len(at as u64)?;
            log.sync_all()?;
        }
        let snapshot_len = snapshot.as_ref().map_or(0, |s| s.len() as u64);
        Ok((
            Persist {
                dir: dir.to_path_buf(),
                log,
                log_len: at as u64,
                snapshot_len,
                unsynced: false,
                last_sync: std::time::Instant::now(),
            },
            snapshot,
            updates,
        ))
    }

    pub(crate) fn append(&mut self, update: &[u8]) -> io::Result<()> {
        let mut frame = Vec::with_capacity(12 + update.len());
        frame.extend_from_slice(&(update.len() as u32).to_le_bytes());
        frame.extend_from_slice(&frame_sum(update));
        frame.extend_from_slice(update);
        self.log.write_all(&frame)?;
        self.log_len += frame.len() as u64;
        self.unsynced = true;
        // Group commit: a burst of writes shares one fsync. The library's flusher thread syncs
        // what is left within `FLUSH_INTERVAL`, so a crash loses at most that much local work.
        if self.last_sync.elapsed() >= FLUSH_INTERVAL {
            self.flush()?;
        }
        Ok(())
    }

    /// Make everything appended so far durable.
    pub(crate) fn flush(&mut self) -> io::Result<()> {
        if self.unsynced {
            self.log.sync_data()?;
            self.unsynced = false;
        }
        self.last_sync = std::time::Instant::now();
        Ok(())
    }

    pub(crate) fn should_compact(&self) -> bool {
        self.log_len > COMPACT_MIN_LOG && self.log_len > self.snapshot_len
    }

    /// Replace the snapshot with `snapshot` (a full export of the document) and empty the log.
    pub(crate) fn compact(&mut self, snapshot: &[u8]) -> io::Result<()> {
        let tmp = self.dir.join("snapshot.tmp");
        {
            let mut f = File::create(&tmp)?;
            f.write_all(snapshot)?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, self.dir.join(SNAPSHOT))?;
        if let Ok(d) = File::open(&self.dir) {
            let _ = d.sync_all();
        }
        self.log.set_len(0)?;
        self.log.sync_all()?;
        self.unsynced = false;
        self.log_len = 0;
        self.snapshot_len = snapshot.len() as u64;
        Ok(())
    }

    pub(crate) fn sizes(&self) -> (u64, u64) {
        (self.snapshot_len, self.log_len)
    }
}

impl Drop for Persist {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}
