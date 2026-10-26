//! The `status` and `devices` reports. The JSON shape is documented in `docs/keeper-cli.md`.

use crate::daemon::Shared;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use shoebox_core::model::Role;
use shoebox_core::store::{LibraryStore, Page, TimeRange};
use shoebox_node::PathKind;

pub const SCHEMA_VERSION: u32 = 1;

fn role_name(r: Role) -> &'static str {
    match r {
        Role::Keeper => "keeper",
        Role::Phone => "phone",
        Role::Viewer => "viewer",
    }
}

pub(crate) async fn devices(shared: &Shared) -> Result<Value> {
    let guard = shared.library.lock().await;
    let ln = guard
        .as_ref()
        .ok_or_else(|| anyhow!("not paired with a library yet"))?;
    let list = ln.store().device_list().await?;
    let presence = ln.devices();
    let me = ln.device();
    let rows: Vec<Value> = list
        .devices
        .iter()
        .map(|d| {
            let online = d.device == me || presence.iter().any(|p| p.id == d.device && p.online);
            json!({
                "id": hex::encode(d.device.0),
                "name": d.name,
                "role": role_name(d.role),
                "online": online,
                "revoked": d.revoked,
                "admin": list.admins.iter().any(|a| a.admin == d.device),
                "self": d.device == me,
            })
        })
        .collect();
    Ok(json!({ "devices": rows }))
}

pub(crate) async fn report(shared: &Shared) -> Result<Value> {
    let mut out = json!({
        "schema": SCHEMA_VERSION,
        "version": env!("CARGO_PKG_VERSION"),
        "name": shared.opts.name,
        "device_id": hex::encode(shared.node.id().as_bytes()),
        "uptime_secs": shared.started.elapsed().as_secs(),
        "paired": false,
    });
    let guard = shared.library.lock().await;
    let Some(ln) = guard.as_ref() else {
        return Ok(out);
    };
    let photos = ln
        .store()
        .photos_by_time(TimeRange::ALL, Page::all())
        .await?;
    let mut held = 0usize;
    for p in &photos {
        if shared.node.blobs().has(&p.original.hash).await? {
            held += 1;
        }
    }
    let at_risk = ln.at_risk(Page::all()).await?.len();
    let membership = format!("{:?}", ln.membership().await?).to_lowercase();
    let devices = ln.store().device_list().await?;
    let mut peers = vec![];
    for d in devices
        .devices
        .iter()
        .filter(|d| !d.revoked && d.device != ln.device())
    {
        let Ok(id) = shoebox_node::to_iroh_id(&d.device) else {
            continue;
        };
        let (path, latency) = match shared.node.peer_info(id).await {
            Some(info) => (
                match info.path {
                    PathKind::Lan => "lan",
                    PathKind::Direct => "direct",
                    PathKind::Relay { .. } => "relay",
                    PathKind::Unknown => "none",
                },
                info.latency.map(|l| l.as_millis() as u64),
            ),
            None => ("none", None),
        };
        peers.push(json!({ "id": hex::encode(d.device.0), "name": d.name, "path": path, "latency_ms": latency }));
    }
    out["paired"] = json!(true);
    out["alerts"] = json!(crate::alerts::load(&shared.opts.storage));
    out["membership"] = json!(membership);
    out["library"] = json!({
        "id": hex::encode(ln.library_id().0),
        "photos": photos.len(),
        "held_originals": held,
        "at_risk": at_risk,
        "redundancy_target": ln.redundancy_target().await?,
        "recovery_kit_saved": ln.recovery_kit_saved().await?,
    });
    out["devices"] = json!(devices.devices.iter().filter(|d| !d.revoked).count());
    out["peers"] = json!(peers);
    out["shared_albums"] = albums_json(shared).await?["albums"].clone();
    Ok(out)
}

async fn albums_json(shared: &Shared) -> Result<Value> {
    let guard = shared.albums.lock().await;
    let Some(albums) = guard.as_ref() else {
        return Ok(json!({ "albums": [] }));
    };
    let rows: Vec<Value> = albums
        .list()
        .await?
        .into_iter()
        .map(|a| {
            json!({
                "id": hex::encode(a.id.0),
                "name": a.name,
                "photos": a.item_count,
                "members": a.member_count,
                "keep_originals": a.keep_originals,
            })
        })
        .collect();
    Ok(json!({ "albums": rows }))
}

/// `album list|keep|drop|join`.
pub(crate) async fn album(shared: &Shared, req: &Value) -> Result<Value> {
    if shared.library.lock().await.is_none() {
        return Err(anyhow!("not paired with a library yet"));
    }
    let action = req["action"].as_str().unwrap_or("list");
    if action == "list" {
        return albums_json(shared).await;
    }
    let guard = shared.albums.lock().await;
    let albums = guard
        .as_ref()
        .ok_or_else(|| anyhow!("shared albums are not ready yet"))?;
    if action == "join" {
        let text = req["text"]
            .as_str()
            .ok_or_else(|| anyhow!("missing `text`"))?;
        let id = albums.join(text, &shared.opts.name).await?;
        let name = albums
            .list()
            .await?
            .into_iter()
            .find(|a| a.id == id)
            .map(|a| a.name)
            .unwrap_or_default();
        return Ok(json!({ "id": hex::encode(id.0), "name": name }));
    }
    // keep / drop: the id may be shortened to a unique prefix.
    let want = req["id"]
        .as_str()
        .ok_or_else(|| anyhow!("missing `id`"))?
        .to_lowercase();
    let all = albums.list().await?;
    let hits: Vec<_> = all
        .iter()
        .filter(|a| hex::encode(a.id.0).starts_with(&want))
        .collect();
    let target = match hits.as_slice() {
        [one] => *one,
        [] => return Err(anyhow!("no album with the id `{want}` (see `album list`)")),
        _ => {
            return Err(anyhow!(
                "`{want}` matches several albums: use more of the id"
            ))
        }
    };
    match action {
        "keep" => albums.set_keep_originals(&target.id, true).await?,
        "drop" => albums.set_keep_originals(&target.id, false).await?,
        other => return Err(anyhow!("unknown album action `{other}`")),
    }
    Ok(json!({ "id": hex::encode(target.id.0), "name": target.name }))
}
