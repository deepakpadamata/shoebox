//! One iroh-docs namespace as a key → bytes store with change detection and sync waiting.

use anyhow::{anyhow, bail, Result};
use futures::StreamExt;
use iroh_blobs::Hash;
use iroh_docs::api::Doc;
use iroh_docs::engine::LiveEvent;
use iroh_docs::store::Query;
use iroh_docs::AuthorId;
use shoebox_node::Node;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Notify;
use tracing::{debug, warn};

/// How long [`Replica::sync_with`] waits for a peer before giving up.
pub const SYNC_TIMEOUT: Duration = Duration::from_secs(30);

/// Prefix of the iroh-docs error for a failure in the connection-closing phase.
const CLOSE_PHASE_ERROR: &str = "Failed to close connection";

/// Content that has been missing this long is fetched by us instead of waiting for iroh-docs.
const FETCH_AFTER: Duration = Duration::from_millis(750);
/// How long freshly written entry content stays protected from blob GC after the entry exists.
const CONTENT_HOLD: Duration = Duration::from_secs(60);
/// After a failed content fetch, wait this long before trying the same hash again.
const FETCH_RETRY: Duration = Duration::from_secs(4);
/// In a sync round, ask iroh-docs again when nothing happened for this long.
const ASK_AGAIN: Duration = Duration::from_secs(2);
/// How often a round asks before giving up (stale finishes and silent requests together).
const MAX_ASKS: usize = 8;

pub struct Replica {
    pub doc: Doc,
    pub author: AuthorId,
    node: Node,
    /// `(author, key)` → content hash already applied, so a scan only returns what changed.
    known: HashMap<(AuthorId, Vec<u8>), Hash>,
    /// Peers this namespace has exchanged entries with: candidate sources for missing content.
    peers: Arc<Mutex<HashSet<iroh::EndpointId>>>,
    /// When each still-missing content hash was first noticed.
    missing_since: HashMap<Hash, Instant>,
    /// Hashes with a fetch task in flight (or cooling down after a failure).
    fetching: Arc<Mutex<HashSet<Hash>>>,
    /// Signalled when something may have changed and a rescan is worthwhile.
    dirty: Arc<Notify>,
    /// Temp tags protecting recently written entry content from blob GC (see [`Replica::put`]).
    recent: VecDeque<(Instant, iroh_blobs::api::TempTag)>,
}

impl Replica {
    pub fn new(doc: Doc, author: AuthorId, node: Node) -> Self {
        Self {
            doc,
            author,
            node,
            known: HashMap::new(),
            peers: Default::default(),
            missing_since: HashMap::new(),
            fetching: Default::default(),
            dirty: Arc::new(Notify::new()),
            recent: VecDeque::new(),
        }
    }

    pub fn node(&self) -> &Node {
        &self.node
    }

    /// Wake the rescan task.
    pub fn dirty(&self) -> Arc<Notify> {
        self.dirty.clone()
    }

    /// Shared set of peers that entries came from (updated by the event-drain task).
    pub fn peers_handle(&self) -> Arc<Mutex<HashSet<iroh::EndpointId>>> {
        self.peers.clone()
    }

    pub fn note_peer(&self, id: iroh::EndpointId) {
        self.peers.lock().unwrap().insert(id);
    }

    /// Download content that iroh-docs failed to fetch (it tries once per sync run, and a later
    /// sync of an already-known entry does not ask again). Retries every few seconds.
    fn fetch_missing(&self, hashes: Vec<Hash>) {
        self.node.note_activity();
        let peers: Vec<iroh::EndpointId> = self.peers.lock().unwrap().iter().copied().collect();
        if peers.is_empty() || hashes.is_empty() {
            return;
        }
        let todo: Vec<Hash> = {
            let mut busy = self.fetching.lock().unwrap();
            hashes.into_iter().filter(|h| busy.insert(*h)).collect()
        };
        if todo.is_empty() {
            return;
        }
        let node = self.node.clone();
        let fetching = self.fetching.clone();
        let dirty = self.dirty.clone();
        tokio::spawn(async move {
            let downloader = node.blobs().raw().downloader(node.endpoint());
            for hash in todo {
                let result = tokio::time::timeout(
                    Duration::from_secs(30),
                    downloader.download(hash, peers.clone()),
                )
                .await;
                match result {
                    Ok(Ok(_)) => {
                        fetching.lock().unwrap().remove(&hash);
                        dirty.notify_one();
                    }
                    other => {
                        debug!("fetching missing docs content {hash} failed: {other:?}");
                        let fetching = fetching.clone();
                        tokio::spawn(async move {
                            tokio::time::sleep(FETCH_RETRY).await;
                            fetching.lock().unwrap().remove(&hash);
                        });
                    }
                }
            }
        });
    }

