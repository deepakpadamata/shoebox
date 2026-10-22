//! One iroh-docs namespace as a key → bytes store with change detection and sync waiting.

use anyhow::{anyhow, bail, Result};
use futures::StreamExt;
use iroh_blobs::Hash;
use iroh_docs::api::Doc;
use iroh_docs::engine::LiveEvent;
use iroh_docs::store::Query;
use iroh_docs::AuthorId;
use shoebox_node::Node;
use std::collections::HashMap;
use std::time::Duration;
use tracing::{debug, warn};

/// How long [`Replica::sync_with`] waits for a peer before giving up.
pub const SYNC_TIMEOUT: Duration = Duration::from_secs(30);

/// Prefix of the iroh-docs error for a failure in the connection-closing phase.
const CLOSE_PHASE_ERROR: &str = "Failed to close connection";

pub struct Replica {
    pub doc: Doc,
    pub author: AuthorId,
    node: Node,
    /// `(author, key)` → content hash already applied, so a scan only returns what changed.
    known: HashMap<(AuthorId, Vec<u8>), Hash>,
}

impl Replica {
    pub fn new(doc: Doc, author: AuthorId, node: Node) -> Self {
        Self {
            doc,
            author,
            node,
            known: HashMap::new(),
        }
    }

    /// Write `value` under `key` as this replica's author.
    pub async fn put(&mut self, key: &str, value: Vec<u8>) -> Result<()> {
        let hash = self
            .doc
            .set_bytes(self.author, key.as_bytes().to_vec(), value)
            .await
            .map_err(|e| anyhow!("writing {key}: {e}"))?;
        self.known
            .insert((self.author, key.as_bytes().to_vec()), hash);
        Ok(())
    }

    /// Delete this author's entry for `key` (other authors' entries are untouched).
    pub async fn del(&mut self, key: &str) -> Result<()> {
        self.doc
            .del(self.author, key.as_bytes().to_vec())
            .await
            .map_err(|e| anyhow!("deleting {key}: {e}"))?;
        self.known.remove(&(self.author, key.as_bytes().to_vec()));
        Ok(())
    }

    /// Entries (any author) that are new or changed since the last scan *and* whose content is
    /// already stored locally. Entries whose content has not arrived yet are returned by a later
    /// scan once it has.
    pub async fn scan(&mut self) -> Result<Scan> {
        let stream = self
            .doc
            .get_many(Query::all().build())
            .await
            .map_err(|e| anyhow!("listing entries: {e}"))?;
        let mut stream = Box::pin(stream);
        let mut candidates = vec![];
        while let Some(entry) = stream.next().await {
            let entry = entry.map_err(|e| anyhow!("reading entry: {e}"))?;
            let id = (entry.author(), entry.key().to_vec());
            if self.known.get(&id) == Some(&entry.content_hash()) {
                continue;
            }
            candidates.push((id, entry.content_hash()));
        }
        drop(stream);
        let blobs = self.node.blobs();
        let mut out = vec![];
        let mut pending = 0;
        for ((author, key), hash) in candidates {
            let core_hash = shoebox_core::Blake3Hash(*hash.as_bytes());
            if !blobs.has(&core_hash).await? {
                pending += 1; // content still downloading
                continue;
            }
            let bytes = blobs.get(&core_hash).await?;
            self.known.insert((author, key.clone()), hash);
            match String::from_utf8(key) {
                Ok(key) => out.push((key, bytes.to_vec())),
                Err(_) => warn!("ignoring entry with a non-UTF-8 key"),
            }
        }
        Ok(Scan {
            entries: out,
            pending,
        })
    }
}

/// Result of [`Replica::scan`].
pub struct Scan {
    /// New or changed entries whose content is local.
    pub entries: Vec<(String, Vec<u8>)>,
    /// Entries that are listed but whose content has not arrived yet.
    pub pending: usize,
}

/// Wait for a finished sync with `peer`, then for the content it queued to arrive.
pub async fn wait_synced<S>(events: &mut S, peer: iroh::EndpointId) -> Result<()>
where
    S: futures::Stream<Item = Result<LiveEvent, anyhow::Error>> + Unpin,
{
    let wait = async {
        let mut finished = false;
        while let Some(event) = events.next().await {
            match event? {
                LiveEvent::SyncFinished(ev) if ev.peer == peer => {
                    if let Err(reason) = ev.result {
                        // iroh-docs 0.101 on iroh 1.4 can report a failure from its *closing*
                        // handshake ("Failed to close connection") after the exchange itself
                        // completed and the entries were applied. That is not a failed sync.
                        if !reason.contains(CLOSE_PHASE_ERROR) {
                            bail!("sync with {} failed: {reason}", peer.fmt_short());
                        }
                        debug!(
                            "sync with {} ended with a close-phase error: {reason}",
                            peer.fmt_short()
                        );
                    }
                    finished = true;
                }
                LiveEvent::PendingContentReady if finished => return Ok(()),
                _ => {}
            }
        }
        bail!("event stream ended before the sync finished")
    };
    tokio::time::timeout(SYNC_TIMEOUT, wait)
        .await
        .map_err(|_| anyhow!("sync with {} timed out", peer.fmt_short()))?
}
