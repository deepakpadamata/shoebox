//! Endpoint secret key persistence (file-based for now; platform keystores come in M3/M4).

use anyhow::{bail, Context, Result};
use std::path::Path;

/// Load the 32-byte secret at `path`, or generate and store a new one (mode `0600` on Unix).
pub fn load_or_create_secret(path: &Path) -> Result<iroh::SecretKey> {
    if path.exists() {
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        let bytes: [u8; 32] = match bytes.try_into() {
            Ok(b) => b,
            Err(_) => bail!("{} is not a 32-byte key", path.display()),
        };
        return Ok(iroh::SecretKey::from_bytes(&bytes));
    }
    let key = iroh::SecretKey::generate();
    write_private(path, &key.to_bytes())?;
    Ok(key)
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts
        .open(path)
        .with_context(|| format!("creating {}", path.display()))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_once_then_reloads_the_same_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("k");
        let a = load_or_create_secret(&path).unwrap();
        let b = load_or_create_secret(&path).unwrap();
        assert_eq!(a.to_bytes(), b.to_bytes());
    }

    #[test]
    fn rejects_a_corrupt_key_file_instead_of_regenerating() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("k");
        std::fs::write(&path, b"short").unwrap();
        assert!(load_or_create_secret(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"short", "left untouched");
    }
}
