//! The "don't auto-classify old mail" cutoff. Pure: the caller passes `now`.

use crate::clock::{DAY, Timestamp};

/// The choices the setting offers, in days; `0` means never skip.
pub const AGE_CHOICES: [(u16, &str); 6] = [
    (0, "Never skip"),
    (1, "1 day"),
    (7, "1 week"),
    (30, "1 month"),
    (90, "3 months"),
    (365, "1 year"),
];

/// Shipped default: a new user with a big mailbox must not pay to classify its history.
pub const DEFAULT_MAX_AGE_DAYS: u16 = 7;

/// Whether mail received at `received` is too old to classify automatically under a cutoff of
/// `max_age_days` (`0` = off). Mail exactly at the cutoff is still classified. An unknown
/// received time is never skipped.
pub fn too_old_for_auto(received: Option<Timestamp>, now: Timestamp, max_age_days: u16) -> bool {
    max_age_days != 0
        && received.is_some_and(|at| now - at > Timestamp::from(max_age_days) * DAY)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: Timestamp = 1_000_000_000;

    #[test]
    fn cutoff_boundary() {
        let cutoff = 7 * DAY;
        assert!(!too_old_for_auto(Some(NOW - cutoff), NOW, 7), "exactly at the cutoff stays");
        assert!(!too_old_for_auto(Some(NOW - cutoff + 1), NOW, 7), "just inside");
        assert!(too_old_for_auto(Some(NOW - cutoff - 1), NOW, 7), "just outside");
    }

    #[test]
    fn off_and_unknown_never_skip() {
        assert!(!too_old_for_auto(Some(0), NOW, 0));
        assert!(!too_old_for_auto(None, NOW, 7));
    }

    #[test]
    fn future_mail_is_fresh() {
        assert!(!too_old_for_auto(Some(NOW + DAY), NOW, 1));
    }
}
