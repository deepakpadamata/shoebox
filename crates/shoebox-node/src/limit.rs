//! Failure limits for the handshakes open to devices outside the library (pairing, recovery).
//!
//! Counted per remote device, not globally: the secrets these handshakes check cannot be guessed,
//! so a limit only caps wasted work, and a global one would let any stranger lock everybody out by
//! failing on purpose. A stranger can make new keys to get around a per-device limit, which gains
//! nothing against an unguessable secret.

use iroh::EndpointId;
use std::collections::{HashMap, VecDeque};

const WINDOW_MS: u64 = 60_000;
/// Above this many tracked devices, the ones with no recent failure are forgotten.
const PRUNE_AT: usize = 4_096;

#[derive(Default)]
pub(crate) struct FailureLimit {
    failed: HashMap<EndpointId, VecDeque<u64>>,
}

impl FailureLimit {
    /// Whether `peer` is still under `max` failures in the last minute.
    pub(crate) fn allows(&mut self, peer: &EndpointId, now: u64, max: usize) -> bool {
        if self.failed.len() > PRUNE_AT {
            self.failed.retain(|_, times| {
                Self::forget_old(times, now);
                !times.is_empty()
            });
        }
        match self.failed.get_mut(peer) {
            Some(times) => {
                Self::forget_old(times, now);
                times.len() < max
            }
            None => true,
        }
    }

    pub(crate) fn record(&mut self, peer: EndpointId, now: u64) {
        self.failed.entry(peer).or_default().push_back(now);
    }

    fn forget_old(times: &mut VecDeque<u64>, now: u64) {
        while times
            .front()
            .is_some_and(|t| now.saturating_sub(*t) > WINDOW_MS)
        {
            times.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(b: u8) -> EndpointId {
        iroh::SecretKey::from_bytes(&[b; 32]).public()
    }

    #[test]
    fn one_device_failing_does_not_lock_out_another() {
        let mut l = FailureLimit::default();
        for _ in 0..3 {
            assert!(l.allows(&peer(1), 1_000, 3));
            l.record(peer(1), 1_000);
        }
        assert!(!l.allows(&peer(1), 1_000, 3), "the failing device waits");
        assert!(l.allows(&peer(2), 1_000, 3), "others are not affected");
        assert!(
            l.allows(&peer(1), 62_000, 3),
            "a minute later it may try again"
        );
    }

    #[test]
    fn many_strangers_do_not_grow_the_table_forever() {
        let mut l = FailureLimit::default();
        for i in 0..=PRUNE_AT as u32 {
            let mut b = [0u8; 32];
            b[..4].copy_from_slice(&i.to_le_bytes());
            l.record(iroh::SecretKey::from_bytes(&b).public(), 0);
        }
        l.allows(&peer(1), 120_000, 3);
        assert!(l.failed.is_empty());
    }
}
