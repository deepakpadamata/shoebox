//! The local control socket: newline-delimited JSON over a Unix domain socket (mode 0600).
//!
//! One request per connection line: `{"cmd": "status" | "devices" | "revoke" | "pair" | "album" |
//! "shutdown", ...}`. Replies are `{"type":"ok","data":…}` or `{"type":"error","message":…}`.
//! `pair` is a short conversation: the daemon first sends `{"type":"code","sas":…,
//! "peer_name":…,"peer_role":…}`, the client answers `{"confirm":true|false}` and the final
//! `ok`/`error` follows.

use crate::daemon::Shared;
use crate::status;
use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use shoebox_core::ids::EndpointId;
use shoebox_core::store::LibraryStore;
use shoebox_node::{LibraryNode, LibraryTicket, PairTicket, PairingCode};
use shoebox_store::Library;
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, oneshot};

pub(crate) async fn serve(shared: Arc<Shared>, path: &Path) -> Result<tokio::task::JoinHandle<()>> {
    let _ = std::fs::remove_file(path);
    let listener =
        UnixListener::bind(path).with_context(|| format!("binding {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let shared = shared.clone();
            tokio::spawn(async move {
                if let Err(e) = handle(shared, stream).await {
                    tracing::debug!("control connection: {e:#}");
                }
            });
        }
    }))
}

async fn send(w: &mut (impl AsyncWriteExt + Unpin), v: Value) -> Result<()> {
    let mut line = serde_json::to_vec(&v)?;
    line.push(b'\n');
    w.write_all(&line).await?;
    Ok(())
}

async fn handle(shared: Arc<Shared>, stream: UnixStream) -> Result<()> {
    let (r, mut w) = stream.into_split();
    let mut lines = BufReader::new(r).lines();
    while let Some(line) = lines.next_line().await? {
        let request: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                send(
                    &mut w,
                    json!({"type":"error","message":format!("bad request: {e}")}),
                )
                .await?;
                continue;
            }
        };
        let cmd = request["cmd"].as_str().unwrap_or("").to_string();
        let reply = if cmd == "pair" {
            pair(&shared, &request, &mut lines, &mut w).await
        } else if cmd == "add_device" {
            add_device(&shared, &request, &mut lines, &mut w).await
        } else {
            dispatch(&shared, &cmd, &request).await
        };
        match reply {
            Ok(data) => send(&mut w, json!({"type":"ok","data":data})).await?,
            Err(e) => send(&mut w, json!({"type":"error","message":format!("{e:#}")})).await?,
        }
        if cmd == "shutdown" {
            break;
        }
    }
    Ok(())
}

/// The recovery phrase a request carries, if any.
fn phrase(req: &Value) -> Option<String> {
    req["phrase"]
        .as_str()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
}

async fn library(shared: &Arc<Shared>) -> Result<LibraryNode<Library>> {
    shared
        .library
        .lock()
        .await
        .clone()
        .ok_or_else(|| anyhow!("not paired with a library yet"))
}

/// A device named by its full id, a unique id prefix (as `devices` prints it) or its name.
/// Removed devices only match when `include_revoked`.
async fn resolve_device(
    ln: &LibraryNode<Library>,
    text: &str,
    include_revoked: bool,
) -> Result<EndpointId> {
    let text = text.trim();
    anyhow::ensure!(!text.is_empty(), "missing `device`");
    let list = ln.store().device_list().await?;
    let live: Vec<_> = list
        .devices
        .iter()
        .filter(|d| include_revoked || !d.revoked)
        .collect();
    let lower = text.to_ascii_lowercase();
    let mut hits: Vec<_> = live
        .iter()
        .filter(|d| hex::encode(d.device.0) == lower)
        .collect();
    if hits.is_empty() && lower.len() >= 4 && lower.chars().all(|c| c.is_ascii_hexdigit()) {
        hits = live
            .iter()
            .filter(|d| hex::encode(d.device.0).starts_with(&lower))
            .collect();
    }
    if hits.is_empty() {
        hits = live
            .iter()
            .filter(|d| d.name.to_ascii_lowercase() == lower)
            .collect();
    }
    match hits.as_slice() {
        [one] => Ok(one.device),
        [] => bail!("no device matches `{text}` (see `shoebox-keeper devices`)"),
        _ => bail!("`{text}` matches more than one device: use more of its id"),
    }
}

