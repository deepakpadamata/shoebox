//! Fetching one blob from one holder versus several at once (A-099), across real devices.
//!
//! ```text
//! fetch_bench serve <MiB> <dir>                 hold <MiB> of deterministic bytes, print the get command
//! fetch_bench get <MiB> <hash> <dir> <id>...    fetch it from each holder alone, then from all at once
//! ```
//!
//! The bytes are the same on every holder for the same size, so holders agree on the hash. Nodes
//! use the public relays and address lookup, like the app; nothing touches a library.

use anyhow::{anyhow, bail, Context, Result};
use shoebox_core::ids::Blake3Hash;
use shoebox_node::{Node, NodeConfig, Protection, RelaySetting};
use std::path::{Path, PathBuf};
use std::time::Instant;

fn bytes(mib: usize) -> Vec<u8> {
    let mut x = 0x9E37_79B9_7F4A_7C15u64;
    (0..mib * 1024 * 1024)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

async fn node(dir: &Path, name: &str) -> Result<Node> {
    let mut cfg = NodeConfig::local_only(dir, name);
    cfg.relay = RelaySetting::Default;
    cfg.mdns = true;
    cfg.gc_interval_ms = None;
    Node::open(cfg).await
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("serve") if args.len() == 3 => serve(args[1].parse()?, PathBuf::from(&args[2])).await,
        Some("get") if args.len() >= 5 => {
            let hash = Blake3Hash(
                hex::decode(&args[2])?
                    .try_into()
                    .map_err(|_| anyhow!("not a hash"))?,
            );
            let ids = args[4..]
                .iter()
                .map(|s| s.parse().context("not an endpoint id"))
                .collect::<Result<Vec<_>>>()?;
            get(args[1].parse()?, hash, PathBuf::from(&args[3]), ids).await
        }
        _ => bail!("usage: fetch_bench serve <MiB> <dir> | get <MiB> <hash> <dir> <id>..."),
    }
}

async fn serve(mib: usize, dir: PathBuf) -> Result<()> {
    let node = node(&dir, "bench-holder").await?;
    let hash = node
        .blobs()
        .add_bytes(bytes(mib), Protection::Library)
        .await?;
    println!("holder {} has {hash}", node.id());
    println!("get {mib} {hash} <dir> {}", node.id());
    // Serve until killed.
    std::future::pending::<()>().await;
    node.shutdown().await
}

async fn get(mib: usize, hash: Blake3Hash, dir: PathBuf, ids: Vec<iroh::EndpointId>) -> Result<()> {
    let size = (mib * 1024 * 1024) as u64;
    let mut runs: Vec<(String, Vec<iroh::EndpointId>)> = ids
        .iter()
        .map(|id| (format!("only {}", id.fmt_short()), vec![*id]))
        .collect();
    if ids.len() > 1 {
        runs.push(("all at once".into(), ids.clone()));
    }
    for (i, (label, holders)) in runs.into_iter().enumerate() {
        let run_dir = dir.join(format!("run-{i}"));
        let _ = std::fs::remove_dir_all(&run_dir);
        let node = node(&run_dir, "bench-fetcher").await?;
        let started = Instant::now();
        let report = node.download_blob_from(&holders, &hash, Some(size)).await;
        let secs = started.elapsed().as_secs_f64();
        match report {
            Ok(report) => {
                let paths: Vec<String> =
                    futures::future::join_all(holders.iter().map(|h| node.peer_info(*h)))
                        .await
                        .into_iter()
                        .flatten()
                        .map(|p| format!("{}={:?}", p.id.fmt_short(), p.path))
                        .collect();
                let split: Vec<String> = report
                    .sources
                    .iter()
                    .map(|(p, b)| {
                        format!("{}:{:.0}%", p.fmt_short(), *b as f64 * 100.0 / size as f64)
                    })
                    .collect();
                println!(
                    "{label:<16} {mib} MiB in {secs:6.2} s = {:6.1} MB/s   from [{}]   paths [{}]",
                    size as f64 / secs / 1e6,
                    split.join(" "),
                    paths.join(" ")
                );
            }
            Err(e) => println!("{label:<16} failed after {secs:.1} s: {e:#}"),
        }
        node.shutdown().await?;
        let _ = std::fs::remove_dir_all(&run_dir);
    }
    Ok(())
}
