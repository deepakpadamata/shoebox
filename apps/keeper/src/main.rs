use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use keeper::control::Client;
use keeper::service::{self, ServiceSpec};
use keeper::{default_storage, render, Daemon, KeeperOptions};
use serde_json::{json, Value};
use shoebox_node::RelaySetting;
use std::io::Write;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "shoebox-keeper",
    version,
    about = "The always-on Shoebox backstop device"
)]
struct Cli {
    /// Where the keeper keeps its data (default: $XDG_DATA_HOME/shoebox).
    #[arg(long, global = true, env = "SHOEBOX_STORAGE")]
    storage: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the daemon in the foreground.
    Run {
        #[arg(long, default_value = "keeper")]
        name: String,
        /// `default`, `off`, or the URL of your own relay.
        #[arg(long, default_value = "default")]
        relay: String,
        /// Do not announce/discover devices on the local network.
        #[arg(long)]
        no_mdns: bool,
    },
    /// Pair this keeper with a library using a ticket from “Add a device”.
    Pair {
        ticket: String,
        #[arg(long, default_value = "keeper")]
        name: String,
        /// Skip the interactive code check (not recommended).
        #[arg(long)]
        yes: bool,
    },
    /// Show what the keeper is doing.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// List the library's devices, or revoke one.
    Devices {
        #[command(subcommand)]
        action: Option<DevicesCmd>,
        #[arg(long)]
        json: bool,
    },
    /// Shared albums (available after shared albums land).
    Album {
        #[command(subcommand)]
        action: AlbumCmd,
    },
    /// Install or remove the system service.
    Service {
        #[command(subcommand)]
        action: ServiceCmd,
    },
    /// Stop the running daemon.
    Stop,
}

#[derive(Subcommand)]
enum DevicesCmd {
    /// Revoke a device (this keeper must be an admin).
    Revoke { id: String },
}

#[derive(Subcommand)]
enum AlbumCmd {
    List,
    Keep { id: String },
    Drop { id: String },
}

#[derive(Subcommand)]
enum ServiceCmd {
    Install {
        /// Install system-wide (needs root) instead of for the invoking user.
        #[arg(long)]
        system: bool,
        /// With --system: the dedicated user to run as.
        #[arg(long)]
        user: Option<String>,
        /// Print the unit instead of installing it.
        #[arg(long)]
        print: bool,
    },
    Uninstall {
        #[arg(long)]
        system: bool,
    },
}

fn relay(s: &str) -> Result<RelaySetting> {
    Ok(match s {
        "default" => RelaySetting::Default,
        "off" => RelaySetting::Disabled,
        url if url.starts_with("http") => RelaySetting::Custom(url.into()),
        other => bail!("--relay must be `default`, `off` or a URL, not `{other}`"),
    })
}

async fn client(storage: &std::path::Path) -> Result<Client> {
    Client::connect(&storage.join("control.sock")).await
}

