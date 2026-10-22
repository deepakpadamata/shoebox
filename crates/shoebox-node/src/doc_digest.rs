//! A tiny protocol to ask a peer for a digest of one of its iroh-docs namespaces.
//!
//! iroh-docs 0.101 on iroh 1.4 cannot tell us reliably whether a sync actually moved data (its
//! result is an error from the closing handshake after nearly every exchange). Two replicas have
//! converged exactly when their entry digests are equal, so callers compare digests and sync
//! again until they match.

use anyhow::{anyhow, Result};
use futures::StreamExt;
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use iroh_docs::api::Doc;
use iroh_docs::protocol::Docs;
use iroh_docs::store::Query;
use iroh_docs::NamespaceId;

/// ALPN of the digest protocol.
pub const DOC_DIGEST_ALPN: &[u8] = b"shoebox/doc-digest/0";

/// Order-independent digest of every entry (including deletion markers) of `doc`: author, key,
/// content hash and timestamp. Content availability does not matter.
pub async fn doc_digest(doc: &Doc) -> Result<[u8; 32]> {
    let stream = doc
        .get_many(Query::all().include_empty().build())
        .await
        .map_err(|e| anyhow!("listing entries: {e}"))?;
    let mut stream = Box::pin(stream);
    let mut entries: Vec<[u8; 32]> = vec![];
    while let Some(entry) = stream.next().await {
        let entry = entry.map_err(|e| anyhow!("reading entry: {e}"))?;
        let mut h = blake3::Hasher::new();
        h.update(entry.author().as_bytes());
        h.update(&(entry.key().len() as u64).to_le_bytes());
        h.update(entry.key());
        h.update(entry.content_hash().as_bytes());
        h.update(&entry.timestamp().to_le_bytes());
        entries.push(*h.finalize().as_bytes());
    }
    entries.sort_unstable();
    let mut all = blake3::Hasher::new();
    all.update(&(entries.len() as u64).to_le_bytes());
    for e in &entries {
        all.update(e);
    }
    Ok(*all.finalize().as_bytes())
}

/// Serves digests of the namespaces this node has open.
#[derive(Debug, Clone)]
pub(crate) struct DigestProtocol {
    pub docs: Docs,
}

impl ProtocolHandler for DigestProtocol {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let (mut send, mut recv) = connection.accept_bi().await?;
        let request = recv.read_to_end(64).await.map_err(AcceptError::from_err)?;
        let reply = match <[u8; 32]>::try_from(request.as_slice()) {
            Ok(ns) => match self.docs.open(NamespaceId::from(ns)).await {
                Ok(Some(doc)) => match doc_digest(&doc).await {
                    Ok(digest) => [&[1u8][..], &digest].concat(),
                    Err(_) => vec![0],
                },
                _ => vec![0],
            },
            Err(_) => vec![0],
        };
        send.write_all(&reply)
            .await
            .map_err(AcceptError::from_err)?;
        send.finish()?;
        connection.closed().await;
        Ok(())
    }
}
