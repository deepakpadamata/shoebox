//! `shoebox-dev`: a small CLI to demo and debug the M1/M2 stack before any UI exists.
//!
//! One data directory = one device. `init` creates a library, `join` joins someone else's, and
//! `run` keeps the device online (serving blobs and metadata, fetching what its role wants). The
//! other commands open the directory briefly, so they cannot run *while* `run` is using the same
//! directory; use a second directory (a second "device") to play both sides on one machine.

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use shoebox_core::clock::SystemClock;
use shoebox_core::identity::KeyPair;
use shoebox_core::ids::EndpointId;
use shoebox_core::model::{BlobKind, PhotoRecord, Role};
use shoebox_core::store::{LibraryStore, Page, PeerAddr, StoreBackend, TimeRange};
use shoebox_node::{
    DecideFn, FetcherConfig, ImportMode, ImportOptions, LibraryNode, LibraryTicket,
    MaintenanceConfig, Node, NodeConfig, NodeEvent, PairTicket, PathKind, PresenceConfig,
    RelaySetting,
};
use shoebox_store::{HybridBackend, Library};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

#[derive(Parser)]
#[command(
    name = "shoebox-dev",
    version,
    about = "Developer CLI for the Shoebox sync core"
)]
struct Cli {
    /// Data directory of this device (one directory = one device).
    #[arg(long, global = true, env = "SHOEBOX_DEV_DIR")]
    dir: Option<PathBuf>,
    /// Print machine-readable JSON instead of text.
    #[arg(long, global = true)]
    json: bool,
    /// No relay and no mDNS: direct connections to known addresses only (tests, offline demos).
    #[arg(long, global = true)]
    local_only: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Create a new library on this device.
    Init {
        #[arg(long, default_value = "dev-device")]
        name: String,
        #[arg(long, default_value = "keeper")]
        role: String,
    },
    /// Join another device's library with its ticket, and sync once.
    Join {
        ticket: String,
        #[arg(long, default_value = "dev-device")]
        name: String,
        #[arg(long, default_value = "keeper")]
        role: String,
    },
    /// Stay online: serve this library, sync with peers and fetch what this device's role wants.
    Run,
    /// Print a ticket other devices can join with (only useful while `run` is up).
    Ticket,
    /// Import files and folders.
    Import {
        paths: Vec<PathBuf>,
        /// Reference the files in place instead of copying them into the blob store.
        #[arg(long)]
        reference: bool,
    },
    /// List the library, newest first.
    Ls,
    /// Sync once with every known peer.
    Sync,
    /// Show known peers and whether they can be reached right now.
    Peers,
    /// Which devices hold a photo, and how many copies exist.
    Holdings { id: String },
    /// Fetch a blob of a photo from the best device that has it.
    Fetch {
        id: String,
        #[arg(long, default_value = "original")]
        kind: String,
    },
    /// Summary of this device and library.
    Status,
}

/// What a device remembers between invocations.
#[derive(Serialize, Deserialize)]
struct DevState {
    name: String,
    role: Role,
    /// The ticket that created or joined the library (its capability is what matters).
    ticket: String,
    peers: Vec<PeerAddr>,
    /// Dev tool only: hex seed of the library root key, present on the device that ran `init`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    root_seed: Option<String>,
}

struct Session {
    ln: LibraryNode<Library>,
    state: DevState,
    dir: PathBuf,
}

fn state_path(dir: &Path) -> PathBuf {
    dir.join("dev.json")
}

fn parse_role(s: &str) -> Result<Role> {
    match s.to_ascii_lowercase().as_str() {
        "keeper" => Ok(Role::Keeper),
        "companion" | "phone" => Ok(Role::Companion),
        "viewer" => Ok(Role::Viewer),
        other => bail!("unknown role `{other}` (use keeper, companion or viewer)"),
    }
}

fn parse_kind(s: &str) -> Result<BlobKind> {
    match s.to_ascii_lowercase().as_str() {
        "original" => Ok(BlobKind::Original),
        "preview" => Ok(BlobKind::Preview),
        "thumb" | "thumbnail" => Ok(BlobKind::Thumb),
        other => bail!("unknown kind `{other}` (use original, preview or thumb)"),
    }
}

