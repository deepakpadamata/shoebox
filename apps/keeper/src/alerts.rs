//! Security alerts on the keeper (R1-06): a new admin device, or a replaced recovery phrase. The
//! keeper has no screen, so it keeps them in `alerts.json` until someone runs
//! `shoebox-keeper alerts ack`; `status` shows them first.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alert {
    /// `admin_added` or `phrase_replaced`.
    pub kind: String,
    /// The new admin (empty for `phrase_replaced`).
    pub device_id: String,
    pub device_name: String,
    pub at_ms: u64,
}

fn path(storage: &Path) -> PathBuf {
    storage.join("alerts.json")
}

pub fn load(storage: &Path) -> Vec<Alert> {
    std::fs::read(path(storage))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn add(storage: &Path, alert: Alert) {
    let mut all = load(storage);
    all.retain(|a| !(a.kind == alert.kind && a.device_id == alert.device_id));
    all.push(alert);
    if let Ok(bytes) = serde_json::to_vec_pretty(&all) {
        let tmp = storage.join("alerts.json.tmp");
        if std::fs::write(&tmp, bytes).is_ok() {
            let _ = std::fs::rename(tmp, path(storage));
        }
    }
}

pub fn clear(storage: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path(storage)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// What the alert says, for `status`.
pub fn describe(a: &Alert) -> String {
    if a.kind == "phrase_replaced" {
        "the library's recovery phrase was replaced on another device".into()
    } else {
        format!(
            "“{}” became an admin of the library: if that was not you, someone has the recovery phrase",
            a.device_name
        )
    }
}
