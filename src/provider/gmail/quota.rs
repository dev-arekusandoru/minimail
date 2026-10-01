//! Client-side token bucket that keeps Gmail usage inside its own quota.
//!
//! Gmail charges per request (`messages.get` is the expensive one) against a
//! per-user allowance of 15 000 units per minute. The bucket spends a fraction
//! of that allowance so a long backfill never trips the server limit, and the
//! provider empties it whenever the server *does* complain.

use std::time::{Duration, Instant};

/// Units the provider is willing to spend per second (9 000 per minute,
/// well under Gmail's 15 000 per-minute allowance).
pub const UNITS_PER_SEC: u32 = 150;
/// Bucket size: enough to absorb a burst of batches before throttling itself.
pub const BURST: u32 = 250;

/// A token bucket of Gmail quota units.
pub struct Quota {
    rate: f64,
    burst: f64,
    tokens: f64,
    last: Instant,
}

impl Quota {
    pub fn new(units_per_sec: u32, burst: u32) -> Self {
        let burst = burst.max(1) as f64;
        Self {
            rate: units_per_sec.max(1) as f64,
            burst,
            tokens: burst,
            last: Instant::now(),
        }
    }

    fn refill(&mut self, now: Instant) {
        let elapsed = now.saturating_duration_since(self.last).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.rate).min(self.burst);
        self.last = now;
    }

    /// Books `units` of budget and returns how long the caller must wait before
    /// sending them. The caller is expected to sleep for that long on its own
    /// (background) thread. A zero return means the burst covers the request.
    pub fn reserve(&mut self, units: u32, now: Instant) -> Duration {
        self.refill(now);
        self.tokens -= units as f64;
        if self.tokens >= 0.0 {
            Duration::ZERO
        } else {
            Duration::from_secs_f64(-self.tokens / self.rate)
        }
    }

    /// Empties the bucket: the server said we were going too fast, so the next
    /// reservation has to wait for the bucket to refill.
    pub fn drain(&mut self, now: Instant) {
        self.refill(now);
        self.tokens = 0.0;
    }
}
