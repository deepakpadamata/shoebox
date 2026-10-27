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
        /// Do not look for newer releases (twice a day, on the Shoebox website).
        #[arg(long)]
        no_update_check: bool,
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
    /// Rejoin a library with its 24-word recovery phrase, through a surviving device.
    Recover {
        /// The surviving device's link (`shoebox-keeper address` on another keeper).
        #[arg(long)]
        from: String,
        /// The recovery phrase; read from the terminal when omitted (keeps it out of shell history).
        #[arg(long)]
        phrase: Option<String>,
        #[arg(long, default_value = "keeper")]
        name: String,
    },
    /// Print this keeper's link, which a recovering device can use to reach it.
    Address,
    /// Show what the keeper is doing.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// List the library's devices, or manage them: remove one, change a role, make an admin.
    Devices {
        #[command(subcommand)]
        action: Option<DevicesCmd>,
        #[arg(long)]
        json: bool,
    },
    /// Shared albums this keeper belongs to: list, keep every original, or join one.
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
    /// Security alerts (a new admin, a replaced recovery phrase): list them, or `ack` to clear.
    Alerts {
        #[command(subcommand)]
        action: Option<AlertsCmd>,
    },
    /// Print the licence of this keeper, where to get its source, and the third-party notices.
    Licences,
}

#[derive(Subcommand)]
enum AlertsCmd {
    /// You have seen them: clear the list.
    Ack,
}

#[derive(Subcommand)]
enum DevicesCmd {
    /// Remove a device from the library: it can no longer connect. Without --phrase this keeper
    /// must be an admin and the device must not be one; with the recovery phrase any device can
    /// be removed. The device is given by its id (or the start of it, as `devices` prints it) or its name.
    #[command(alias = "revoke")]
    Remove {
        device: String,
        /// Use the recovery phrase (asked for with the typing hidden).
        #[arg(long)]
        phrase: bool,
    },
    /// Add a new device: prints a pairing string to paste into "Add this device to an existing
    /// library" in the other device's app (it works once, for five minutes), then shows a
    /// six-digit code to compare. The keeper must be an admin.
    Add {
        /// The new device's role: keeper, companion or viewer.
        #[arg(long, default_value = "companion")]
        role: String,
        /// Skip the code comparison prompt (not recommended).
        #[arg(long)]
        yes: bool,
    },
    /// Give a device another role: keeper, companion or viewer. Without --phrase this keeper
    /// must be an admin.
    Role {
        device: String,
        role: String,
        #[arg(long)]
        phrase: bool,
    },
    /// Make a device an admin, so it can add devices, change roles and remove devices without the
    /// phrase. Always needs the recovery phrase (asked for with the typing hidden).
    MakeAdmin { device: String },
    /// Replace the recovery phrase (for example after it may have been seen). Prints the new 24
    /// words once; every device keeps working. Needs the current phrase.
    ReplacePhrase,
}

