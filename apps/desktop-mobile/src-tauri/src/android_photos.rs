//! The Android camera roll as a [`PhotoSource`]: a thin client of the Kotlin `PhotoLibraryPlugin`
//! (MediaStore, permissions, battery and network state).

use crate::photos_source::{Asset, Conditions, Permission, PhotoSource, Scan};
use anyhow::{anyhow, Result};
use futures::future::BoxFuture;
use serde::Deserialize;
use serde_json::json;
use std::path::PathBuf;
use tauri::plugin::{Builder, PluginHandle, TauriPlugin};
use tauri::{Manager, Wry};

/// Managed state: the handle to the Kotlin plugin.
pub struct PhotoLibrary(pub PluginHandle<Wry>);

/// Register the plugin (call on the builder before `setup`).
pub fn init() -> TauriPlugin<Wry> {
    Builder::new("photo-library")
        .setup(|app, api| {
            let handle =
                api.register_android_plugin("org.shoebox.desktop", "PhotoLibraryPlugin")?;
            app.manage(PhotoLibrary(handle));
            Ok(())
        })
        .build()
}

pub struct AndroidPhotoSource(pub PluginHandle<Wry>);

#[derive(Deserialize)]
struct PermissionReply {
    state: String,
}

#[derive(Deserialize)]
struct ScanReply {
    assets: Vec<Asset>,
    removed: Vec<String>,
    token: String,
}

#[derive(Deserialize)]
struct PathReply {
    path: String,
}

#[derive(Deserialize)]
struct ConditionsReply {
    on_wifi: bool,
    charging: bool,
    battery_pct: u8,
}

impl AndroidPhotoSource {
    async fn call<T: serde::de::DeserializeOwned + Send + 'static>(
        &self,
        command: &'static str,
        payload: serde_json::Value,
    ) -> Result<T> {
        let handle = self.0.clone();
        tokio::task::spawn_blocking(move || {
            handle
                .run_mobile_plugin::<T>(command, payload)
                .map_err(|e| anyhow!("{e}"))
        })
        .await?
    }
}

fn permission(state: &str) -> Permission {
    match state {
        "granted" => Permission::Granted,
        "limited" => Permission::Limited,
        _ => Permission::Denied,
    }
}

impl PhotoSource for AndroidPhotoSource {
    fn permission(&self) -> BoxFuture<'_, Result<Permission>> {
        Box::pin(async move {
            let r: PermissionReply = self.call("permission", json!({})).await?;
            Ok(permission(&r.state))
        })
    }
    fn request_permission(&self) -> BoxFuture<'_, Result<Permission>> {
        Box::pin(async move {
            let r: PermissionReply = self.call("requestPermission", json!({})).await?;
            Ok(permission(&r.state))
        })
    }
    fn scan(&self, token: Option<String>) -> BoxFuture<'_, Result<Scan>> {
        Box::pin(async move {
            let r: ScanReply = self.call("scan", json!({ "token": token })).await?;
            Ok(Scan {
                assets: r.assets,
                removed: r.removed,
                token: r.token,
            })
        })
    }
    fn read_original(&self, id: &str) -> BoxFuture<'_, Result<PathBuf>> {
        let id = id.to_string();
        Box::pin(async move {
            let r: PathReply = self.call("readOriginal", json!({ "id": id })).await?;
            Ok(PathBuf::from(r.path))
        })
    }
    fn conditions(&self) -> BoxFuture<'_, Result<Conditions>> {
        Box::pin(async move {
            let r: ConditionsReply = self.call("conditions", json!({})).await?;
            Ok(Conditions {
                on_wifi: r.on_wifi,
                charging: r.charging,
                battery_pct: r.battery_pct,
            })
        })
    }
}
