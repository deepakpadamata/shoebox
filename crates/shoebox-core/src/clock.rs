//! Wall-clock abstraction so HLC issuance is deterministic in tests.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

pub trait Clock: Send + Sync + 'static {
    /// Milliseconds since the Unix epoch.
    fn now_ms(&self) -> u64;
}

/// The real system clock.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}

/// A clock that only moves when told to. Clones share the same time.
#[derive(Clone, Debug, Default)]
pub struct ManualClock(Arc<AtomicU64>);

impl ManualClock {
    pub fn new(start_ms: u64) -> Self {
        Self(Arc::new(AtomicU64::new(start_ms)))
    }
    pub fn set(&self, ms: u64) {
        self.0.store(ms, Ordering::SeqCst);
    }
    pub fn advance(&self, ms: u64) {
        self.0.fetch_add(ms, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_clock_is_shared_between_clones_and_moves_only_when_told() {
        let a = ManualClock::new(10);
        let b = a.clone();
        assert_eq!(b.now_ms(), 10);
        a.advance(5);
        assert_eq!(b.now_ms(), 15);
        b.set(3);
        assert_eq!(a.now_ms(), 3);
    }

    #[test]
    fn system_clock_is_after_2020() {
        assert!(SystemClock.now_ms() > 1_577_836_800_000);
    }
}