fn node_config(cli: &Cli, dir: &Path, name: &str, role: Role) -> NodeConfig {
    let mut cfg = NodeConfig::local_only(dir.join("node"), name);
    cfg.role = role;
    if !cli.local_only {
        cfg.relay = RelaySetting::Default;
        cfg.mdns = true;
    }
    cfg
}

fn data_dir(cli: &Cli) -> Result<PathBuf> {
    match &cli.dir {
        Some(d) => Ok(d.clone()),
        None => {
            let home = std::env::var_os("HOME").ok_or_else(|| anyhow!("no HOME; pass --dir"))?;
            Ok(PathBuf::from(home).join(".shoebox-dev"))
        }
    }
}

fn short(id: &EndpointId) -> String {
    hex::encode(&id.0[..4])
}

/// Open an existing device (after `init` or `join`).
async fn open(cli: &Cli) -> Result<Session> {
    let dir = data_dir(cli)?;
    let text = std::fs::read_to_string(state_path(&dir)).map_err(|_| {
        anyhow!(
            "no library in {}: run `shoebox-dev init` or `join <ticket>` first",
            dir.display()
        )
    })?;
    let state: DevState = serde_json::from_str(&text).context("reading dev.json")?;
    let ticket: LibraryTicket = state
        .ticket
        .parse()
        .map_err(|e| anyhow!("corrupt ticket in dev.json: {e}"))?;
    let node = Node::open(node_config(cli, &dir, &state.name, state.role)).await?;
    let backend = HybridBackend::for_node(node.clone(), SystemClock);
    let library = backend.open_library(&ticket.cap).await?;
    let ln = LibraryNode::new(node, Arc::new(library), SystemClock).with_capability(ticket.cap);
    for peer in &state.peers {
        ln.add_peer(peer.clone())?;
    }
    Ok(Session { ln, state, dir })
}

impl Session {
    async fn close(self) -> Result<()> {
        self.ln.node().shutdown().await
    }

    /// The photo whose id starts with `prefix` (must be unambiguous).
    async fn find(&self, prefix: &str) -> Result<PhotoRecord> {
        let prefix = prefix.to_ascii_lowercase();
        if prefix.len() < 4 || !prefix.chars().all(|c| c.is_ascii_hexdigit()) {
            bail!("`{prefix}` is not a photo id prefix (give at least 4 hex digits)");
        }
        let mut all = self
            .ln
            .store()
            .photos_by_time(TimeRange::ALL, Page::all())
            .await?;
        all.extend(self.ln.store().deleted_photos().await?);
        let mut hits: Vec<PhotoRecord> = all
            .into_iter()
            .filter(|p| p.id.to_string().starts_with(&prefix))
            .collect();
        match hits.len() {
            0 => bail!("no photo with an id starting with `{prefix}`"),
            1 => Ok(hits.remove(0)),
            n => bail!("`{prefix}` matches {n} photos; give more digits"),
        }
    }

    /// Sync with every known peer, ignoring failures (they are reported).
    async fn sync_all(&self) -> Vec<(EndpointId, Result<()>)> {
        let mut out = vec![];
        for id in self.ln.peers() {
            out.push((id, self.ln.sync_now(id).await));
        }
        out
    }
}

fn iso(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}",
        rem / 3600,
        rem % 3600 / 60
    )
}

fn print(cli: &Cli, value: Value, text: impl FnOnce() -> String) {
    if cli.json {
        println!("{value}");
    } else {
        println!("{}", text());
    }
}

async fn local(ln: &LibraryNode<Library>, rec: &PhotoRecord) -> Result<Value> {
    let blobs = ln.node().blobs();
    Ok(json!({
        "thumb": blobs.has(&rec.thumb.hash).await?,
        "preview": blobs.has(&rec.preview.hash).await?,
        "original": blobs.has(&rec.original.hash).await?,
    }))
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match &cli.cmd {
        Cmd::Init { name, role } => cmd_init(&cli, name, role).await,
        Cmd::Join { ticket, name, role } => cmd_join(&cli, ticket, name, role).await,
        Cmd::Run => cmd_run(&cli).await,
        Cmd::Ticket => {
            let s = open(&cli).await?;
            let ticket = s.ln.ticket()?.to_string();
            print(&cli, json!({ "ticket": ticket }), || ticket.clone());
            s.close().await
        }
        Cmd::Import { paths, reference } => cmd_import(&cli, paths, *reference).await,
        Cmd::Ls => cmd_ls(&cli).await,
        Cmd::Sync => cmd_sync(&cli).await,
        Cmd::Peers => cmd_peers(&cli).await,
        Cmd::Holdings { id } => cmd_holdings(&cli, id).await,
        Cmd::Fetch { id, kind } => cmd_fetch(&cli, id, kind).await,
        Cmd::Status => cmd_status(&cli).await,
    }
}

