//! Hybrid logical clock (HLC): physical milliseconds + logical counter + device-id tie-break.
//!
//! Total order: `(physical_ms, counter, device)`. Two distinct devices can never produce equal
//! HLCs, so every LWW comparison is decided deterministically on every replica.

use crate::ids::EndpointId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Hlc {
    pub physical_ms: u64,
    pub counter: u32,
    pub device: EndpointId,
}

/// Per-device clock that issues strictly increasing [`Hlc`]s.
#[derive(Clone, Debug)]
pub struct HlcClock {
    device: EndpointId,
    last_physical: u64,
    last_counter: u32,
}

impl HlcClock {
    pub fn new(device: EndpointId) -> Self {
        Self {
            device,
            last_physical: 0,
            last_counter: 0,
        }
    }
    /// Issue a timestamp for a local event at wall-clock `now_ms`.
    pub fn tick(&mut self, now_ms: u64) -> Hlc {
        let physical = now_ms.max(self.last_physical);
        let counter = if physical == self.last_physical {
            self.last_counter + 1
        } else {
            0
        };
        self.issue(physical, counter)
    }
    /// Merge a remote timestamp, then issue a timestamp that is after both.
    pub fn observe(&mut self, now_ms: u64, remote: &Hlc) -> Hlc {
        let physical = now_ms.max(self.last_physical).max(remote.physical_ms);
        let mut counter = 0;
        if physical == self.last_physical {
            counter = self.last_counter + 1;
        }
        if physical == remote.physical_ms {
            counter = counter.max(remote.counter + 1);
        }
        self.issue(physical, counter)
    }

    fn issue(&mut self, physical_ms: u64, counter: u32) -> Hlc {
        self.last_physical = physical_ms;
        self.last_counter = counter;
        Hlc {
            physical_ms,
            counter,
            device: self.device,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(b: u8) -> EndpointId {
        EndpointId([b; 32])
    }

    #[test]
    fn ordering_is_physical_then_counter_then_device() {
        let a = Hlc {
            physical_ms: 1,
            counter: 9,
            device: dev(9),
        };
        let b = Hlc {
            physical_ms: 2,
            counter: 0,
            device: dev(0),
        };
        assert!(a < b);
        let c = Hlc {
            physical_ms: 2,
            counter: 1,
            device: dev(0),
        };
        assert!(b < c);
        let d = Hlc {
            physical_ms: 2,
            counter: 1,
            device: dev(1),
        };
        assert!(c < d);
    }

    #[test]
    fn tick_advances_with_wall_clock() {
        let mut clock = HlcClock::new(dev(1));
        let a = clock.tick(100);
        let b = clock.tick(250);
        assert_eq!((a.physical_ms, a.counter), (100, 0));
        assert_eq!((b.physical_ms, b.counter), (250, 0));
        assert!(a < b);
    }

    #[test]
    fn tick_bumps_counter_when_wall_clock_does_not_advance() {
        let mut clock = HlcClock::new(dev(1));
        let a = clock.tick(100);
        let b = clock.tick(100);
        let c = clock.tick(50); // wall clock went backwards
        assert_eq!((b.physical_ms, b.counter), (100, 1));
        assert_eq!((c.physical_ms, c.counter), (100, 2));
        assert!(a < b && b < c);
    }

    #[test]
    fn observe_moves_clock_past_a_remote_ahead_of_us() {
        let mut clock = HlcClock::new(dev(1));
        clock.tick(100);
        let remote = Hlc {
            physical_ms: 500,
            counter: 3,
            device: dev(2),
        };
        let after = clock.observe(120, &remote);
        assert!(after > remote);
        assert_eq!((after.physical_ms, after.counter), (500, 4));
        assert_eq!(after.device, dev(1));
        // and later local ticks stay after it
        assert!(clock.tick(130) > after);
    }

    #[test]
    fn observe_with_equal_physical_takes_max_counter_plus_one() {
        let mut clock = HlcClock::new(dev(1));
        clock.tick(100);
        clock.tick(100); // counter 1
        let remote = Hlc {
            physical_ms: 100,
            counter: 7,
            device: dev(2),
        };
        let after = clock.observe(100, &remote);
        assert_eq!((after.physical_ms, after.counter), (100, 8));
    }

    #[test]
    fn observe_with_old_remote_behaves_like_tick() {
        let mut clock = HlcClock::new(dev(1));
        let a = clock.tick(1000);
        let remote = Hlc {
            physical_ms: 10,
            counter: 0,
            device: dev(2),
        };
        let after = clock.observe(1000, &remote);
        assert!(after > a);
        assert_eq!((after.physical_ms, after.counter), (1000, 1));
    }
}
