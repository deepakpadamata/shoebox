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
    "set_device_role",
    "make_admin",
    "answer_recovery",
    "replace_recovery_phrase",
    "dismiss_alert",
    "health",
    "free_up_space",
    "settings",
    "set_redundancy_target",
    "set_relay",
    "rename_device",
    "watch_folder",
    "export_library",
    "rename_album",
    "set_poster",
    "backup_status",
    "backup_configure",
    "backup_run",
    "sync_burst",
    "shared_albums",
    "shared_create",
    "shared_invite",
    "shared_join",
    "shared_leave",
    "shared_items",
    "shared_members",
    "shared_feed",
    "shared_add_check",
    "shared_add",
    "shared_save",
    "shared_settings",
    "shared_seen",
    "shared_fetch",
    "shared_remove_member",
    "diagnostics",
    "device_link",
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
        "replace_recovery_phrase" => out(core
            .replace_recovery_phrase(&arg::<String>(&args, "phrase")?)
            .await?),
        "answer_recovery" => {
            core.answer_recovery(arg(&args, "accept")?)?;
            Ok(json!({}))
        }
        "dismiss_alert" => {
            core.dismiss_alert(&arg::<String>(&args, "device_id")?)?;
            Ok(json!({}))
        }
        "make_admin" => {
            core.make_admin(
                &arg::<String>(&args, "id")?,
                &arg::<String>(&args, "phrase")?,
            )
            .await?;
            Ok(json!({}))
        }
        "set_device_role" => {
            let phrase = opt::<String>(&args, "phrase")?;
            core.set_device_role(
                &arg::<String>(&args, "id")?,
                &arg::<String>(&args, "role")?,
                phrase.as_deref(),
            )
            .await?;
            Ok(json!({}))
        }
        "revoke_device" => {
            let phrase = opt::<String>(&args, "phrase")?;
            core.revoke_device(&arg::<String>(&args, "id")?, phrase.as_deref())
                .await?;
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
        "rename_album" => {
            core.rename_album(
                &arg::<String>(&args, "album")?,
                &arg::<String>(&args, "name")?,
            )
            .await?;
            Ok(json!({}))
        }
        "set_poster" => {
            core.set_poster(&arg::<String>(&args, "id")?, &arg::<String>(&args, "data")?)
                .await?;
            Ok(json!({}))
        }
        "backup_status" => out(core.backup_status().await?),
        "backup_configure" => out(core.backup_configure(arg(&args, "policy")?).await?),
        "backup_run" => out(core.backup_run(true).await?),
        "sync_burst" => out(core
            .sync_burst(std::time::Duration::from_secs(
                opt::<u64>(&args, "seconds")?.unwrap_or(25),
            ))
            .await?),
        "shared_albums" => out(core.shared_albums().await?),
        "shared_create" => out(core.shared_create(&arg::<String>(&args, "name")?).await?),
        "shared_invite" => out(core
            .shared_invite(
                &arg::<String>(&args, "album")?,
                &arg::<String>(&args, "role")?,
                opt::<u32>(&args, "days")?.unwrap_or(7),
            )
            .await?),
        "shared_join" => out(core.shared_join(&arg::<String>(&args, "text")?).await?),
        "shared_leave" => {
            core.shared_leave(&arg::<String>(&args, "album")?).await?;
            Ok(json!({}))
        }
        "shared_items" => out(core
            .shared_items(
                &arg::<String>(&args, "album")?,
                opt::<String>(&args, "after")?,
                opt::<u32>(&args, "limit")?.unwrap_or(500),
            )
            .await?),
        "shared_members" => out(core.shared_members(&arg::<String>(&args, "album")?).await?),
        "shared_feed" => out(core.shared_feed(&arg::<String>(&args, "album")?).await?),
        "shared_add_check" => out(core
            .shared_add_check(
                &arg::<String>(&args, "album")?,
                &arg::<Vec<String>>(&args, "ids")?,
            )
            .await?),
        "shared_add" => out(core
            .shared_add(
                &arg::<String>(&args, "album")?,
                &arg::<Vec<String>>(&args, "ids")?,
                opt::<String>(&args, "location")?,
                opt::<bool>(&args, "remember")?.unwrap_or(false),
            )
            .await?),
        "shared_save" => out(core
            .shared_save(
                &arg::<String>(&args, "album")?,
                &arg::<Vec<String>>(&args, "ids")?,
            )
            .await?),
        "shared_settings" => {
            core.shared_settings(
                &arg::<String>(&args, "album")?,
                opt::<bool>(&args, "keep_originals")?,
                opt::<String>(&args, "location")?,
            )
            .await?;
            Ok(json!({}))
        }
        "shared_seen" => {
            core.shared_seen(&arg::<String>(&args, "album")?).await?;
            Ok(json!({}))
        }
        "shared_fetch" => {
            core.shared_fetch(
                &arg::<String>(&args, "album")?,
                &arg::<String>(&args, "id")?,
                &arg::<String>(&args, "kind")?,
            )
            .await?;
            Ok(json!({}))
        }
        "shared_remove_member" => {
            core.shared_remove_member(
                &arg::<String>(&args, "album")?,
                &arg::<String>(&args, "member")?,
            )
            .await?;
            Ok(json!({}))
        }
        "device_link" => out(core.device_link()),
        "diagnostics" => out(core.diagnostics().await?),
        "export_library" => out(core.export_library(&arg::<String>(&args, "dest")?).await?),
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
