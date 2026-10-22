//! Node configuration and on-disk layout.

use serde::{Deserialize, Serialize};
use shoebox_core::model::Role;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RelaySetting {
    /// No relay: direct and LAN connections only.
    Disabled,
    /// The public n0 relays and address lookup.
    Default,
    /// Your own relay server (see `docs/` for self-hosting); direct and LAN paths still work.
    Custom(String),
}

/// Everything a node needs to start. Persisted to `<data_dir>/config.toml` on open.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeConfig {
    #[serde(skip)]
    pub data_dir: PathBuf,
    /// Human-readable device name (shown in the devices list).
    pub name: String,
    pub role: Role,
    pub relay: RelaySetting,
    /// Discover devices on the local network with mDNS (works without internet).
    #[serde(default = "default_true")]
    pub mdns: bool,
    /// How often unprotected blobs are garbage-collected; `None` disables GC.
    #[serde(default = "default_gc_interval_ms")]
    pub gc_interval_ms: Option<u64>,
    /// Folders imported automatically whenever files appear in them.
    #[serde(default)]
    pub watched_folders: Vec<PathBuf>,
    /// Originals (photo ids, hex) the user pinned on this device; never replicated.
    #[serde(default)]
    pub pinned_originals: Vec<String>,
}

fn default_true() -> bool {
    true
}

fn default_gc_interval_ms() -> Option<u64> {
    Some(5 * 60 * 1000)
}

impl NodeConfig {
    /// A config with no relay and no discovery service, for tests and LAN-only use.
    pub fn local_only(data_dir: impl Into<PathBuf>, name: impl Into<String>) -> Self {
        Self {
            data_dir: data_dir.into(),
            name: name.into(),
            role: Role::Keeper,
            relay: RelaySetting::Disabled,
            // Off for tests and explicit local-only use: no multicast traffic, no cross-talk.
            mdns: false,
            gc_interval_ms: default_gc_interval_ms(),
            watched_folders: vec![],
            pinned_originals: vec![],
        }
    }

    pub fn with_gc_interval(mut self, interval: std::time::Duration) -> Self {
        self.gc_interval_ms = Some(interval.as_millis() as u64);
        self
    }

    pub fn blobs_dir(&self) -> PathBuf {
        self.data_dir.join("blobs")
    }
    pub fn store_dir(&self) -> PathBuf {
        self.data_dir.join("store")
    }
    pub fn keys_dir(&self) -> PathBuf {
        self.data_dir.join("keys")
    }
    pub fn config_path(&self) -> PathBuf {
        self.data_dir.join("config.toml")
    }
    pub fn key_path(&self) -> PathBuf {
        self.keys_dir().join("endpoint.key")
    }

    /// Create the directory layout.
    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        for dir in [self.blobs_dir(), self.store_dir(), self.keys_dir()] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }

    pub fn save(&self) -> anyhow::Result<()> {
        std::fs::write(self.config_path(), toml::to_string_pretty(self)?)?;
        Ok(())
    }

    pub fn load(data_dir: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(data_dir.join("config.toml"))?;
        let mut cfg: NodeConfig = toml::from_str(&text)?;
        cfg.data_dir = data_dir.to_path_buf();
        Ok(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_round_trips_through_toml() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = NodeConfig::local_only(dir.path(), "Pi");
        cfg.role = Role::Viewer;
        cfg.relay = RelaySetting::Default;
        cfg.ensure_dirs().unwrap();
        cfg.save().unwrap();
        assert_eq!(NodeConfig::load(dir.path()).unwrap(), cfg);
    }
}
