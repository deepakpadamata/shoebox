//! Where the device's secrets live (M3-02).
//!
//! The endpoint secret (and nothing else: the library root key is never stored) belongs in the
//! operating system's keystore. [`KeyringStore`] uses it (macOS Keychain, Windows Credential
//! Manager, the Secret Service on Linux); [`FileStore`] is the fallback and the dev default.
//! [`migrate_file_key`] moves a key written by an earlier run into the store and wipes the file.

#[cfg(feature = "keyring")]
use anyhow::Context;
use anyhow::{anyhow, bail, Result};
use std::fmt::Debug;
use std::path::{Path, PathBuf};

pub trait SecretStore: Send + Sync + Debug {
    /// The 32-byte secret called `name`, created (randomly) on first use.
    fn load_or_create(&self, name: &str) -> Result<[u8; 32]>;
    /// The secret if it exists.
    fn load(&self, name: &str) -> Result<Option<[u8; 32]>>;
    fn store(&self, name: &str, secret: &[u8; 32]) -> Result<()>;
    /// Short human-readable description, for the settings screen ("macOS Keychain").
    fn describe(&self) -> String;
}

fn random() -> [u8; 32] {
    let mut s = [0u8; 32];
    getrandom::fill(&mut s).expect("operating system randomness");
    s
}

/// Keys as files under a directory (mode 0600). Used when no keystore is available, with a warning.
#[derive(Debug)]
pub struct FileStore {
    dir: PathBuf,
}

impl FileStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.key"))
    }
}

impl FileStore {
    /// Overwrite and delete a stored secret.
    pub fn wipe(&self, name: &str) -> Result<()> {
        let path = self.path(name);
        if path.exists() {
            std::fs::write(&path, [0u8; 32])?;
            std::fs::remove_file(path)?;
        }
        Ok(())
    }
}

/// Move a secret from the file store into a real keystore and wipe the file (a device that ran
/// before the keystore existed keeps its identity). Never replaces a different key that is
/// already in the keystore.
pub fn migrate_from_files(files: &FileStore, to: &dyn SecretStore, name: &str) -> Result<bool> {
    let Some(secret) = files.load(name)? else {
        return Ok(false);
    };
    match to.load(name)? {
        Some(existing) if existing != secret => {
            bail!("the keystore already holds a different device key; not overwriting it")
        }
        Some(_) => {}
        None => to.store(name, &secret)?,
    }
    files.wipe(name)?;
    Ok(true)
}

impl SecretStore for FileStore {
    fn load(&self, name: &str) -> Result<Option<[u8; 32]>> {
        match std::fs::read(self.path(name)) {
            Ok(b) => Ok(Some(b.try_into().map_err(|_| {
                anyhow!("{} is damaged (not 32 bytes)", self.path(name).display())
            })?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn store(&self, name: &str, secret: &[u8; 32]) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let path = self.path(name);
        std::fs::write(&path, secret)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }

    fn load_or_create(&self, name: &str) -> Result<[u8; 32]> {
        if let Some(s) = self.load(name)? {
            return Ok(s);
        }
        let s = random();
        self.store(name, &s)?;
        Ok(s)
    }

    fn describe(&self) -> String {
        format!("a file in {} (no system keystore)", self.dir.display())
    }
}

/// The operating system keystore.
#[cfg(feature = "keyring")]
#[derive(Debug)]
pub struct KeyringStore {
    service: String,
}

#[cfg(feature = "keyring")]
impl KeyringStore {
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }
    fn entry(&self, name: &str) -> Result<keyring::Entry> {
        keyring::Entry::new(&self.service, name).context("opening the keystore entry")
    }
    /// Whether the keystore works here (it may not, for example, on a headless Linux box).
    pub fn available(&self) -> bool {
        self.entry("probe")
            .and_then(|e| match e.get_password() {
                Ok(_) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(e) => Err(anyhow!("{e}")),
            })
            .is_ok()
    }
}

#[cfg(feature = "keyring")]
impl SecretStore for KeyringStore {
    fn load(&self, name: &str) -> Result<Option<[u8; 32]>> {
        match self.entry(name)?.get_password() {
            Ok(hex_text) => {
                let bytes = hex::decode(hex_text.trim()).context("keystore entry is not hex")?;
                Ok(Some(bytes.try_into().map_err(|_| {
                    anyhow!("keystore entry has the wrong length")
                })?))
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => bail!("reading the keystore: {e}"),
        }
    }

    fn store(&self, name: &str, secret: &[u8; 32]) -> Result<()> {
        self.entry(name)?
            .set_password(&hex::encode(secret))
            .map_err(|e| anyhow!("writing the keystore: {e}"))
    }

    fn load_or_create(&self, name: &str) -> Result<[u8; 32]> {
        if let Some(s) = self.load(name)? {
            return Ok(s);
        }
        let s = random();
        self.store(name, &s)?;
        Ok(s)
    }

    fn describe(&self) -> String {
        if cfg!(target_os = "macos") {
            "macOS Keychain".into()
        } else if cfg!(target_os = "windows") {
            "Windows Credential Manager".into()
        } else {
            "the Secret Service".into()
        }
    }
}

/// Move the endpoint key a previous run left in `node_dir/keys/endpoint.key` into `store`
/// under `name` and delete the file. Returns whether a file was migrated. Refuses to overwrite
/// a different key already in the store (that would silently change the device identity).
pub fn migrate_file_key(node_dir: &Path, store: &dyn SecretStore, name: &str) -> Result<bool> {
    let path = node_dir.join("keys").join("endpoint.key");
    let Ok(bytes) = std::fs::read(&path) else {
        return Ok(false);
    };
    let secret: [u8; 32] = bytes
        .try_into()
        .map_err(|_| anyhow!("{} is damaged (not 32 bytes)", path.display()))?;
    match store.load(name)? {
        Some(existing) if existing != secret => {
            bail!("the keystore already holds a different device key; not overwriting it")
        }
        Some(_) => {}
        None => store.store(name, &secret)?,
    }
    // Overwrite before unlinking so the bytes do not linger in a trivially recoverable file.
    std::fs::write(&path, [0u8; 32])?;
    std::fs::remove_file(&path)?;
    Ok(true)
}
