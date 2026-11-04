//! A counting global allocator shared by the bench binaries (M4-11).

use std::sync::atomic::{AtomicU64, Ordering};

/// Counts live heap bytes and their peak, so each stage's memory can be attributed (M4-11):
/// RSS alone mixes both replicas, freed-but-kept pages and the allocator's own slack.
struct Counting;

pub static LIVE: AtomicU64 = AtomicU64::new(0);
pub static PEAK: AtomicU64 = AtomicU64::new(0);

// SAFETY: forwards to the system allocator unchanged; only counts.
#[allow(unsafe_code)]
unsafe impl std::alloc::GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: std::alloc::Layout) -> *mut u8 {
        let p = unsafe { std::alloc::System.alloc(l) };
        if !p.is_null() {
            let now = LIVE.fetch_add(l.size() as u64, Ordering::Relaxed) + l.size() as u64;
            PEAK.fetch_max(now, Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: std::alloc::Layout) {
        unsafe { std::alloc::System.dealloc(p, l) };
        LIVE.fetch_sub(l.size() as u64, Ordering::Relaxed);
    }
    unsafe fn realloc(&self, p: *mut u8, l: std::alloc::Layout, new: usize) -> *mut u8 {
        let q = unsafe { std::alloc::System.realloc(p, l, new) };
        if !q.is_null() {
            if new >= l.size() {
                let now = LIVE.fetch_add((new - l.size()) as u64, Ordering::Relaxed)
                    + (new - l.size()) as u64;
                PEAK.fetch_max(now, Ordering::Relaxed);
            } else {
                LIVE.fetch_sub((l.size() - new) as u64, Ordering::Relaxed);
            }
        }
        q
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

pub fn live() -> u64 {
    LIVE.load(Ordering::Relaxed)
}

/// Restart peak tracking from the current live bytes.
pub fn reset_peak() {
    PEAK.store(live(), Ordering::Relaxed);
}
