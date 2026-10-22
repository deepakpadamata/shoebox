//! The command surface the UI sees: one name per `AppCore` operation, JSON in and out. Both the
//! Tauri shell (`invoke`) and the dev HTTP bridge (`POST /api/<command>`) go through
//! [`dispatch`], so a command added here exists in both.

use crate::core::AppCore;
use crate::types::PageCursor;
use anyhow::{anyhow, bail, Result};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::sync::Arc;

fn arg<T: DeserializeOwned>(args: &Value, name: &str) -> Result<T> {
    serde_json::from_value(args.get(name).cloned().unwrap_or(Value::Null))
        .map_err(|e| anyhow!("argument `{name}`: {e}"))
}

fn opt<T: DeserializeOwned>(args: &Value, name: &str) -> Result<Option<T>> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => serde_json::from_value(v.clone())
            .map(Some)
            .map_err(|e| anyhow!("argument `{name}`: {e}")),
    }
}

fn out<T: serde::Serialize>(v: T) -> Result<Value> {
    Ok(serde_json::to_value(v)?)
}

/// Every command name, for the UI's typed client and for tests.
pub const COMMANDS: &[&str] = &[
    "app_state",
    "create_library",
    "join",
    "recover",
    "confirm_pairing",
    "pair_start",
    "mark_recovery_kit_saved",
    "library_page",
    "counts",
    "photo_detail",
    "set_favorite",
    "delete",
    "restore",
    "recently_deleted",
    "import_paths",
    "pin_original",
    "fetch_now",
    "albums",
    "create_album",
    "album_add",
    "album_remove",
    "album_page",
    "devices",
    "revoke_device",
    "health",
    "free_up_space",
    "settings",
    "set_redundancy_target",
    "set_relay",
    "rename_device",
    "watch_folder",
];

pub async fn dispatch(core: &Arc<AppCore>, cmd: &str, args: Value) -> Result<Value> {
    match cmd {
        "app_state" => out(core.app_state().await?),
        "create_library" => out(core
            .create_library(
                &arg::<String>(&args, "device_name")?,
                &arg::<String>(&args, "role")?,
            )
            .await?),
        "join" => {
            core.join(
                &arg::<String>(&args, "ticket")?,
                &arg::<String>(&args, "device_name")?,
            )
            .await?;
            Ok(json!({}))
        }
        "recover" => {
            core.recover(
                &arg::<String>(&args, "link")?,
                &arg::<String>(&args, "phrase")?,
                &arg::<String>(&args, "device_name")?,
            )
            .await?;
            Ok(json!({}))
        }
        "confirm_pairing" => {
            core.confirm_pairing(arg(&args, "accept")?)?;
            Ok(json!({}))
        }
        "pair_start" => out(core.pair_start(&arg::<String>(&args, "role")?).await?),
        "mark_recovery_kit_saved" => {
            core.mark_recovery_kit_saved().await?;
            Ok(json!({}))
        }
        "library_page" => out(core
            .library_page(
                opt::<PageCursor>(&args, "cursor")?,
                opt::<u32>(&args, "limit")?.unwrap_or(200),
            )
            .await?),
        "counts" => out(core.counts().await?),
        "photo_detail" => out(core.photo_detail(&arg::<String>(&args, "id")?).await?),
        "set_favorite" => {
            core.set_favorite(&arg::<Vec<String>>(&args, "ids")?, arg(&args, "favorite")?)
                .await?;
            Ok(json!({}))
        }
        "delete" => {
            core.delete(&arg::<Vec<String>>(&args, "ids")?).await?;
            Ok(json!({}))
        }
        "restore" => {
            core.restore(&arg::<Vec<String>>(&args, "ids")?).await?;
            Ok(json!({}))
        }
        "recently_deleted" => out(core.recently_deleted().await?),
        "import_paths" => out(core
            .import_paths(&arg::<Vec<String>>(&args, "paths")?)
            .await?),
        "pin_original" => {
            core.pin_original(&arg::<String>(&args, "id")?, arg(&args, "pin")?)
                .await?;
            Ok(json!({}))
        }
        "fetch_now" => {
            core.fetch_now(&arg::<String>(&args, "id")?, &arg::<String>(&args, "kind")?)
                .await?;
            Ok(json!({}))
        }
        "albums" => out(core.albums().await?),
        "create_album" => out(core.create_album(&arg::<String>(&args, "name")?).await?),
        "album_add" => {
            core.album_add(
                &arg::<String>(&args, "album")?,
                &arg::<Vec<String>>(&args, "ids")?,
            )
            .await?;
            Ok(json!({}))
        }
        "album_remove" => {
            core.album_remove(
                &arg::<String>(&args, "album")?,
                &arg::<Vec<String>>(&args, "ids")?,
            )
            .await?;
            Ok(json!({}))
        }
        "album_page" => out(core
            .album_page(
                &arg::<String>(&args, "album")?,
                opt::<PageCursor>(&args, "cursor")?,
                opt::<u32>(&args, "limit")?.unwrap_or(200),
            )
            .await?),
        "devices" => out(core.devices().await?),
        "revoke_device" => {
            core.revoke_device(&arg::<String>(&args, "id")?).await?;
            Ok(json!({}))
        }
        "health" => out(core.health().await?),
        "free_up_space" => out(core.free_up_space(arg(&args, "dry_run")?).await?),
        "settings" => out(core.settings().await?),
        "set_redundancy_target" => {
            core.set_redundancy_target(arg(&args, "copies")?).await?;
            Ok(json!({}))
        }
        "set_relay" => {
            core.set_relay(&arg::<String>(&args, "relay")?).await?;
            Ok(json!({}))
        }
        "rename_device" => {
            core.rename_device(&arg::<String>(&args, "name")?).await?;
            Ok(json!({}))
        }
        "watch_folder" => {
            core.watch_folder(&arg::<String>(&args, "dir")?).await?;
            Ok(json!({}))
        }
        other => bail!("unknown command `{other}`"),
    }
}

/// A message fit for the user: the error chain's first line, without Rust jargon.
pub fn user_message(e: &anyhow::Error) -> String {
    let full = format!("{e:#}");
    let first = full.lines().next().unwrap_or("something went wrong");
    first
        .replace("backend error: ", "")
        .replace("invalid request: ", "")
}