async fn cmd_init(cli: &Cli, name: &str, role: &str) -> Result<()> {
    let dir = data_dir(cli)?;
    if state_path(&dir).exists() {
        bail!(
            "{} already holds a library (use another --dir, or delete it)",
            dir.display()
        );
    }
    let role = parse_role(role)?;
    std::fs::create_dir_all(&dir)?;
    let node = Node::open(node_config(cli, &dir, name, role)).await?;
    let device = EndpointId(*node.id().as_bytes());
    let backend = HybridBackend::for_node(node.clone(), SystemClock);
    // A real root key (M2-01). The dev tool keeps its seed in dev.json for convenience; real
    // devices never store it (it lives only in the recovery kit, M2-02).
    let root = KeyPair::generate();
    let (library, cap) = backend.create_library(&root.root_public()).await?;
    let ln =
        LibraryNode::new(node.clone(), Arc::new(library), SystemClock).with_capability(cap.clone());
    // This device is the library's first admin and first member; the kit is the only way back.
    let kit = ln.create_identity(&root, role, name).await?;
    let kit_path = dir.join("recovery-kit.txt");
    std::fs::write(&kit_path, kit.to_text())?;
    let ticket = ln.ticket()?.to_string();
    let state = DevState {
        name: name.into(),
        role,
        ticket: ticket.clone(),
        peers: vec![],
        root_seed: Some(hex::encode(root.seed())),
    };
    std::fs::write(state_path(&dir), serde_json::to_string_pretty(&state)?)?;
    node.shutdown().await?;
    print(
        cli,
        json!({ "device": hex::encode(device.0), "ticket": ticket }),
        || {
            format!(
                "created a library on {} ({})\nrun `shoebox-dev run` to share it",
                name,
                short(&device)
            )
        },
    );
    Ok(())
}

async fn cmd_join(cli: &Cli, ticket_text: &str, name: &str, role: &str) -> Result<()> {
    let dir = data_dir(cli)?;
    if state_path(&dir).exists() {
        bail!("{} already holds a library", dir.display());
    }
    let ticket: PairTicket = ticket_text.parse().map_err(|e| {
        anyhow!("bad ticket: {e} (use a `ticket:` line printed by `shoebox-dev run`)")
    })?;
    let role = parse_role(role)?;
    if ticket.role != role {
        bail!(
            "this ticket offers the {:?} role, not {role:?}; use the matching ticket line",
            ticket.role
        );
    }
    std::fs::create_dir_all(&dir)?;
    let node = Node::open(node_config(cli, &dir, name, role)).await?;
    // Pairing: the dev tool accepts the code automatically (both sides print it).
    let decide: DecideFn = Arc::new(|code| {
        Box::pin(async move {
            eprintln!(
                "pairing code {} (accepted automatically by the dev tool)",
                code.sas
            );
            true
        })
    });
    let grant = match node.pair_join(&ticket, name, decide).await {
        Ok(g) => g,
        Err(e) => {
            node.shutdown().await?;
            let _ = std::fs::remove_dir_all(&dir);
            bail!("pairing failed: {e:#} (is `shoebox-dev run` up on the other side?)");
        }
    };
    let library_ticket = LibraryTicket {
        cap: grant.cap.clone(),
        peers: grant.peers.clone(),
    };
    let backend = HybridBackend::for_node(node.clone(), SystemClock);
    let library = backend.open_library(&grant.cap).await?;
    let ln = LibraryNode::new(node.clone(), Arc::new(library), SystemClock)
        .with_capability(grant.cap.clone());
    for peer in &grant.peers {
        ln.add_peer(peer.clone())?;
    }
    let mut synced = false;
    for peer in &grant.peers {
        match ln.sync_now(peer.id).await {
            Ok(()) => {
                synced = true;
                break;
            }
            Err(e) => eprintln!("could not sync with {}: {e:#}", short(&peer.id)),
        }
    }
    if !synced {
        node.shutdown().await?;
        let _ = std::fs::remove_dir_all(&dir);
        bail!("paired, but could not sync with the inviting device");
    }
    let photos = ln
        .store()
        .photos_by_time(TimeRange::ALL, Page::all())
        .await?
        .len();
    let state = DevState {
        name: name.into(),
        role,
        ticket: library_ticket.to_string(),
        peers: grant.peers.clone(),
        root_seed: None,
    };
    std::fs::write(state_path(&dir), serde_json::to_string_pretty(&state)?)?;
    node.shutdown().await?;
    print(cli, json!({ "photos": photos }), || {
        format!("joined; the library has {photos} photo(s)")
    });
    Ok(())
}

