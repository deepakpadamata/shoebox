//! Rate limits: requests per minute per key (an address or a share) and bytes per day per share.

use std::collections::HashMap;
use std::sync::Mutex;

const MINUTE: i64 = 60_000;
const DAY: i64 = 86_400_000;
const SWEEP_AT: usize = 20_000;

#[derive(Default)]
pub struct Limiter {
    windows: Mutex<HashMap<String, (i64, u32)>>,
    daily: Mutex<HashMap<String, (i64, u64)>>,
}

impl Limiter {
    /// Count a request for `key`; false when it is over `per_minute`.
    pub fn hit(&self, key: &str, now_ms: i64, per_minute: u32) -> bool {
        let minute = now_ms.div_euclid(MINUTE);
        let mut w = self.windows.lock().unwrap_or_else(|e| e.into_inner());
        if w.len() > SWEEP_AT {
            w.retain(|_, (m, _)| *m == minute);
        }
        let e = w.entry(key.to_string()).or_insert((minute, 0));
        if e.0 != minute {
            *e = (minute, 0);
        }
        e.1 += 1;
        e.1 <= per_minute
    }

    /// Count `bytes` served for `key` today; false when that would pass `per_day` (nothing is counted then).
    pub fn charge(&self, key: &str, now_ms: i64, bytes: u64, per_day: u64) -> bool {
        let day = now_ms.div_euclid(DAY);
        let mut d = self.daily.lock().unwrap_or_else(|e| e.into_inner());
        if d.len() > SWEEP_AT {
            d.retain(|_, (x, _)| *x == day);
        }
        let e = d.entry(key.to_string()).or_insert((day, 0));
        if e.0 != day {
            *e = (day, 0);
        }
        if e.1.saturating_add(bytes) > per_day {
            return false;
        }
        e.1 += bytes;
        true
    }
}
