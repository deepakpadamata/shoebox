//! What is moving right now (the Devices screen): the blobs this device is downloading and the
//! ones it is sending, with their progress, and the speed in each direction over the last few
//! seconds.
//!
//! Downloads are reported by [`crate::Node::download_blob_from`] as they go. Uploads come from
//! iroh-blobs' provider events (connections, get requests and their transfer progress; progress
//! updates are dropped rather than slow a transfer down).
//!
//! Get requests are intercepted for one rule: while serving is paused (a phone on mobile data with
//! "Wi-Fi only" set, [`crate::Node::pause_serving`]), a blob larger than [`PAUSED_SERVE_MAX`] is
//! refused as rate limited, which a peer treats as "try again later". Peers ask by hash and the
//! blob layer does not know an original from a preview, so the size decides: thumbnails and
//! previews still go, originals wait.

use iroh::EndpointId as IrohId;
use iroh_blobs::provider::events::{
    AbortReason, ConnectMode, EventMask, EventSender, ObserveMode, ProviderMessage, RequestMode,
    RequestUpdate, ThrottleMode,
};
use shoebox_core::ids::{Blake3Hash, EndpointId};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Speeds are averaged over this long.
const WINDOW: Duration = Duration::from_secs(5);

/// While serving is paused, blobs up to this size are still sent: thumbnails and previews (a
/// 2048 px preview is a few hundred KB), not originals.
pub const PAUSED_SERVE_MAX: u64 = 1024 * 1024;

/// One blob on its way in or out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transfer {
    pub hash: Blake3Hash,
    /// The device it comes from (downloads) or goes to (uploads), when known.
    pub peer: Option<EndpointId>,
    /// Size of the whole blob, when known.
    pub size: Option<u64>,
    /// Bytes moved so far in this transfer.
    pub done: u64,
}

/// A snapshot of [`Transfers`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TransferSnapshot {
    pub downloads: Vec<Transfer>,
    pub uploads: Vec<Transfer>,
    /// Bytes per second over the last few seconds.
    pub download_rate: u64,
    pub upload_rate: u64,
}

#[derive(Default)]
struct Rate(VecDeque<(Instant, u64)>);

impl Rate {
    fn add(&mut self, bytes: u64) {
        if bytes > 0 {
            self.0.push_back((Instant::now(), bytes));
        }
        self.trim();
    }
    fn trim(&mut self) {
        while self.0.front().is_some_and(|(t, _)| t.elapsed() > WINDOW) {
            self.0.pop_front();
        }
    }
    fn per_second(&mut self) -> u64 {
        self.trim();
        let bytes: u64 = self.0.iter().map(|(_, b)| b).sum();
        bytes / WINDOW.as_secs().max(1)
    }
}

struct Download {
    transfer: Transfer,
    /// Bytes reported by the current phase (a split fetch, then one holder at a time, each count
    /// from zero) and what earlier phases moved.
    phase: u64,
    earlier: u64,
    /// Downloads of the same blob at once (the fetcher and "fetch now").
    count: usize,
}

struct Upload {
    transfer: Transfer,
    /// End offset of the last progress update, to turn offsets into bytes sent.
    offset: Option<u64>,
}

#[derive(Default)]
struct State {
    downloads: HashMap<Blake3Hash, Download>,
    /// By (connection, request).
    uploads: HashMap<(u64, u64), Upload>,
    peers: HashMap<u64, IrohId>,
    down: Rate,
    up: Rate,
}

/// Shared by the node and its blob protocol. Cheap to clone.
#[derive(Clone, Default)]
pub(crate) struct Transfers(Arc<Mutex<State>>, Arc<AtomicBool>);

/// Ends a download's entry when the download returns, however it returns.
pub(crate) struct DownloadGuard {
    transfers: Transfers,
    hash: Blake3Hash,
}

impl Drop for DownloadGuard {
    fn drop(&mut self) {
        let mut s = self.transfers.0.lock().unwrap();
        if let Some(d) = s.downloads.get_mut(&self.hash) {
            d.count -= 1;
            if d.count == 0 {
                s.downloads.remove(&self.hash);
            }
        }
    }
}

impl Transfers {
    /// Refuse large blobs to peers (true) or serve everything again (false).
    pub(crate) fn set_serving_paused(&self, paused: bool) {
        self.1.store(paused, Ordering::SeqCst);
    }

    pub(crate) fn serving_paused(&self) -> bool {
        self.1.load(Ordering::SeqCst)
    }

    /// Whether a get request for `hash` is served now: always, unless serving is paused and the
    /// blob is larger than [`PAUSED_SERVE_MAX`]. A blob not stored here is left to the provider.
    async fn may_serve(&self, blobs: &crate::blobs::BlobStore, hash: &Blake3Hash) -> bool {
        if !self.serving_paused() {
            return true;
        }
        !matches!(blobs.size(hash).await, Ok(Some(size)) if size > PAUSED_SERVE_MAX)
    }

    pub(crate) fn snapshot(&self) -> TransferSnapshot {
        let mut s = self.0.lock().unwrap();
        let mut downloads: Vec<Transfer> =
            s.downloads.values().map(|d| d.transfer.clone()).collect();
        let mut uploads: Vec<Transfer> = s.uploads.values().map(|u| u.transfer.clone()).collect();
        downloads.sort_by_key(|t| t.hash);
        uploads.sort_by_key(|t| t.hash);
        TransferSnapshot {
            downloads,
            uploads,
            download_rate: s.down.per_second(),
            upload_rate: s.up.per_second(),
        }
    }