async fn cmd_run(cli: &Cli) -> Result<()> {
    let s = open(cli).await?;
    s.ln.start_fetcher(FetcherConfig::default());
    s.ln.start_maintenance(MaintenanceConfig::default());
    s.ln.start_presence(PresenceConfig::default()).await?;
    let me = EndpointId(*s.ln.node().id().as_bytes());
    // The lines the other side (and the tests) look for. Flushed so pipes see them immediately.
    println!("device: {}", hex::encode(me.0));
    println!("role: {:?}", s.state.role);
    use std::io::Write;
    // Admin devices offer one single-use pairing ticket per role and a fresh one after each use.
    for role in [Role::Keeper, Role::Viewer, Role::Companion] {
        match offer(&s.ln, role).await {
            Ok(ticket) => println!("{}", ticket_line(role, &ticket)),
            Err(e) => {
                println!("no pairing tickets: {e:#}");
                break;
            }
        }
    }
    std::io::stdout().flush().ok();

    let mut events = s.ln.node().events();
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            ev = events.recv() => {
                let Ok(ev) = ev else { continue };
                if let NodeEvent::LibraryRotated { cap } = &ev {
                    // The library moved to a new epoch: remember the new capability.
                    let ticket = LibraryTicket { cap: cap.clone(), peers: s.state.peers.clone() };
                    let mut state = DevState {
                        name: s.state.name.clone(),
                        role: s.state.role,
                        ticket: ticket.to_string(),
                        peers: s.state.peers.clone(),
                        root_seed: s.state.root_seed.clone(),
                    };
                    state.ticket = ticket.to_string();
                    let _ = std::fs::write(state_path(&s.dir), serde_json::to_string_pretty(&state).unwrap_or_default());
                }
                if let NodeEvent::PairingDone { .. } = ev {
                    // Every ticket is single use: hand out fresh ones.
                    for role in [Role::Keeper, Role::Viewer, Role::Companion] {
                        if let Ok(ticket) = offer(&s.ln, role).await {
                            println!("{}", ticket_line(role, &ticket));
                        }
                    }
                    std::io::stdout().flush().ok();
                }
                if let Some(line) = describe(&ev) {
                    println!("{line}");
                    std::io::stdout().flush().ok();
                }
            }
        }
    }
    println!("shutting down");
    s.close().await
}

fn ticket_line(role: Role, ticket: &PairTicket) -> String {
    match role {
        Role::Keeper => format!("ticket: {ticket}"),
        Role::Viewer => format!("ticket-viewer: {ticket}"),
        Role::Companion => format!("ticket-companion: {ticket}"),
    }
}

/// A one-day pairing ticket for `role`; the dev tool confirms the code automatically.
async fn offer(ln: &LibraryNode<Library>, role: Role) -> Result<PairTicket> {
    let decide: DecideFn = Arc::new(|code| {
        Box::pin(async move {
            println!(
                "pairing code {} for {} (accepted automatically)",
                code.sas, code.peer_name
            );
            true
        })
    });
    ln.start_pairing(role, Duration::from_secs(24 * 3600), decide)
        .await
}

fn describe(ev: &NodeEvent) -> Option<String> {
    Some(match ev {
        NodeEvent::PeerConnected { id } => format!("peer connected: {}", id.fmt_short()),
        NodeEvent::PeerDisconnected { id } => format!("peer disconnected: {}", id.fmt_short()),
        NodeEvent::SyncProgress { peer, phase } => format!("sync {}: {phase:?}", peer.fmt_short()),
        NodeEvent::BlobFetched { hash, from } => format!(
            "fetched {} from {}",
            &hash.to_string()[..8],
            from.fmt_short()
        ),
        NodeEvent::PairingDone { peer } => format!("paired a device (via {})", short(peer)),
        NodeEvent::PresenceChanged { device, online } => {
            format!(
                "device {} is {}",
                short(device),
                if *online { "online" } else { "offline" }
            )
        }
        NodeEvent::ImportProgress {
            done,
            total,
            imported,
            duplicates,
            ..
        } => {
            format!("import {done}/{total} ({imported} new, {duplicates} duplicates)")
        }
        _ => return None,
    })
}