    /// Write `value` under `key` as this replica's author.
    ///
    /// The content blob is added by us and kept under a temp tag for [`CONTENT_HOLD`] after the
    /// entry is inserted. Without that, blob GC can delete a just-written entry's content: GC
    /// snapshots the hashes docs references, then sweeps, and an entry written in between is
    /// neither in the snapshot nor tagged. (Reproduced with raw iroh-docs: ~0.3% of entries lost
    /// their content on the writer under a 100 ms GC interval.)
    pub async fn put(&mut self, key: &str, value: Vec<u8>) -> Result<()> {
        self.node.note_activity();
        let len = value.len() as u64;
        let tag = self
            .node
            .blobs()
            .raw()
            .blobs()
            .add_bytes(value)
            .temp_tag()
            .await
            .map_err(|e| anyhow!("storing content for {key}: {e}"))?;
        let hash = tag.hash();
        self.doc
            .set_hash(self.author, key.as_bytes().to_vec(), hash, len)
            .await
            .map_err(|e| anyhow!("writing {key}: {e}"))?;
        let now = Instant::now();
        self.recent.push_back((now, tag));
        while self
            .recent
            .front()
            .is_some_and(|(t, _)| now.duration_since(*t) > CONTENT_HOLD)
        {
            self.recent.pop_front();
        }
        self.known
            .insert((self.author, key.as_bytes().to_vec()), hash);
        Ok(())
    }

    /// Delete this author's entry for `key` (other authors' entries are untouched).
    pub async fn del(&mut self, key: &str) -> Result<()> {
        self.node.note_activity();
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
        self.node.note_activity();
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
        let mut overdue = vec![];
        let mut still_missing = HashSet::new();
        for ((author, key), hash) in candidates {
            let core_hash = shoebox_core::Blake3Hash(*hash.as_bytes());
            if !blobs.has(&core_hash).await? {
                // Content still downloading (or never arrived).
                pending += 1;
                still_missing.insert(hash);
                let since = *self.missing_since.entry(hash).or_insert_with(Instant::now);
                if since.elapsed() >= FETCH_AFTER {
                    overdue.push(hash);
                }
                continue;
            }
            let bytes = blobs.get(&core_hash).await?;
            self.known.insert((author, key.clone()), hash);
            match String::from_utf8(key) {
                Ok(key) => out.push((key, bytes.to_vec())),
                Err(_) => warn!("ignoring entry with a non-UTF-8 key"),
            }
        }
        self.missing_since.retain(|h, _| still_missing.contains(h));
        if pending > 0 {
            self.fetch_missing(overdue);
            // Keep looking until everything has arrived, even if no further event comes.
            let dirty = self.dirty.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(500)).await;
                dirty.notify_one();
            });
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

/// Rounds of "sync, then compare digests" before giving up on exact agreement (live writes on
/// either side can keep the digests moving; the library converges via gossip regardless).
const MAX_ROUNDS: usize = 6;

/// Sync `doc` with `peer` and return once both sides hold the same entries (verified by comparing
/// digests, see `shoebox_node::doc_digest`) and the content we queued has arrived.
pub async fn sync_and_wait<S>(
    doc: &Doc,
    node: &Node,
    events: &mut S,
    peer: iroh::EndpointAddr,
) -> Result<()>
where
    S: futures::Stream<Item = Result<LiveEvent, anyhow::Error>> + Unpin,
{
    let overall = async {
        for round in 0..MAX_ROUNDS {
            sync_round(doc, events, &peer).await?;
            let mine = shoebox_node::doc_digest(doc).await?;
            match node.peer_doc_digest(peer.id, doc.id()).await {
                Ok(Some(theirs)) if theirs == mine => return Ok(()),
                Ok(Some(_)) => {
                    debug!("digests differ after round {round}; syncing again");
                    tokio::time::sleep(Duration::from_millis(40)).await;
                }
                // The peer cannot (or will not) tell us: trust the sync event.
                Ok(None) | Err(_) => return Ok(()),
            }
        }
        debug!("digests still differ after {MAX_ROUNDS} rounds (concurrent writes?)");
        Ok(())
    };
    tokio::time::timeout(SYNC_TIMEOUT, overall)
        .await
        .map_err(|_| anyhow!("sync with {} timed out", peer.id.fmt_short()))?
}

