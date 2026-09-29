//! Injectable time source. All time-based behavior (snooze wake-up, Waiting
//! resurfacing, outbox flush, session timing) reads `Clock::now`, so tests can
//! drive it deterministically with `FakeClock`.

use std::cell::Cell;
use std::time::{SystemTime, UNIX_EPOCH};

/// Whole seconds since the Unix epoch (UTC).
pub type Timestamp = i64;

pub const MINUTE: Timestamp = 60;
pub const HOUR: Timestamp = 60 * MINUTE;
pub const DAY: Timestamp = 24 * HOUR;

pub trait Clock {
    fn now(&self) -> Timestamp;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as Timestamp)
    }
}

/// Manually advanced clock for tests.
pub struct FakeClock(Cell<Timestamp>);

impl FakeClock {
    pub fn new(now: Timestamp) -> Self {
        Self(Cell::new(now))
    }

    pub fn set(&self, now: Timestamp) {
        self.0.set(now);
    }

    pub fn advance(&self, secs: Timestamp) {
        self.0.set(self.0.get() + secs);
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Timestamp {
        self.0.get()
    }
}