async fn cmd_import(cli: &Cli, paths: &[PathBuf], reference: bool) -> Result<()> {
    if paths.is_empty() {
        bail!("give at least one file or folder");
    }
    let s = open(cli).await?;
    let opts = ImportOptions {
        mode: if reference {
            ImportMode::Reference
        } else {
            ImportMode::Copy
        },
        ..Default::default()
    };
    let report = s.ln.import_paths(paths, opts).await?;
    let value = json!({
        "imported": report.imported.len(),
        "duplicates": report.duplicates.len(),
        "skipped": report.skipped.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
        "failed": report.failed.iter().map(|(p, e)| json!({"path": p.display().to_string(), "error": e})).collect::<Vec<_>>(),
    });
    print(cli, value, || {
        let mut text = format!(
            "imported {}, {} already in the library, {} skipped, {} failed",
            report.imported.len(),
            report.duplicates.len(),
            report.skipped.len(),
            report.failed.len()
        );
        for (p, e) in &report.failed {
            text.push_str(&format!("\n  failed: {}: {e}", p.display()));
        }
        text
    });
    s.close().await
}

async fn cmd_ls(cli: &Cli) -> Result<()> {
    let s = open(cli).await?;
    let photos =
        s.ln.store()
            .photos_by_time(TimeRange::ALL, Page::all())
            .await?;
    let mut rows = vec![];
    for rec in &photos {
        rows.push(json!({
            "id": rec.id.to_string(),
            "taken_at": rec.taken_at.0 / 1000,
            "mime": rec.original.mime,
            "size": rec.original.size,
            "camera": rec.exif.camera_model,
            "copies": s.ln.copy_count(&rec.id).await?,
            "local": local(&s.ln, rec).await?,
        }));
    }
    let value = Value::Array(rows.clone());
    print(cli, value, || {
        if rows.is_empty() {
            return "(empty library)".into();
        }
        let mut text =
            String::from("ID        TAKEN             SIZE      COPIES  LOCAL(t/p/o)  TYPE");
        for r in &rows {
            let l = &r["local"];
            let flag = |k: &str| if l[k] == true { 'y' } else { '-' };
            text.push_str(&format!(
                "\n{}  {}  {:>8}  {:>6}  {}/{}/{}         {}",
                &r["id"].as_str().unwrap()[..8],
                iso(r["taken_at"].as_i64().unwrap()),
                r["size"].as_u64().unwrap(),
                r["copies"].as_u64().unwrap(),
                flag("thumb"),
                flag("preview"),
                flag("original"),
                r["mime"].as_str().unwrap()
            ));
        }
        text
    });
    s.close().await
}

async fn cmd_sync(cli: &Cli) -> Result<()> {
    let s = open(cli).await?;
    let results = s.sync_all().await;
    let rows: Vec<Value> = results
        .iter()
        .map(|(id, r)| json!({"peer": hex::encode(id.0), "ok": r.is_ok(), "error": r.as_ref().err().map(|e| format!("{e:#}"))}))
        .collect();
    let any_failed = results.iter().any(|(_, r)| r.is_err());
    print(cli, Value::Array(rows), || {
        if results.is_empty() {
            return "no peers known (join a library first)".into();
        }
        results
            .iter()
            .map(|(id, r)| match r {
                Ok(()) => format!("{}: synced", short(id)),
                Err(e) => format!("{}: FAILED ({e:#})", short(id)),
            })
            .collect::<Vec<_>>()
            .join("\n")
    });
    s.close().await?;
    if any_failed {
        std::process::exit(1);
    }
    Ok(())
}

