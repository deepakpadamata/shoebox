//! The Android camera roll as a [`PhotoSource`]: a thin client of the Kotlin `PhotoLibraryPlugin`
//! (MediaStore, permissions, battery and network state).

use crate::photos_source::{Asset, Conditions, Permission, PhotoSource, Picked, Scan};
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
struct FrameReply {
    jpeg: String,
}

#[derive(Deserialize)]
struct PickReply {
    items: Vec<Picked>,
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

/// Device secrets wrapped by the Android Keystore (M4-02).
#[derive(Debug)]
pub struct AndroidKeystore(pub PluginHandle<Wry>);

#[derive(Deserialize)]
struct SecretReply {
    hex: Option<String>,
}

impl crate::keystore::SecretStore for AndroidKeystore {
    fn load(&self, name: &str) -> Result<Option<[u8; 32]>> {
        let r: SecretReply = self
            .0
            .run_mobile_plugin("secretLoad", json!({ "name": name }))
            .map_err(|e| anyhow!("{e}"))?;
        match r.hex {
            None => Ok(None),
            Some(h) => {
                Ok(Some(hex::decode(h)?.try_into().map_err(|_| {
                    anyhow!("the stored secret has the wrong length")
                })?))
            }
        }
    }

    fn store(&self, name: &str, secret: &[u8; 32]) -> Result<()> {
        self.0
            .run_mobile_plugin::<serde_json::Value>(
                "secretStore",
                json!({ "name": name, "hex": hex::encode(secret) }),
            )
            .map(|_| ())
            .map_err(|e| anyhow!("{e}"))
    }

    fn load_or_create(&self, name: &str) -> Result<[u8; 32]> {
        if let Some(s) = self.load(name)? {
            return Ok(s);
        }
        let mut s = [0u8; 32];
        getrandom::fill(&mut s).expect("operating system randomness");
        self.store(name, &s)?;
        Ok(s)
    }

    fn describe(&self) -> String {
        "the Android Keystore".into()
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
    fn pick(&self, files: bool) -> BoxFuture<'_, Result<Vec<Picked>>> {
        Box::pin(async move {
            let source = if files { "files" } else { "gallery" };
            let r: PickReply = self.call("pickMedia", json!({ "source": source })).await?;
            Ok(r.items)
        })
    }
    fn read_picked(&self, handle: &str) -> BoxFuture<'_, Result<PathBuf>> {
        let uri = handle.to_string();
        Box::pin(async move {
            let r: PathReply = self.call("readPicked", json!({ "uri": uri })).await?;
            Ok(PathBuf::from(r.path))
        })
    }
    fn video_frame(&self, path: &str) -> BoxFuture<'_, Result<Option<Vec<u8>>>> {
        let path = path.to_string();
        Box::pin(async move {
            let r: FrameReply = self.call("videoFrame", json!({ "path": path })).await?;
            crate::photos_source::decode_frame(&r.jpeg)
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