fn print(json_out: bool, data: &Value, text: impl FnOnce(&Value) -> String) {
    if json_out {
        println!("{}", serde_json::to_string_pretty(data).expect("json"));
    } else {
        print!("{}", text(data));
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let storage = cli.storage.clone().unwrap_or_else(default_storage);
    match cli.cmd {
        Cmd::Run {
            name,
            relay: r,
            no_mdns,
        } => {
            tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| "info".into()),
                )
                .with_ansi(false)
                .init();
            let mut opts = KeeperOptions::new(&storage, name);
            opts.relay = relay(&r)?;
            opts.mdns = !no_mdns;
            let daemon = Daemon::start(opts).await?;
            tokio::select! {
                _ = tokio::signal::ctrl_c() => tracing::info!("interrupted"),
                _ = wait_sigterm() => tracing::info!("terminated"),
                _ = daemon.wait_for_shutdown() => tracing::info!("stop requested"),
            }
            daemon.shutdown().await?;
        }
        Cmd::Pair { ticket, name, yes } => {
            let mut c = client(&storage).await?;
            println!("Contacting the device that made this ticket…");
            let done = c
                .pair(&ticket, &name, |code| {
                    println!(
                        "\nCheck that the other device shows this code:\n\n    {}\n",
                        code.sas
                    );
                    if yes {
                        return true;
                    }
                    print!("Does it match? [y/N] ");
                    let _ = std::io::stdout().flush();
                    let mut line = String::new();
                    let _ = std::io::stdin().read_line(&mut line);
                    matches!(line.trim().to_lowercase().as_str(), "y" | "yes")
                })
                .await?;
            println!(
                "Paired. Joined library {}.",
                done["library"]
                    .as_str()
                    .unwrap_or("?")
                    .get(..8)
                    .unwrap_or("?")
            );
        }
        Cmd::Status { json } => {
            let data = client(&storage).await?.status().await?;
            print(json, &data, render::status);
        }
        Cmd::Devices { action: None, json } => {
            let data = client(&storage)
                .await?
                .call_raw(json!({"cmd":"devices"}))
                .await?;
            print(json, &data, render::devices);
        }
        Cmd::Devices {
            action: Some(DevicesCmd::Revoke { id }),
            ..
        } => {
            client(&storage)
                .await?
                .call_raw(json!({"cmd":"revoke","device":id}))
                .await?;
            println!("Revoked {id}. It can no longer connect to this library.");
        }
        Cmd::Album { .. } => {
            client(&storage)
                .await?
                .call_raw(json!({"cmd":"album"}))
                .await?;
        }
        Cmd::Stop => {
            client(&storage)
                .await?
                .call_raw(json!({"cmd":"shutdown"}))
                .await?;
            println!("Stopping.");
        }
        Cmd::Service { action } => service_cmd(action, &storage)?,
    }
    Ok(())
}

#[cfg(unix)]
async fn wait_sigterm() {
    use tokio::signal::unix::{signal, SignalKind};
    match signal(SignalKind::terminate()) {
        Ok(mut s) => {
            s.recv().await;
        }
        Err(_) => std::future::pending().await,
    }
}

#[cfg(not(unix))]
async fn wait_sigterm() {
    std::future::pending::<()>().await
}

fn service_cmd(action: ServiceCmd, storage: &std::path::Path) -> Result<()> {
    let home = PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?);
    match action {
        ServiceCmd::Install {
            system,
            user,
            print,
        } => {
            let spec = ServiceSpec {
                exe: std::env::current_exe()?,
                storage: std::fs::canonicalize(storage).unwrap_or_else(|_| storage.to_path_buf()),
                user,
            };
            let (path, text) = if cfg!(target_os = "macos") {
                let logs = home.join("Library/Logs/shoebox");
                (
                    service::unit_path(&home, false),
                    service::launchd_plist(&spec, &logs),
                )
            } else {
                (
                    service::unit_path(&home, system),
                    service::systemd_unit(&spec, system),
                )
            };
            if print {
                println!("# {}\n{text}", path.display());
                return Ok(());
            }
            std::fs::create_dir_all(path.parent().expect("has parent"))?;
            if cfg!(target_os = "macos") {
                std::fs::create_dir_all(home.join("Library/Logs/shoebox"))?;
            }
            std::fs::create_dir_all(storage)?;
            std::fs::write(&path, text)?;
            println!("Wrote {}", path.display());
            if cfg!(target_os = "macos") {
                println!("Start it with: launchctl load -w {}", path.display());
            } else if system {
                println!("Start it with: sudo systemctl daemon-reload && sudo systemctl enable --now shoebox-keeper");
            } else {
                println!("Start it with: systemctl --user daemon-reload && systemctl --user enable --now shoebox-keeper\n(to start at boot without a login: sudo loginctl enable-linger $USER)");
            }
        }
        ServiceCmd::Uninstall { system } => {
            let path = service::unit_path(&home, system);
            if path.exists() {
                std::fs::remove_file(&path)?;
                println!(
                    "Removed {}. Your photos and keys were left in place.",
                    path.display()
                );
            } else {
                println!("No service file at {}", path.display());
            }
        }
    }
    Ok(())
}