async fn cmd_peers(cli: &Cli) -> Result<()> {
    let s = open(cli).await?;
    let mut rows = vec![];
    for id in s.ln.peers() {
        let iroh_id = shoebox_node::to_iroh_id(&id).map_err(|e| anyhow!("{e}"))?;
        let ping = tokio::time::timeout(Duration::from_secs(5), s.ln.node().ping(iroh_id)).await;
        let rtt = match ping {
            Ok(Ok(rtt)) => Some(rtt),
            _ => None,
        };
        let path = match s.ln.node().peer_info(iroh_id).await {
            Some(info) if rtt.is_some() => match info.path {
                PathKind::Lan => "lan".to_string(),
                PathKind::Direct => "direct".to_string(),
                PathKind::Relay { url } => format!("relay {url}"),
                PathKind::Unknown => "unknown".to_string(),
            },
            _ => "none".to_string(),
        };
        rows.push(json!({
            "id": hex::encode(id.0),
            "reachable": rtt.is_some(),
            "latency_ms": rtt.map(|r| r.as_secs_f64() * 1000.0),
            "path": path,
        }));
    }
    let value = Value::Array(rows.clone());
    print(cli, value, || {
        if rows.is_empty() {
            return "no peers known".into();
        }
        rows.iter()
            .map(|r| {
                format!(
                    "{}  {}  {}",
                    &r["id"].as_str().unwrap()[..8],
                    if r["reachable"] == true {
                        "reachable"
                    } else {
                        "UNREACHABLE"
                    },
                    match r["latency_ms"].as_f64() {
                        Some(ms) => format!("{ms:.1} ms via {}", r["path"].as_str().unwrap()),
                        None => String::new(),
                    }
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    });
    s.close().await
}

async fn cmd_holdings(cli: &Cli, prefix: &str) -> Result<()> {
    let s = open(cli).await?;
    let rec = s.find(prefix).await?;
    let holders = s.ln.holders(&rec.original.hash).await?;
    let copies = s.ln.copy_count(&rec.id).await?;
    let target = s.ln.redundancy_target().await?;
    let rows: Vec<Value> = holders
        .iter()
        .map(|h| json!({"device": hex::encode(h.device.0), "kind": format!("{:?}", h.kind).to_lowercase(), "since_ms": h.since.physical_ms}))
        .collect();
    let value =
        json!({"id": rec.id.to_string(), "copies": copies, "target": target, "holders": rows});
    print(cli, value, || {
        let mut text = format!(
            "{}: {copies} cop{} (target {target})",
            &rec.id.to_string()[..8],
            if copies == 1 { "y" } else { "ies" }
        );
        for h in &holders {
            text.push_str(&format!("\n  {}  {:?}", short(&h.device), h.kind));
        }
        text
    });
    s.close().await
}

async fn cmd_fetch(cli: &Cli, prefix: &str, kind: &str) -> Result<()> {
    let kind_enum = parse_kind(kind)?;
    let s = open(cli).await?;
    let rec = s.find(prefix).await?;
    // Learn who holds what before asking.
    let _ = s.sync_all().await;
    let source = s.ln.fetch_now(&rec.id, kind_enum).await?;
    // One-shot command: tell the peers about the copy we just made before we exit, or they
    // would never learn it (nothing keeps this process running to sync later).
    let _ = s.sync_all().await;
    let value = json!({"id": rec.id.to_string(), "kind": kind.to_ascii_lowercase(), "from": hex::encode(source.device.0)});
    print(cli, value, || {
        format!(
            "fetched the {kind} of {} from {}",
            &rec.id.to_string()[..8],
            short(&source.device)
        )
    });
    let _ = (&s.dir, &s.state);
    s.close().await
}

async fn cmd_status(cli: &Cli) -> Result<()> {
    let s = open(cli).await?;
    let live =
        s.ln.store()
            .photos_by_time(TimeRange::ALL, Page::all())
            .await?
            .len();
    let deleted = s.ln.recently_deleted().await?.len();
    let at_risk = s.ln.at_risk(Page::all()).await?.len();
    let me = EndpointId(*s.ln.node().id().as_bytes());
    let value = json!({
        "device": hex::encode(me.0),
        "name": s.state.name,
        "role": format!("{:?}", s.state.role),
        "photos": live,
        "deleted": deleted,
        "at_risk": at_risk,
        "redundancy_target": s.ln.redundancy_target().await?,
        "peers": s.ln.peers().len(),
    });
    print(cli, value.clone(), || {
        format!(
            "{} ({}) {:?}: {live} photo(s), {deleted} deleted, {at_risk} below the redundancy target of {}, {} peer(s)",
            s.state.name,
            short(&me),
            s.state.role,
            value["redundancy_target"],
            s.ln.peers().len()
        )
    });
    s.close().await
}
