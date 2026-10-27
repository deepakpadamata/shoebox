//! `sync_burst` (M4-08/09): what a background wake-up does in the little time the OS grants.
//! Sync with the known devices (the keeper first), then stay reachable while they pull
//! whatever is at risk, and stop cleanly before the deadline. The outcome feeds the
//! notification rules and diagnostics.

use crate::core::AppCore;
use crate::types::BurstReport;
use anyhow::Result;
use shoebox_core::store::{LibraryStore, Page};
use std::sync::Arc;
use std::time::{Duration, Instant};

const PEER_TIMEOUT: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(500);
/// Pause before trying a device again after it refused.
const RETRY: Duration = Duration::from_millis(500);
/// Give up early when nothing improved for this long (the other devices are not pulling).
const STALL: Duration = Duration::from_secs(20);

impl AppCore {
    /// Sync and keep going until nothing is at risk, `budget` runs out, or progress stalls.
    pub async fn sync_burst(self: &Arc<Self>, budget: Duration) -> Result<BurstReport> {
        let started = Instant::now();
        let deadline = started + budget;
        let ln = self.ln().await?;
        tracing::info!(
            after_ms = started.elapsed().as_millis() as u64,
            "burst: library open"
        );
        let at_risk_before = ln.at_risk(Page::all()).await?.len() as u32;

        // Keepers first: they are the likeliest to take the photos off this device.
        let devices = ln.store().device_list().await?;
        // Members this device was never given an address for (it invited them, they dialled in)
        // are looked up by id; the node remembers how they reached it.
        for d in devices
            .devices
            .iter()
            .filter(|d| !d.revoked && d.device != ln.device())
        {
            if !ln.peers().contains(&d.device) {
                let _ = ln.add_peer(shoebox_core::store::PeerAddr::id_only(d.device));
            }
        }
        let mut peers = ln.peers();
        peers.sort_by_key(|p| {
            !devices
                .devices
                .iter()
                .any(|d| d.device == *p && d.role == shoebox_core::model::Role::Keeper)
        });
        let (mut reached, total) = (0u32, peers.len() as u32);
        for p in peers {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            // A quick refusal is often momentary (a device paired a moment ago, a sync the
            // background scheduler started that just failed): keep trying within the time this
            // device gets, instead of giving up on the first answer.
            let give_up = Instant::now() + PEER_TIMEOUT.min(left);
            loop {
                let left = give_up.saturating_duration_since(Instant::now());
                match tokio::time::timeout(left, ln.sync_now(p)).await {
                    Ok(Ok(())) => {
                        reached += 1;
                        break;
                    }
                    Ok(Err(e)) if Instant::now() + RETRY < give_up => {
                        tracing::debug!(peer = ?p, "burst: sync failed, trying again: {e:#}");
                        tokio::time::sleep(RETRY).await;
                    }
                    Ok(Err(e)) => {
                        tracing::warn!(peer = ?p, "burst: sync failed: {e:#}");
                        break;
                    }
                    Err(_) => {
                        tracing::warn!(peer = ?p, "burst: sync timed out");
                        break;
                    }
                }
            }
        }

        let mut at_risk = ln.at_risk(Page::all()).await?.len() as u32;
        let mut best = at_risk;
        let mut last_progress = Instant::now();
        let mut reason = None;
        if total == 0 {
            reason = Some("no other devices are paired yet".to_string());
        } else if reached == 0 {
            reason = Some("none of your devices could be reached".to_string());
        } else {
            while at_risk > 0 {
                if Instant::now() >= deadline {
                    reason = Some("ran out of background time".to_string());
                    break;
                }
                if last_progress.elapsed() > STALL {
                    reason = Some("the other devices are not taking the photos".to_string());
                    break;
                }
                tokio::time::sleep(POLL).await;
                at_risk = ln.at_risk(Page::all()).await?.len() as u32;
                if at_risk < best {
                    best = at_risk;
                    last_progress = Instant::now();
                }
            }
        }
        Ok(BurstReport {
            peers_total: total,
            peers_reached: reached,
            at_risk_before,
            at_risk_after: at_risk,
            elapsed_ms: started.elapsed().as_millis() as f64,
            finished: at_risk == 0,
            budget_exhausted: reason.as_deref() == Some("ran out of background time"),
            reason,
        })
    }
}