async fn dispatch(shared: &Arc<Shared>, cmd: &str, req: &Value) -> Result<Value> {
    match cmd {
        "status" => status::report(shared).await,
        "devices" => status::devices(shared).await,
        "alerts" => Ok(json!({ "alerts": crate::alerts::load(&shared.opts.storage) })),
        "alerts_ack" => {
            crate::alerts::clear(&shared.opts.storage)?;
            Ok(json!({}))
        }
        "revoke" => {
            let ln = library(shared).await?;
            let device = resolve_device(&ln, req["device"].as_str().unwrap_or(""), false).await?;
            // The library moves to a new epoch either way, so the removed device is locked out
            // of what is written next (the same as removing it in the app).
            match phrase(req) {
                Some(p) => ln.revoke_and_rotate_with_phrase(&p, &device).await?,
                None => ln.revoke_and_rotate(&device).await?,
            };
            Ok(json!({}))
        }
        "role" => {
            let ln = library(shared).await?;
            let device = resolve_device(&ln, req["device"].as_str().unwrap_or(""), false).await?;
            let role = match req["role"]
                .as_str()
                .unwrap_or("")
                .to_ascii_lowercase()
                .as_str()
            {
                "keeper" => shoebox_core::model::Role::Keeper,
                "companion" | "phone" => shoebox_core::model::Role::Companion,
                "viewer" => shoebox_core::model::Role::Viewer,
                other => bail!("a role is keeper, companion or viewer, not `{other}`"),
            };
            match phrase(req) {
                Some(p) => ln.set_device_role_with_phrase(&p, &device, role).await?,
                None => ln.set_device_role(&device, role).await?,
            }
            Ok(json!({}))
        }
        "make_admin" => {
            let ln = library(shared).await?;
            let device = resolve_device(&ln, req["device"].as_str().unwrap_or(""), false).await?;
            let p =
                phrase(req).ok_or_else(|| anyhow!("making an admin needs the recovery phrase"))?;
            ln.make_admin_with_phrase(&p, &device).await?;
            Ok(json!({}))
        }
        "replace_phrase" => {
            let ln = library(shared).await?;
            let p = phrase(req)
                .ok_or_else(|| anyhow!("replacing the recovery phrase needs the current one"))?;
            let kit = ln.rotate_root(&p).await?;
            Ok(json!({ "phrase": kit.phrase(), "kit": kit.to_text() }))
        }
        "address" => Ok(json!({
            "link": shoebox_node::PeerLink(shoebox_node::from_endpoint_addr(&shared.node.addr())).to_string(),
        })),
        "recover" => {
            if shared.library.lock().await.is_some() {
                bail!("this keeper is already paired with a library");
            }
            let link: shoebox_node::PeerLink = req["from"]
                .as_str()
                .ok_or_else(|| anyhow!("missing `from`"))?
                .parse()
                .map_err(|e| anyhow!("{e}"))?;
            let phrase = req["phrase"]
                .as_str()
                .ok_or_else(|| anyhow!("missing `phrase`"))?;
            let name = req["name"].as_str().unwrap_or(&shared.opts.name);
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let grant = shared
                .node
                .recover(&link, phrase, shoebox_core::model::Role::Keeper, name, now)
                .await?;
            let ticket = LibraryTicket {
                cap: grant.cap.clone(),
                peers: grant.peers.clone(),
            };
            shared.attach(ticket.clone()).await?;
            shared.save(&ticket)?;
            Ok(json!({ "library": hex::encode(grant.cap.library.0) }))
        }
        "album" => crate::status::album(shared, req).await,
        "shutdown" => {
            let _ = shared.shutdown.send(true);
            Ok(json!({}))
        }
        other => bail!("unknown command `{other}`"),
    }
}

