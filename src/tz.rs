//! The user's wall clock. Instants are whole seconds since the Unix epoch (UTC); everything
//! a person reads — "today", a row's `09:10`, the day an `after:` filter compares — is that
//! instant in *their* zone, so the same [`Now`] carries both the instant and the zone and
//! nothing downstream reads a zone of its own.

use std::rc::Rc;

use chrono::{DateTime, Local, TimeZone as _};

use crate::clock::{DAY, Timestamp};

/// Seconds east of UTC in effect at `ts`, DST transitions included.
pub trait Zone {
    fn offset_at(&self, ts: Timestamp) -> i32;
}

/// UTC: no offset, ever.
pub struct Utc;

impl Zone for Utc {
    fn offset_at(&self, _: Timestamp) -> i32 {
        0
    }
}

/// The platform's local zone, read at call time so a zone change (travel, a settings edit)
/// shows up without a restart.
pub struct SystemZone;

impl Zone for SystemZone {
    fn offset_at(&self, ts: Timestamp) -> i32 {
        match DateTime::from_timestamp(ts, 0) {
            Some(utc) => Local.offset_from_utc_datetime(&utc.naive_utc()).local_minus_utc(),
            // Out of chrono's range: fall back to UTC rather than inventing an offset.
            None => 0,
        }
    }
}

/// A constant offset east of UTC, in seconds.
pub struct FixedZone(pub i32);

impl Zone for FixedZone {
    fn offset_at(&self, _: Timestamp) -> i32 {
        self.0
    }
}

/// An instant together with the zone it is read in. Date-relative logic takes this instead
/// of a bare `Timestamp`; duration arithmetic (snoozing, retry backoff, session length)
/// keeps taking [`Timestamp`], because an offset never changes how long something is.
#[derive(Clone)]
pub struct Now {
    at: Timestamp,
    zone: Rc<dyn Zone>,
}

impl Now {
    pub fn new(at: Timestamp, zone: Rc<dyn Zone>) -> Self {
        Self { at, zone }
    }

    /// The instant itself, for arithmetic no zone touches.
    pub fn at(&self) -> Timestamp {
        self.at
    }

    pub fn zone(&self) -> &dyn Zone {
        &*self.zone
    }

    /// An instant as `(days since the epoch, seconds into that day)` on the wall clock.
    /// The offset is read at the instant, so a message from before a DST change reads on
    /// the clock it was actually sent at.
    pub fn parts(&self, ts: Timestamp) -> (i64, i64) {
        let local = ts + i64::from(self.zone.offset_at(ts));
        (local.div_euclid(DAY), local.rem_euclid(DAY))
    }

    /// The wall clock's seconds since the epoch for `ts`: the `(days, secs)` of
    /// [`Now::parts`] as one number, which is what a "tonight at 18:00" target is written as.
    pub fn local_secs(&self, ts: Timestamp) -> i64 {
        let (days, secs) = self.parts(ts);
        days * DAY + secs
    }

    /// The instant whose wall clock reads `local_secs` (seconds since the epoch, read as
    /// local time). The inverse of [`Now::local_secs`], with one refinement pass so a wall
    /// time on the far side of a DST change lands on the instant that actually reads it.
    pub fn instant(&self, local_secs: i64) -> Timestamp {
        let guess = local_secs - i64::from(self.zone.offset_at(local_secs));
        local_secs - i64::from(self.zone.offset_at(guess))
    }

    /// Days to add to the local midnight of `ts`'s day to reach the next local `weekday`
    /// (`0` = Sunday) at `secs` into the day. Today counts only while it is still ahead.
    pub fn days_until_weekday(&self, ts: Timestamp, weekday: usize, secs: i64) -> i64 {
        let (day, now_secs) = self.parts(ts);
        let today = Now::weekday(day) as i64 == weekday as i64;
        let ahead = if today && now_secs >= secs {
            7
        } else {
            (weekday as i64 - Now::weekday(day) as i64).rem_euclid(7)
        };
        ahead * DAY
    }

    /// Civil `(year, month, day)` of an instant on the wall clock.
    pub fn civil(&self, ts: Timestamp) -> (i64, u32, u32) {
        civil_from_days(self.parts(ts).0)
    }

    /// `yyyy-mm-dd` of an instant on the wall clock: the calendar day a date filter compares.
    pub fn day(&self, ts: Timestamp) -> String {
        let (year, month, day) = self.civil(ts);
        format_ymd(year, month, day)
    }

    /// `HH:MM` of an instant on the wall clock.
    pub fn hhmm(&self, ts: Timestamp) -> String {
        let secs = self.parts(ts).1;
        format!("{:02}:{:02}", secs / 3600, secs % 3600 / 60)
    }

    /// Weekday of a day count (days since the epoch), 0 = Sunday.
    pub fn weekday(days: i64) -> usize {
        (days + 4).rem_euclid(7) as usize
    }
}

pub const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

pub const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// `yyyy-mm-dd`.
pub fn format_ymd(year: i64, month: u32, day: u32) -> String {
    format!("{year:04}-{month:02}-{day:02}")
}

/// Civil `(year, month, day)` for a count of days since 1970-01-01 (Howard Hinnant's
/// `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(at: Timestamp) -> Now {
        Now::new(at, Rc::new(Utc))
    }

    fn fixed(at: Timestamp, offset: i32) -> Now {
        Now::new(at, Rc::new(FixedZone(offset)))
    }

    #[test]
    fn utc_reads_as_it_is_written() {
        let n = utc(1_791_191_400); // 2026-10-05 09:10 UTC, a Monday
        assert_eq!(n.day(1_791_191_400), "2026-10-05");
        assert_eq!(n.hhmm(1_791_191_400), "09:10");
        assert_eq!(n.civil(1_791_191_400), (2026, 10, 5));
        assert_eq!(Now::weekday(n.parts(1_791_191_400).0), 1, "Monday");
    }

    #[test]
    fn a_western_offset_moves_the_label_and_can_change_the_day() {
        let n = fixed(0, -7 * 3600);
        assert_eq!(n.hhmm(0), "17:00");
        assert_eq!(n.day(0), "1969-12-31");
        let east = fixed(0, 5 * 3600 + 1800);
        assert_eq!(east.hhmm(0), "05:30");
        assert_eq!(east.day(0), "1970-01-01");
    }

    #[test]
    fn each_instant_reads_at_the_offset_in_force_then() {
        // A zone whose offset moves by two hours at midnight: before the move the wall
        // clock is an hour ahead of UTC, after it an hour behind.
        struct Stepping;
        impl Zone for Stepping {
            fn offset_at(&self, ts: Timestamp) -> i32 {
                if ts < 86_400 { 3600 } else { -3600 }
            }
        }
        let n = Now::new(0, Rc::new(Stepping));
        // Two instants two hours apart can land on either side of the local midnight.
        assert_eq!(n.parts(86_340), (1, 59 * 60));
        assert_eq!(n.parts(86_460), (0, 23 * 3600 + 60));
    }

    #[test]
    fn civil_dates_are_exact_across_a_leap_day() {
        let n = utc(0);
        assert_eq!(n.civil(0), (1970, 1, 1));
        assert_eq!(n.civil(-1), (1969, 12, 31));
        assert_eq!(n.civil(19_782 * DAY), (2024, 2, 29));
        assert_eq!(n.civil(20_731 * DAY), (2026, 10, 5));
    }
}