#[derive(Subcommand)]
enum AlbumCmd {
    /// List the albums with their size and whether originals are kept.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Keep every original of this album on the keeper (fetches them all).
    Keep { id: String },
    /// Stop keeping originals (they are freed on the next cleanup unless the library needs them).
    Drop { id: String },
    /// Join an album with the invitation text its owner shows.
    Join { invitation: String },
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

/// Ask for a recovery phrase on the terminal with the typing hidden (so it stays out of the
/// screen and the shell history). When input is piped it is read as a line.
fn ask_phrase(prompt: &str) -> Result<String> {
    use std::io::IsTerminal;
    eprint!("{prompt}");
    let _ = std::io::stderr().flush();
    let hide = std::io::stdin().is_terminal()
        && std::process::Command::new("stty")
            .arg("-echo")
            .status()
            .is_ok_and(|s| s.success());
    let mut line = String::new();
    let read = std::io::stdin().read_line(&mut line);
    if hide {
        let _ = std::process::Command::new("stty").arg("echo").status();
        eprintln!();
    }
    read?;
    let line = line.trim().to_string();
    if line.is_empty() {
        bail!("no recovery phrase given");
    }
    Ok(line)
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
        Cmd::Licences => {
            let text = format!(
                "shoebox-keeper {} is free software under the GNU AGPL-3.0-only (apps/LICENSE-AGPL).\n\
                 Its source code: {}\n\n{}",
                env!("CARGO_PKG_VERSION"),
                env!("CARGO_PKG_REPOSITORY"),
                include_str!("../THIRD-PARTY-LICENSES.txt")
            );
            // Piped into `less` or `head`, a closed pipe just ends the output.
            let _ = std::io::Write::write_all(&mut std::io::stdout(), text.as_bytes());
            return Ok(());
        }
        Cmd::Run {
            name,
            relay: r,
            no_mdns,
            no_update_check,
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
            if no_update_check {
                opts.update_feed = shoebox_node::update::PLACEHOLDER_FEED.to_string();
            }
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
        Cmd::Recover { from, phrase, name } => {
            let phrase = match phrase {
                Some(p) => p,
                None => ask_phrase("Recovery phrase (24 words): ")?,
            };
            let done = client(&storage)
                .await?
                .call_raw(json!({"cmd":"recover","from":from,"phrase":phrase,"name":name}))
                .await?;
            println!(
                "Recovered. Joined library {}.",
                done["library"]
                    .as_str()
                    .unwrap_or("?")
                    .get(..8)
                    .unwrap_or("?")
            );
        }
        Cmd::Address => {
            let data = client(&storage)
                .await?
                .call_raw(json!({"cmd":"address"}))
                .await?;
            println!("{}", data["link"].as_str().unwrap_or(""));
        }
        Cmd::Alerts { action } => {
            let mut c = client(&storage).await?;
            match action {
                Some(AlertsCmd::Ack) => {
                    c.call_raw(json!({"cmd":"alerts_ack"})).await?;
                    println!("Alerts cleared.");
                }
                None => {
                    let data = c.call_raw(json!({"cmd":"alerts"})).await?;
                    let list: Vec<keeper::alerts::Alert> =
                        serde_json::from_value(data["alerts"].clone()).unwrap_or_default();
                    if list.is_empty() {
                        println!("No security alerts.");
                    }
                    for a in &list {
                        println!("! {}", keeper::alerts::describe(a));
                    }
                }
            }
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
            action: Some(action),
            ..
        } => {
            let mut c = client(&storage).await?;
            match action {
                DevicesCmd::Remove { device, phrase } => {
                    let phrase = phrase
                        .then(|| ask_phrase("Recovery phrase (24 words): "))
                        .transpose()?;
                    c.call_raw(json!({"cmd":"revoke","device":device,"phrase":phrase}))
                        .await?;
                    println!("Removed {device}. It can no longer connect to this library.");
                }
                DevicesCmd::Add { role, yes } => {
                    let done = c
                        .add_device(
                            &role,
                            |ticket, secs| {
                                println!(
                                    "Paste this into the new device (\"Add this device to an existing library\"). It works once, for {} minutes:\n\n{ticket}\n\nWaiting for the new device…",
                                    secs / 60
                                );
                            },
                            |code| {
                                println!(
                                    "\n{} is asking to join. Check that it shows this code:\n\n    {}\n",
                                    if code.peer_name.is_empty() { "A device" } else { &code.peer_name },
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
                            },
                        )
                        .await?;
                    println!(
                        "Added {}.",
                        done["name"]
                            .as_str()
                            .filter(|n| !n.is_empty())
                            .unwrap_or("the new device")
                    );
                }
                DevicesCmd::Role {
                    device,
                    role,
                    phrase,
                } => {
                    let phrase = phrase
                        .then(|| ask_phrase("Recovery phrase (24 words): "))
                        .transpose()?;
                    c.call_raw(json!({"cmd":"role","device":device,"role":role,"phrase":phrase}))
                        .await?;
                    println!("{device} is now a {role}. It follows as soon as it hears about the change.");
                }
                DevicesCmd::MakeAdmin { device } => {
                    let phrase = ask_phrase("Recovery phrase (24 words): ")?;
                    c.call_raw(json!({"cmd":"make_admin","device":device,"phrase":phrase}))
                        .await?;
                    println!("{device} is now an admin. Your other devices are told.");
                }
                DevicesCmd::ReplacePhrase => {
                    let phrase = ask_phrase("Current recovery phrase (24 words): ")?;
                    let d = c
                        .call_raw(json!({"cmd":"replace_phrase","phrase":phrase}))
                        .await?;
                    println!(
                        "The recovery phrase was replaced. The old one no longer works.\n\n\
                         NEW RECOVERY PHRASE (shown once; write it down and keep it offline):\n\n  {}\n\n{}",
                        d["phrase"].as_str().unwrap_or(""),
                        d["kit"].as_str().unwrap_or("")
                    );
                }
            }
        }
        Cmd::Album { action } => {
            let mut c = client(&storage).await?;
            match action {
                AlbumCmd::List { json } => {
                    let data = c.call_raw(json!({"cmd":"album","action":"list"})).await?;
                    print(json, &data, render::albums);
                }
                AlbumCmd::Keep { id } => {
                    let d = c
                        .call_raw(json!({"cmd":"album","action":"keep","id":id}))
                        .await?;
                    println!(
                        "Keeping every original of “{}”.",
                        d["name"].as_str().unwrap_or(&id)
                    );
                }
                AlbumCmd::Drop { id } => {
                    let d = c
                        .call_raw(json!({"cmd":"album","action":"drop","id":id}))
                        .await?;
                    println!(
                        "No longer keeping originals of “{}”.",
                        d["name"].as_str().unwrap_or(&id)
                    );
                }
                AlbumCmd::Join { invitation } => {
                    let d = c
                        .call_raw(json!({"cmd":"album","action":"join","text":invitation}))
                        .await?;
                    println!("Joined “{}”.", d["name"].as_str().unwrap_or("the album"));
                }
            }
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
