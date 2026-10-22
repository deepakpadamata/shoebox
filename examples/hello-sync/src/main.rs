//! Throwaway spike for M0-01: two in-process iroh endpoints on loopback (no relay, no
//! discovery service) that (1) transfer a blob and (2) sync one iroh-docs entry.
//!
//! Run with `cargo run -p hello-sync`.

use std::time::Duration;

use anyhow::{Context, Result};
use futures::StreamExt;
use iroh::{address_lookup::memory::MemoryLookup, endpoint::presets, protocol::Router, Endpoint};
use iroh_blobs::{store::mem::MemStore, BlobsProtocol, ALPN as BLOBS_ALPN};
use iroh_docs::{
    api::protocol::{AddrInfoOptions, ShareMode},
    engine::LiveEvent,
    protocol::Docs,
    ALPN as DOCS_ALPN,
};
use iroh_gossip::{net::Gossip, ALPN as GOSSIP_ALPN};

struct Node {
    endpoint: Endpoint,
    lookup: MemoryLookup,
    blobs: MemStore,
    docs: Docs,
    router: Router,
}

async fn spawn_node() -> Result<Node> {
    let lookup = MemoryLookup::new();
    let endpoint = Endpoint::builder(presets::Minimal)
        .address_lookup(lookup.clone())
        .bind()
        .await?;
    let blobs = MemStore::default();
    let gossip = Gossip::builder().spawn(endpoint.clone());
    let docs = Docs::memory()
        .spawn(endpoint.clone(), (*blobs).clone(), gossip.clone())
        .await?;
    let router = Router::builder(endpoint.clone())
        .accept(BLOBS_ALPN, BlobsProtocol::new(&blobs, None))
        .accept(GOSSIP_ALPN, gossip)
        .accept(DOCS_ALPN, docs.clone())
        .spawn();
    Ok(Node {
        endpoint,
        lookup,
        blobs,
        docs,
        router,
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    let a = spawn_node().await?;
    let b = spawn_node().await?;
    // No discovery service: tell each node how to reach the other (loopback sockets).
    a.lookup.add_endpoint_info(b.endpoint.addr());
    b.lookup.add_endpoint_info(a.endpoint.addr());

    // --- 1. blob transfer (custom ALPN is exercised by the docs/gossip/blobs ALPNs above) ---
    let payload = b"hello from a, via iroh-blobs".to_vec();
    let tag = a.blobs.blobs().add_bytes(payload.clone()).await?;
    b.blobs
        .downloader(&b.endpoint)
        .download(tag.hash, Some(a.endpoint.id()))
        .await
        .context("blob download")?;
    let got = b.blobs.blobs().get_bytes(tag.hash).await?;
    assert_eq!(got.as_ref(), payload.as_slice());
    println!("blob transferred: {} ({} bytes)", tag.hash, got.len());

    // --- 2. iroh-docs entry sync ---
    let author_a = a.docs.author_default().await?;
    let doc_a = a.docs.create().await?;
    doc_a.set_bytes(author_a, "photos/abc", "record-v1").await?;
    let ticket = doc_a
        .share(ShareMode::Write, AddrInfoOptions::Addresses)
        .await?;

    let (doc_b, mut events) = b.docs.import_and_subscribe(ticket).await?;
    let wait = async {
        while let Some(ev) = events.next().await {
            if let LiveEvent::InsertRemote { .. } = ev? {
                return anyhow::Ok(());
            }
        }
        anyhow::bail!("event stream ended")
    };
    tokio::time::timeout(Duration::from_secs(20), wait)
        .await
        .context("timed out waiting for the remote insert")??;

    let entry = doc_b
        .get_one(iroh_docs::store::Query::single_latest_per_key().key_exact("photos/abc"))
        .await?
        .context("entry not found on b")?;
    println!(
        "docs entry synced: key={:?} hash={}",
        entry.key(),
        entry.content_hash()
    );
    assert_eq!(entry.key(), b"photos/abc");

    a.router.shutdown().await?;
    b.router.shutdown().await?;
    println!("OK");
    Ok(())
}
