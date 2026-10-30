//! Reading a typed send-later time: `14:30`, `tomorrow 9:00`, `2026-11-03 08:15`.
//!
//! Wall-clock text in, an instant out, against the caller's [`Now`]; nothing here reads a
//! clock, so the menu and its tests agree on what "past" means.

use chrono::{NaiveDate, NaiveTime};

use crate::clock::Timestamp;
use crate::tz::Now;

/// Why a typed send-later time was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BadTime {
    /// Nothing in the text reads as a time, a day, or both.
    Unreadable,
    /// Readable, but not ahead of now.
    Past,
}

/// Parse a typed send-later time into the instant it names.
///
/// The day defaults to today; the time is required. `today`/`tomorrow` and a
/// `yyyy-mm-dd` date both override it, the later of the two winning.
pub fn parse_send_later(input: &str, now: &Now) -> Result<Timestamp, BadTime> {
    let mut date = today(now);
    let mut time = None;
    for word in input.split_whitespace() {
        let word = word.to_ascii_lowercase();
        if let Ok(day) = NaiveDate::parse_from_str(&word, "%Y-%m-%d") {
            date = day;
        } else if let Ok(day) = NaiveDate::parse_from_str(&word, "%Y/%m/%d") {
            date = day;
        } else if word == "today" {
            date = today(now);
        } else if word == "tomorrow" {
            date = today(now).succ_opt().ok_or(BadTime::Unreadable)?;
        } else if let Ok(at) = NaiveTime::parse_from_str(&word, "%H:%M") {
            time = Some(at);
        } else {
            return Err(BadTime::Unreadable);
        }
    }
    let time = time.ok_or(BadTime::Unreadable)?;
    // The calendar fields read as local wall clock, which `Now::instant` turns back into
    // the instant that actually shows them.
    let wall = date.and_time(time).and_utc().timestamp();
    let at = now.instant(wall);
    if at <= now.at() {
        return Err(BadTime::Past);
    }
    Ok(at)
}

fn today(now: &Now) -> NaiveDate {
    let (year, month, day) = now.civil(now.at());
    NaiveDate::from_ymd_opt(year as i32, month, day).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use super::*;
    use crate::tz::Utc;

    /// 2026-09-29 12:00 UTC, the instant the UI tests' fake clock starts at.
    const NOON: Timestamp = 1_790_683_200;

    fn noon() -> Now {
        Now::new(NOON, Rc::new(Utc))
    }

    #[test]
    fn a_bare_time_lands_on_today() {
        assert_eq!(
            parse_send_later("14:30", &noon()),
            Ok(NOON + 2 * 3600 + 30 * 60)
        );
        assert_eq!(parse_send_later(" 9:05 ", &noon()), Err(BadTime::Past));
    }

    #[test]
    fn tomorrow_and_a_date_name_their_own_day() {
        let tomorrow = NOON + 21 * 3600 + 5 * 60;
        assert_eq!(parse_send_later("tomorrow 9:05", &noon()), Ok(tomorrow));
        assert_eq!(
            parse_send_later("2026-10-01 08:15", &noon()),
            Ok(NOON + 2 * 86400 - 3 * 3600 - 45 * 60)
        );
        assert_eq!(
            parse_send_later("today 14:30", &noon()),
            Ok(NOON + 2 * 3600 + 30 * 60)
        );
    }

    #[test]
    fn a_time_at_or_before_now_is_past() {
        assert_eq!(parse_send_later("12:00", &noon()), Err(BadTime::Past));
        assert_eq!(parse_send_later("11:59", &noon()), Err(BadTime::Past));
        assert_eq!(
            parse_send_later("2026-09-28 09:00", &noon()),
            Err(BadTime::Past)
        );
    }

    #[test]
    fn anything_else_is_unreadable() {
        assert_eq!(parse_send_later("", &noon()), Err(BadTime::Unreadable));
        assert_eq!(parse_send_later("later", &noon()), Err(BadTime::Unreadable));
        assert_eq!(
            parse_send_later("14:30 tonight", &noon()),
            Err(BadTime::Unreadable)
        );
        assert_eq!(
            parse_send_later("2026-02-30 08:00", &noon()),
            Err(BadTime::Unreadable)
        );
        assert_eq!(parse_send_later("25:00", &noon()), Err(BadTime::Unreadable));
    }
}
