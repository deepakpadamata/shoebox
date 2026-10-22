//! "Is this node busy?" tracking, used to keep blob garbage collection out of the way.
//!
//! iroh-blobs' GC snapshots what is referenced and then sweeps; blobs created or referenced in
//! between can be deleted (we reproduced this with entry content written during a GC run). Rather
//! than try to tag every in-flight blob, the node simply skips GC passes while it has been busy
//! (importing, fetching, syncing, writing library entries) within the last `quiet` period.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Cheap, cloneable "last time something happened" marker.
#[derive(Clone, Debug, Default)]
pub struct Activity(Arc<AtomicU64>);

impl Activity {
    /// Record that the node is doing real work right now.
    pub fn touch(&self) {
        self.0.store(now_ms(), Ordering::Relaxed);
    }

    /// Whether anything happened within the last `quiet`.
    pub fn busy_within(&self, quiet: Duration) -> bool {
        let last = self.0.load(Ordering::Relaxed);
        last != 0 && now_ms().saturating_sub(last) < quiet.as_millis() as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn busy_only_shortly_after_a_touch() {
        let a = Activity::default();
        assert!(
            !a.busy_within(Duration::from_secs(60)),
            "never touched = idle"
        );
        a.touch();
        assert!(a.busy_within(Duration::from_secs(60)));
        std::thread::sleep(Duration::from_millis(60));
        assert!(!a.busy_within(Duration::from_millis(20)));
        let clone = a.clone();
        clone.touch();
        assert!(a.busy_within(Duration::from_secs(60)), "clones share state");
    }
}
