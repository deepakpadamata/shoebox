//! Keeps this library in sync with its peers: on connect, periodically, and with exponential
//! backoff after failures. The store backend does the actual reconciliation (and, for iroh-docs,
//! live gossip sync); this manager makes sure every known peer is *tried* and reports progress.

use crate::library_node::{Inner, LibraryNode};
use crate::node::{NodeEvent, NodeEvents};
use anyhow::{anyhow, Result};
use shoebox_core::ids::EndpointId;
use shoebox_core::store::{LibraryStore, PeerAddr};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::debug;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SyncPhase {
    Started,
    Finished,
    Failed(String),
    /// The next attempt is scheduled after this many milliseconds.
    Retrying {
        in_ms: u64,
    },
}

#[derive(Clone, Debug)]
pub struct SyncConfig {
    /// How often the manager looks for peers that are due.
    pub tick: Duration,
    /// Re-sync a healthy peer at least this often (a safety net behind live sync).
    pub resync_interval: Duration,
    /// First retry delay after a failure; doubles per consecutive failure.
    pub backoff_min: Duration,
    pub backoff_max: Duration,
    /// Give up on one sync attempt after this long.
    pub timeout: Duration,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            tick: Duration::from_secs(1),
            resync_interval: Duration::from_secs(300),
            backoff_min: Duration::from_secs(1),
            backoff_max: Duration::from_secs(60),
            timeout: Duration::from_secs(60),
        }
    }
}

pub(crate) struct PeerState {
    pub addr: PeerAddr,
    failures: u32,
    next_attempt: Instant,
    last_attempt: Option<Instant>,
    syncing: bool,
}

impl PeerState {
    pub(crate) fn new(addr: PeerAddr) -> Self {
        Self {
            addr,
            failures: 0,
            next_attempt: Instant::now(),
            last_attempt: None,
            syncing: false,
        }
    }
}

fn backoff(cfg: &SyncConfig, failures: u32) -> Duration {
    let factor = 2u32.saturating_pow(failures.saturating_sub(1).min(16));
    cfg.backoff_min.saturating_mul(factor).min(cfg.backoff_max)
}

impl<L: LibraryStore> LibraryNode<L> {
    /// Sync with `addr` now and whenever it is due. Also teaches the node how to reach it.
    pub fn add_peer(&self, addr: PeerAddr) -> Result<()> {
        self.node()
            .add_peer_addr(crate::addr::to_endpoint_addr(&addr).map_err(|e| anyhow!("{e}"))?);
        let id = addr.id;
        self.inner
            .peers
            .lock()
            .unwrap()
            .entry(id)
            .or_insert_with(|| PeerState::new(addr));
        let _ = self.inner.nudge.send(id);
        self.presence_join(&id);
        Ok(())
    }

    /// Ids of the peers this library syncs with.
    pub fn peers(&self) -> Vec<EndpointId> {
        let mut ids: Vec<EndpointId> = self.inner.peers.lock().unwrap().keys().copied().collect();
        ids.sort();
        ids
    }

    /// Sync with a known peer right now and report the outcome (also used by the manager).
    pub async fn sync_now(&self, peer: EndpointId) -> Result<()> {
        sync_peer(&self.inner, peer).await
    }

    pub(crate) fn start_sync_manager(&self, rx: mpsc::UnboundedReceiver<EndpointId>) {
        let weak = Arc::downgrade(&self.inner);
        let events = self.node().events();
        let tick = self.inner.sync_config.tick;
        tokio::spawn(manager(weak, events, rx, tick));
    }
}

