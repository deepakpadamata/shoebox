//! Finding a model file and checking it is the one this build was made for (S1-08).
//!
//! A file is used only if its size and BLAKE3 match the hash compiled into
//! `shoebox_search::models`, wherever it came from: the app's resources, next to the keeper, the
//! data folder (copied from the APK, or fetched from a peer) or `models/build` in a checkout.

use shoebox_search::models::ModelFile;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

/// Files already checked, by path and modification time (hashing 100 MB takes a moment).
static CHECKED: Mutex<Option<HashMap<PathBuf, (SystemTime, u64)>>> = Mutex::new(None);

/// Whether `path` holds exactly `file`.
pub fn verify(path: &Path, file: &ModelFile) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if meta.len() != file.size {
        return false;
    }
    let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    if let Some(m) = CHECKED.lock().unwrap().as_ref() {
        if m.get(path) == Some(&(mtime, meta.len())) {
            return true;
        }
    }
    let ok = std::fs::File::open(path)
        .and_then(|mut f| {
            let mut h = blake3::Hasher::new();
            std::io::copy(&mut f, &mut h)?;
            Ok(*h.finalize().as_bytes() == file.blake3)
        })
        .unwrap_or(false);
    if ok {
        CHECKED
            .lock()
            .unwrap()
            .get_or_insert_with(HashMap::new)
            .insert(path.to_path_buf(), (mtime, meta.len()));
    }
    ok
}

/// Whether `bytes` are exactly `file`.
pub fn verify_bytes(bytes: &[u8], file: &ModelFile) -> bool {
    bytes.len() as u64 == file.size && *blake3::hash(bytes).as_bytes() == file.blake3
}

/// Places a build keeps its model files, most specific first: `extra` (the app's resource and
/// data folders), next to the executable (`models/`, `../share/shoebox/models/`), then a
/// checkout's `models/build` (development).
pub fn search_dirs(extra: &[PathBuf]) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = extra.to_vec();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            dirs.push(dir.join("models"));
            dirs.push(dir.join("../share/shoebox/models"));
            // `target/<profile>/` in a checkout.
            dirs.push(dir.join("../../models/build"));
            dirs.push(dir.join("../../../models/build"));
        }
    }
    dirs.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models/build"));
    dirs
}

/// The first verified copy of `file` in `dirs`.
pub fn locate(file: &ModelFile, dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter()
        .map(|d| d.join(file.name))
        .find(|p| verify(p, file))
}