/// Start syncing with `peer` and return as soon as the entry `key` is here with its content,
/// leaving the rest of the sync running in the background. Joining a big album must not wait for
/// all of its entries (tens of thousands take minutes), only for what is needed to open it.
pub async fn sync_until_key<S>(
    doc: &Doc,
    node: &Node,
    events: &mut S,
    peer: iroh::EndpointAddr,
    key: &[u8],
    timeout: Duration,
) -> Result<()>
where
    S: futures::Stream<Item = Result<LiveEvent, anyhow::Error>> + Unpin,
{
    doc.start_sync(vec![peer.clone()])
        .await
        .map_err(|e| anyhow!("starting sync: {e}"))?;
    let deadline = Instant::now() + timeout;
    let mut asked = Instant::now();
    loop {
        node.note_activity();
        // iroh-docs ignores a request while a sync with the same peer is running, and that sync
        // may have started before the data we want existed: ask again now and then.
        if asked.elapsed() >= Duration::from_secs(2) {
            asked = Instant::now();
            doc.start_sync(vec![peer.clone()])
                .await
                .map_err(|e| anyhow!("starting sync: {e}"))?;
        }
        let stream = doc
            .get_many(Query::key_exact(key).build())
            .await
            .map_err(|e| anyhow!("looking for an entry: {e}"))?;
        let mut stream = Box::pin(stream);
        if let Some(entry) = stream.next().await {
            let entry = entry.map_err(|e| anyhow!("reading entry: {e}"))?;
            let hash = shoebox_core::Blake3Hash(*entry.content_hash().as_bytes());
            if node.blobs().has(&hash).await? {
                return Ok(());
            }
            // The entry came from some peer, but docs fetches its content only from the peers
            // that announced it, and one of those may be a member that does not know us yet. The
            // peer we are joining through does: ask it directly.
            if node.download_blob(peer.id, &hash).await.is_ok() {
                return Ok(());
            }
        }
        drop(stream);
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            bail!("no answer from {}", peer.id.fmt_short());
        }
        // Keep draining events (the engine stalls if a subscriber stops), and look again soon.
        tokio::select! {
            ev = events.next() => if ev.is_none() { bail!("event stream ended") },
            _ = tokio::time::sleep(Duration::from_millis(100).min(left)) => {}
        }
    }
}

/// One request for a sync with `peer`: wait until a sync that *started after the request* has
/// finished and the content it queued has arrived.
///
/// iroh-docs ignores a request while a sync with that peer is already running, and that running
/// sync may have begun before the data we expect existed. So when a stale sync finishes we ask
/// again, which then starts a fresh one.
async fn sync_round<S>(doc: &Doc, events: &mut S, peer: &iroh::EndpointAddr) -> Result<()>
where
    S: futures::Stream<Item = Result<LiveEvent, anyhow::Error>> + Unpin,
{
    let mut since = std::time::SystemTime::now();
    doc.start_sync(vec![peer.clone()])
        .await
        .map_err(|e| anyhow!("starting sync: {e}"))?;
    let mut finished = false;
    let mut retries = 0;
    loop {
        // iroh-docs sometimes drops a request without any event (R1-02: right after a sync that
        // ended with a close-phase error, the next request produced nothing for 10 s): do not wait
        // on the event alone, ask again now and then, like `sync_until_key`.
        let event = match tokio::time::timeout(ASK_AGAIN, events.next()).await {
            Ok(Some(event)) => event?,
            Ok(None) => bail!("event stream ended before the sync finished"),
            Err(_) if finished => {
                // The sync is done; the "content ready" event did not come. The caller compares
                // digests and fetches missing content itself, so stop waiting here.
                return Ok(());
            }
            Err(_) => {
                retries += 1;
                if retries > MAX_ASKS {
                    bail!("could not start a fresh sync with {}", peer.id.fmt_short());
                }
                debug!(
                    "no sync with {} started yet; asking again",
                    peer.id.fmt_short()
                );
                since = std::time::SystemTime::now();
                doc.start_sync(vec![peer.clone()])
                    .await
                    .map_err(|e| anyhow!("starting sync: {e}"))?;
                continue;
            }
        };
        match event {
            LiveEvent::SyncFinished(ev) if ev.peer == peer.id => {
                if ev.started < since {
                    // Stale: it began before our request. Ask for a fresh one.
                    retries += 1;
                    if retries > MAX_ASKS {
                        bail!("could not start a fresh sync with {}", peer.id.fmt_short());
                    }
                    since = std::time::SystemTime::now();
                    doc.start_sync(vec![peer.clone()])
                        .await
                        .map_err(|e| anyhow!("starting sync: {e}"))?;
                    continue;
                }
                if let Err(reason) = ev.result {
                    // iroh-docs 0.101 on iroh 1.4 reports "Failed to close connection" from its
                    // closing handshake after nearly every sync, including complete ones, so the
                    // event cannot tell us whether data moved; the caller verifies by digest.
                    if !reason.contains(CLOSE_PHASE_ERROR) {
                        bail!("sync with {} failed: {reason}", peer.id.fmt_short());
                    }
                    debug!(
                        "sync with {} ended with a close-phase error: {reason}",
                        peer.id.fmt_short()
                    );
                }
                finished = true;
            }
            LiveEvent::PendingContentReady if finished => return Ok(()),
            _ => {}
        }
    }
}