async fn pair(
    shared: &Arc<Shared>,
    req: &Value,
    lines: &mut tokio::io::Lines<BufReader<tokio::net::unix::OwnedReadHalf>>,
    w: &mut tokio::net::unix::OwnedWriteHalf,
) -> Result<Value> {
    if shared.library.lock().await.is_some() {
        bail!("this keeper is already paired with a library");
    }
    let ticket: PairTicket = req["ticket"]
        .as_str()
        .ok_or_else(|| anyhow!("missing `ticket`"))?
        .parse()
        .map_err(|e| anyhow!("{e}"))?;
    let name = req["name"]
        .as_str()
        .unwrap_or(&shared.opts.name)
        .to_string();

    let (code_tx, mut code_rx) = mpsc::unbounded_channel::<(PairingCode, oneshot::Sender<bool>)>();
    let decide: shoebox_node::DecideFn = Arc::new(move |code| {
        let (tx, rx) = oneshot::channel();
        let _ = code_tx.send((code, tx));
        Box::pin(async move { rx.await.unwrap_or(false) })
    });
    let node = shared.node.clone();
    let join = tokio::spawn(async move { node.pair_join(&ticket, &name, decide).await });
    tokio::pin!(join);
    let grant = loop {
        tokio::select! {
            Some((code, answer)) = code_rx.recv() => {
                send(w, json!({"type":"code","sas":code.sas,"peer_name":code.peer_name,
                    "peer_role":format!("{:?}", code.peer_role).to_lowercase()})).await?;
                let confirm = lines.next_line().await?.unwrap_or_default();
                let ok = serde_json::from_str::<Value>(&confirm)
                    .ok()
                    .and_then(|v| v["confirm"].as_bool())
                    .unwrap_or(false);
                let _ = answer.send(ok);
            }
            result = &mut join => break result??,
        }
    };
    let library_ticket = LibraryTicket {
        cap: grant.cap.clone(),
        peers: grant.peers.clone(),
    };
    shared.attach(library_ticket.clone()).await?;
    shared.save(&library_ticket)?;
    Ok(json!({
        "library": hex::encode(grant.cap.library.0),
        "role": format!("{:?}", grant.role).to_lowercase(),
    }))
}

/// How long a pairing ticket from `add_device` works.
const ADD_DEVICE_TTL: std::time::Duration = std::time::Duration::from_secs(300);

/// Offer to add one new device: reply with a ticket to paste into the other device's app, then
/// relay the six-digit code for the person to compare, and report how it went. The keeper must be
/// an admin (the same rule as in the app).
async fn add_device(
    shared: &Arc<Shared>,
    req: &Value,
    lines: &mut tokio::io::Lines<BufReader<tokio::net::unix::OwnedReadHalf>>,
    w: &mut tokio::net::unix::OwnedWriteHalf,
) -> Result<Value> {
    use shoebox_core::model::Role;
    let ln = library(shared).await?;
    let role = match req["role"]
        .as_str()
        .unwrap_or("companion")
        .to_ascii_lowercase()
        .as_str()
    {
        "keeper" => Role::Keeper,
        "companion" | "phone" => Role::Companion,
        "viewer" => Role::Viewer,
        other => bail!("a role is keeper, companion or viewer, not `{other}`"),
    };
    let (code_tx, mut code_rx) = mpsc::unbounded_channel::<(PairingCode, oneshot::Sender<bool>)>();
    let decide: shoebox_node::DecideFn = Arc::new(move |code| {
        let (tx, rx) = oneshot::channel();
        let _ = code_tx.send((code, tx));
        Box::pin(async move { rx.await.unwrap_or(false) })
    });
    // Subscribe first: the outcome event must not be missed.
    let mut events = shared.node.events();
    let ticket = ln.start_pairing(role, ADD_DEVICE_TTL, decide).await.map_err(|e| {
        if format!("{e:#}").contains("admin") {
            anyhow!("only an admin device can add devices; make this keeper an admin first: shoebox-keeper devices make-admin <this keeper>")
        } else {
            e
        }
    })?;
    send(
        w,
        json!({"type":"ticket","ticket":ticket.to_string(),
               "role":format!("{role:?}").to_lowercase(),
               "expires_in_secs":ADD_DEVICE_TTL.as_secs()}),
    )
    .await?;
    let deadline = tokio::time::sleep(ADD_DEVICE_TTL + std::time::Duration::from_secs(60));
    tokio::pin!(deadline);
    let mut peer_name = String::new();
    loop {
        tokio::select! {
            Some((code, answer)) = code_rx.recv() => {
                peer_name = code.peer_name.clone();
                send(w, json!({"type":"code","sas":code.sas,"peer_name":code.peer_name,
                    "peer_role":format!("{:?}", code.peer_role).to_lowercase()})).await?;
                let confirm = lines.next_line().await?.unwrap_or_default();
                let ok = serde_json::from_str::<Value>(&confirm)
                    .ok()
                    .and_then(|v| v["confirm"].as_bool())
                    .unwrap_or(false);
                let _ = answer.send(ok);
            }
            ev = events.recv() => match ev {
                Ok(shoebox_node::NodeEvent::PairingDone { peer }) => {
                    return Ok(json!({"device": hex::encode(peer.0), "name": peer_name}));
                }
                Ok(shoebox_node::NodeEvent::PairingFailed { reason }) => bail!("{reason}"),
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => bail!("the keeper is shutting down"),
            },
            _ = &mut deadline => bail!("nobody used the ticket in time"),
        }
    }
}

