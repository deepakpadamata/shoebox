//! Bytes in the data directory, when there is no bucket (a server build only).

use crate::store::Cid;
use std::collections::BTreeMap;
use std::io;
use std::path::Path;

/// Create the directories and drop uploads that were in flight when the service stopped (they are not resumable).
pub(crate) fn prepare(dir: &Path) -> io::Result<()> {
    for sub in ["libs", "shares", "tmp"] {
        std::fs::create_dir_all(dir.join(sub))?;
    }
    for e in std::fs::read_dir(dir.join("tmp"))? {
        let _ = std::fs::remove_file(e?.path());
    }
    Ok(())
}

/// What a library holds on disk, from its `blobs/` and `objects/` directories.
pub(crate) fn scan_library(
    dir: &Path,
    lib: &[u8; 16],
    blobs: &mut BTreeMap<Cid, u64>,
    objects: &mut BTreeMap<String, u64>,
) -> io::Result<()> {
    let lib_dir = dir.join("libs").join(hex::encode(lib));
    for (sub, is_blob) in [("blobs", true), ("objects", false)] {
        let Ok(rd) = std::fs::read_dir(lib_dir.join(sub)) else {
            continue;
        };
        for f in rd {
            let f = f?;
            let name = f.file_name().to_string_lossy().into_owned();
            let len = f.metadata()?.len();
            if is_blob {
                if let Some(cid) = hex::decode(&name).ok().and_then(|b| b.try_into().ok()) {
                    blobs.insert(cid, len);
                }
            } else {
                objects.insert(name, len);
            }
        }
    }
    Ok(())
}
