//! Where the bookkeeping is kept: small records under path-like keys (`libs/<id>/lib.postcard`,
//! `shares/<id>/meta.postcard`, `reports/<id>`, `uploads/<token>`, ...). On a server that is a data directory
//! ([`DirMeta`]); in a Cloudflare Worker it is a Durable Object's SQLite database (`apps/cloud-worker`).
//!
//! Every call is synchronous and small: the store keeps everything in memory and writes a record when it changes.

use std::io;

pub trait Meta: Send + Sync {
    /// The record under `key`, with every line [`Meta::append`]ed to it since it was last [`Meta::put`].
    fn get(&self, key: &str) -> io::Result<Option<Vec<u8>>>;
    /// Replace the record under `key`.
    fn put(&self, key: &str, value: &[u8]) -> io::Result<()>;
    /// Add a line (a newline is added) to the record under `key`, creating it.
    fn append(&self, key: &str, line: &str) -> io::Result<()>;
    /// Delete the record under `key`; deleting what is not there is not an error.
    fn delete(&self, key: &str) -> io::Result<()>;
    /// The names one level below `dir`: `children("libs")` lists library ids, `children("reports")` share ids.
    fn children(&self, dir: &str) -> io::Result<Vec<String>>;
}

#[cfg(feature = "server")]
pub use dir::DirMeta;

#[cfg(feature = "server")]
mod dir {
    use super::Meta;
    use std::io::{self, Write};
    use std::path::{Path, PathBuf};

    /// Records as files in a data directory; a key is a path below it.
    pub struct DirMeta {
        root: PathBuf,
    }

    impl DirMeta {
        pub fn new(root: &Path) -> Self {
            Self {
                root: root.to_path_buf(),
            }
        }
        fn path(&self, key: &str) -> io::Result<PathBuf> {
            if key
                .split('/')
                .any(|p| p.is_empty() || p == "." || p == ".." || p.contains('\\'))
            {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "a bad key"));
            }
            Ok(self.root.join(key))
        }
    }

    pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, path)
    }

    impl Meta for DirMeta {
        fn get(&self, key: &str) -> io::Result<Option<Vec<u8>>> {
            match std::fs::read(self.path(key)?) {
                Ok(b) => Ok(Some(b)),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e),
            }
        }
        fn put(&self, key: &str, value: &[u8]) -> io::Result<()> {
            write_atomic(&self.path(key)?, value)
        }
        fn append(&self, key: &str, line: &str) -> io::Result<()> {
            let path = self.path(key)?;
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)?;
            writeln!(f, "{line}")
        }
        fn delete(&self, key: &str) -> io::Result<()> {
            let path = self.path(key)?;
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
            // A share's directory goes with its last file.
            if let Some(dir) = path.parent() {
                if dir != self.root {
                    let _ = std::fs::remove_dir(dir);
                }
            }
            Ok(())
        }
        fn children(&self, dir: &str) -> io::Result<Vec<String>> {
            let rd = match std::fs::read_dir(self.path(dir)?) {
                Ok(rd) => rd,
                Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(vec![]),
                Err(e) => return Err(e),
            };
            let mut out = vec![];
            for e in rd {
                let name = e?.file_name().to_string_lossy().into_owned();
                if !name.ends_with(".tmp") {
                    out.push(name);
                }
            }
            Ok(out)
        }
    }
}

/// Records in memory, for tests of the store without a disk.
#[derive(Default)]
pub struct MemMeta {
    map: std::sync::Mutex<std::collections::BTreeMap<String, Vec<u8>>>,
}

impl Meta for MemMeta {
    fn get(&self, key: &str) -> io::Result<Option<Vec<u8>>> {
        Ok(self
            .map
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
            .cloned())
    }
    fn put(&self, key: &str, value: &[u8]) -> io::Result<()> {
        self.map
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key.to_string(), value.to_vec());
        Ok(())
    }
    fn append(&self, key: &str, line: &str) -> io::Result<()> {
        let mut m = self.map.lock().unwrap_or_else(|e| e.into_inner());
        let v = m.entry(key.to_string()).or_default();
        v.extend_from_slice(line.as_bytes());
        v.push(b'\n');
        Ok(())
    }
    fn delete(&self, key: &str) -> io::Result<()> {
        self.map
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(key);
        Ok(())
    }
    fn children(&self, dir: &str) -> io::Result<Vec<String>> {
        let prefix = format!("{dir}/");
        let m = self.map.lock().unwrap_or_else(|e| e.into_inner());
        let mut out: Vec<String> = m
            .keys()
            .filter_map(|k| k.strip_prefix(&prefix))
            .map(|rest| rest.split('/').next().unwrap_or(rest).to_string())
            .collect();
        out.dedup();
        Ok(out)
    }
}