    pub(crate) fn start_download(
        &self,
        hash: Blake3Hash,
        size: Option<u64>,
        peer: IrohId,
    ) -> DownloadGuard {
        let mut s = self.0.lock().unwrap();
        s.downloads
            .entry(hash)
            .or_insert_with(|| Download {
                transfer: Transfer {
                    hash,
                    peer: Some(crate::addr::from_iroh_id(&peer)),
                    size,
                    done: 0,
                },
                phase: 0,
                earlier: 0,
                count: 0,
            })
            .count += 1;
        DownloadGuard {
            transfers: self.clone(),
            hash,
        }
    }

    /// `bytes` received so far by the current phase of the download of `hash`, the latest from
    /// `peer`. A count lower than the last one is a new phase starting from zero.
    pub(crate) fn download_progress(&self, hash: &Blake3Hash, peer: IrohId, bytes: u64) {
        let mut s = self.0.lock().unwrap();
        let State {
            downloads, down, ..
        } = &mut *s;
        let Some(d) = downloads.get_mut(hash) else {
            return;
        };
        let new = if bytes < d.phase {
            d.earlier += d.phase;
            bytes
        } else {
            bytes - d.phase
        };
        d.phase = bytes;
        d.transfer.done = d.earlier + d.phase;
        d.transfer.peer = Some(crate::addr::from_iroh_id(&peer));
        down.add(new);
    }

    /// The provider's event sender: connections are notifications, get requests are intercepted
    /// (see [`Transfers::may_serve`]) and followed, push stays off.
    pub(crate) fn provider_events(&self, blobs: crate::blobs::BlobStore) -> EventSender {
        let mask = EventMask {
            connected: ConnectMode::Notify,
            get: RequestMode::InterceptLog,
            get_many: RequestMode::None,
            push: RequestMode::Disabled,
            observe: ObserveMode::None,
            throttle: ThrottleMode::None,
        };
        let (sender, mut rx) = EventSender::channel(256, mask);
        let transfers = self.clone();
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                match msg {
                    ProviderMessage::ClientConnectedNotify(m) => {
                        if let Some(peer) = m.inner.endpoint_id {
                            transfers
                                .0
                                .lock()
                                .unwrap()
                                .peers
                                .insert(m.inner.connection_id, peer);
                        }
                    }
                    ProviderMessage::ConnectionClosed(m) => {
                        let id = m.inner.connection_id;
                        let mut s = transfers.0.lock().unwrap();
                        s.peers.remove(&id);
                        s.uploads.retain(|(c, _), _| *c != id);
                    }
                    ProviderMessage::GetRequestReceived(m) => {
                        let key = (m.inner.connection_id, m.inner.request_id);
                        let hash = Blake3Hash(*m.inner.request.hash.as_bytes());
                        let (reply, mut updates) = (m.tx, m.rx);
                        let (t, blobs) = (transfers.clone(), blobs.clone());
                        // Its own task: a size lookup must not hold up other requests' events.
                        tokio::spawn(async move {
                            if !t.may_serve(&blobs, &hash).await {
                                tracing::debug!("not serving {hash} while serving is paused");
                                reply.send(Err(AbortReason::RateLimited)).await.ok();
                                return;
                            }
                            reply.send(Ok(())).await.ok();
                            while let Ok(Some(update)) = updates.recv().await {
                                t.upload_update(key, update);
                            }
                            t.0.lock().unwrap().uploads.remove(&key);
                        });
                    }
                    _ => {}
                }
            }
        });
        sender
    }

    fn upload_update(&self, key: (u64, u64), update: RequestUpdate) {
        let mut s = self.0.lock().unwrap();
        let peer = s.peers.get(&key.0).map(crate::addr::from_iroh_id);
        let State { uploads, up, .. } = &mut *s;
        match update {
            RequestUpdate::Started(t) => {
                uploads.insert(
                    key,
                    Upload {
                        transfer: Transfer {
                            hash: Blake3Hash(*t.hash.as_bytes()),
                            peer,
                            size: Some(t.size),
                            done: 0,
                        },
                        offset: None,
                    },
                );
            }
            RequestUpdate::Progress(p) => {
                if let Some(u) = uploads.get_mut(&key) {
                    // A ranged request starts mid-blob: the first offset only sets the baseline.
                    let sent = u.offset.map_or(0, |o| p.end_offset.saturating_sub(o));
                    u.offset = Some(p.end_offset);
                    u.transfer.done += sent;
                    up.add(sent);
                }
            }
            RequestUpdate::Completed(c) => {
                if let Some(u) = uploads.remove(&key) {
                    up.add(c.stats.payload_bytes_sent.saturating_sub(u.transfer.done));
                }
            }
            RequestUpdate::Aborted(_) => {
                uploads.remove(&key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(n: u8) -> IrohId {
        iroh::SecretKey::from_bytes(&[n; 32]).public()
    }

    #[test]
    fn a_download_counts_its_phases_and_ends_with_its_last_guard() {
        let t = Transfers::default();
        let h = Blake3Hash([1; 32]);
        let first = t.start_download(h, Some(100), peer(1));
        let second = t.start_download(h, Some(100), peer(1));
        t.download_progress(&h, peer(1), 30);
        t.download_progress(&h, peer(2), 50);
        // The one-at-a-time phase counts from zero again.
        t.download_progress(&h, peer(2), 20);
        let snap = t.snapshot();
        assert_eq!(snap.downloads.len(), 1);
        assert_eq!(snap.downloads[0].done, 70);
        assert_eq!(
            snap.downloads[0].peer,
            Some(crate::addr::from_iroh_id(&peer(2)))
        );
        assert_eq!(snap.download_rate, 70 / WINDOW.as_secs());
        drop(first);
        assert_eq!(
            t.snapshot().downloads.len(),
            1,
            "the other download still runs"
        );
        drop(second);
        assert!(t.snapshot().downloads.is_empty());
    }
}
