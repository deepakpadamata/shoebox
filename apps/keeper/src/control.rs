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
use shoebox_node::{LibraryTicket, PairTicket, PairingCode};
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

async fn dispatch(shared: &Arc<Shared>, cmd: &str, req: &Value) -> Result<Value> {
    match cmd {
        "status" => status::report(shared).await,
        "devices" => status::devices(shared).await,
        "revoke" => {
            let id = req["device"]
                .as_str()
                .ok_or_else(|| anyhow!("missing `device`"))?;
            let bytes: [u8; 32] = hex::decode(id.trim())
                .ok()
                .and_then(|b| b.try_into().ok())
                .ok_or_else(|| anyhow!("`device` must be a 64-digit hex device id"))?;
            let guard = shared.library.lock().await;
            let ln = guard
                .as_ref()
                .ok_or_else(|| anyhow!("not paired with a library yet"))?;
            ln.revoke_device(&EndpointId(bytes)).await?;
            Ok(json!({}))
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