/// A client for the control socket (used by the CLI subcommands and by tests).
pub struct Client {
    lines: tokio::io::Lines<BufReader<tokio::net::unix::OwnedReadHalf>>,
    w: tokio::net::unix::OwnedWriteHalf,
}

impl Client {
    pub async fn connect(path: &Path) -> Result<Client> {
        let stream = UnixStream::connect(path).await.with_context(|| {
            format!(
                "no keeper is listening on {} (is it running?)",
                path.display()
            )
        })?;
        let (r, w) = stream.into_split();
        Ok(Client {
            lines: BufReader::new(r).lines(),
            w,
        })
    }

    async fn read_reply(&mut self) -> Result<Value> {
        let line = self
            .lines
            .next_line()
            .await?
            .ok_or_else(|| anyhow!("keeper closed the connection"))?;
        Ok(serde_json::from_str(&line)?)
    }

    /// Send a request and return its `data`, or the daemon's error message as an `Err`.
    pub async fn call_raw(&mut self, request: Value) -> Result<Value> {
        send(&mut self.w, request).await?;
        let reply = self.read_reply().await?;
        match reply["type"].as_str() {
            Some("ok") => Ok(reply["data"].clone()),
            Some("error") => bail!("{}", reply["message"].as_str().unwrap_or("error")),
            _ => bail!("unexpected reply: {reply}"),
        }
    }

    pub async fn status(&mut self) -> Result<Value> {
        self.call_raw(json!({"cmd":"status"})).await
    }

    /// Offer to add one device as `role`. `show` gets the ticket to paste into the other device;
    /// `confirm` sees the six-digit code and decides. Returns the new device (`device`, `name`).
    pub async fn add_device(
        &mut self,
        role: &str,
        mut show: impl FnMut(&str, u64),
        mut confirm: impl FnMut(&PairingCode) -> bool,
    ) -> Result<Value> {
        send(&mut self.w, json!({"cmd":"add_device","role":role})).await?;
        loop {
            let reply = self.read_reply().await?;
            match reply["type"].as_str() {
                Some("ticket") => show(
                    reply["ticket"].as_str().unwrap_or_default(),
                    reply["expires_in_secs"].as_u64().unwrap_or(0),
                ),
                Some("code") => {
                    let code = PairingCode {
                        sas: reply["sas"].as_str().unwrap_or_default().to_string(),
                        peer_name: reply["peer_name"].as_str().unwrap_or_default().to_string(),
                        peer_role: shoebox_core::model::Role::Keeper,
                    };
                    let ok = confirm(&code);
                    send(&mut self.w, json!({"confirm": ok})).await?;
                }
                Some("ok") => return Ok(reply["data"].clone()),
                Some("error") => bail!("{}", reply["message"].as_str().unwrap_or("error")),
                _ => bail!("unexpected reply: {reply}"),
            }
        }
    }

    /// Pair this keeper into a library. `confirm` sees the six-digit code and decides.
    pub async fn pair(
        &mut self,
        ticket: &str,
        name: &str,
        mut confirm: impl FnMut(&PairingCode) -> bool,
    ) -> Result<Value> {
        send(
            &mut self.w,
            json!({"cmd":"pair","ticket":ticket,"name":name}),
        )
        .await?;
        loop {
            let reply = self.read_reply().await?;
            match reply["type"].as_str() {
                Some("code") => {
                    let code = PairingCode {
                        sas: reply["sas"].as_str().unwrap_or_default().to_string(),
                        peer_name: reply["peer_name"].as_str().unwrap_or_default().to_string(),
                        peer_role: shoebox_core::model::Role::Keeper,
                    };
                    let ok = confirm(&code);
                    send(&mut self.w, json!({"confirm": ok})).await?;
                }
                Some("ok") => return Ok(reply["data"].clone()),
                Some("error") => bail!("{}", reply["message"].as_str().unwrap_or("error")),
                _ => bail!("unexpected reply: {reply}"),
            }
        }
    }
}