async fn manager<L: LibraryStore>(
    weak: Weak<Inner<L>>,
    mut events: NodeEvents,
    mut nudge: mpsc::UnboundedReceiver<EndpointId>,
    tick: Duration,
) {
    let mut ticker = tokio::time::interval(tick);
    loop {
        let due: Vec<EndpointId> = tokio::select! {
            ev = events.recv() => match ev {
                Ok(NodeEvent::PeerConnected { id }) => vec![crate::addr::from_iroh_id(&id)],
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            },
            Some(id) = nudge.recv() => vec![id],
            _ = ticker.tick() => vec![],
        };
        let Some(inner) = weak.upgrade() else { break };
        let now = Instant::now();
        let cfg = inner.sync_config.clone();
        let mut run: Vec<EndpointId> = vec![];
        {
            let peers = inner.peers.lock().unwrap();
            if due.is_empty() {
                // Periodic pass: anything whose scheduled time has come.
                run.extend(
                    peers
                        .iter()
                        .filter(|(_, p)| !p.syncing && p.next_attempt <= now)
                        .map(|(id, _)| *id),
                );
            } else {
                // A connect or an explicit nudge: sync now, but not in a tight loop (connections
                // flap during every sync).
                for id in due {
                    if let Some(p) = peers.get(&id) {
                        let recent = p
                            .last_attempt
                            .is_some_and(|t| now.duration_since(t) < cfg.backoff_min);
                        if !p.syncing && !recent {
                            run.push(id);
                        }
                    }
                }
            }
        }
        for id in run {
            let inner = inner.clone();
            tokio::spawn(async move {
                let _ = sync_peer(&inner, id).await;
            });
        }
    }
}

/// One sync attempt with state bookkeeping and events. A concurrent attempt is a no-op.
pub(crate) async fn sync_peer<L: LibraryStore>(
    inner: &Arc<Inner<L>>,
    id: EndpointId,
) -> Result<()> {
    let cfg = inner.sync_config.clone();
    let addr = {
        let mut peers = inner.peers.lock().unwrap();
        let p = peers
            .get_mut(&id)
            .ok_or_else(|| anyhow!("unknown peer {id}"))?;
        if p.syncing {
            return Ok(());
        }
        p.syncing = true;
        p.last_attempt = Some(Instant::now());
        p.addr.clone()
    };
    let iroh_id = crate::addr::to_iroh_id(&id).map_err(|e| anyhow!("{e}"))?;
    inner.node.emit(NodeEvent::SyncProgress {
        peer: iroh_id,
        phase: SyncPhase::Started,
    });

    let result = match tokio::time::timeout(cfg.timeout, inner.store.sync_with(&addr)).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(anyhow!("{e}")),
        Err(_) => Err(anyhow!("sync timed out after {:?}", cfg.timeout)),
    };

    let mut retry_in = None;
    {
        let mut peers = inner.peers.lock().unwrap();
        if let Some(p) = peers.get_mut(&id) {
            p.syncing = false;
            match &result {
                Ok(()) => {
                    p.failures = 0;
                    p.next_attempt = Instant::now() + cfg.resync_interval;
                }
                Err(_) => {
                    p.failures += 1;
                    let delay = backoff(&cfg, p.failures);
                    p.next_attempt = Instant::now() + delay;
                    retry_in = Some(delay);
                }
            }
        }
    }
    match &result {
        Ok(()) => inner.node.emit(NodeEvent::SyncProgress {
            peer: iroh_id,
            phase: SyncPhase::Finished,
        }),
        Err(e) => {
            debug!("sync with {} failed: {e:#}", iroh_id.fmt_short());
            inner.node.emit(NodeEvent::SyncProgress {
                peer: iroh_id,
                phase: SyncPhase::Failed(e.to_string()),
            });
            if let Some(d) = retry_in {
                inner.node.emit(NodeEvent::SyncProgress {
                    peer: iroh_id,
                    phase: SyncPhase::Retrying {
                        in_ms: d.as_millis() as u64,
                    },
                });
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_and_is_capped() {
        let cfg = SyncConfig {
            backoff_min: Duration::from_secs(1),
            backoff_max: Duration::from_secs(10),
            ..Default::default()
        };
        let secs: Vec<u64> = (1..=6).map(|n| backoff(&cfg, n).as_secs()).collect();
        assert_eq!(secs, vec![1, 2, 4, 8, 10, 10]);
        assert_eq!(backoff(&cfg, 0).as_secs(), 1);
        assert_eq!(backoff(&cfg, 10_000).as_secs(), 10, "no overflow");
    }
}
