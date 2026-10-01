//! Date operands for `before:` / `after:` / `on:`: absolute `yyyy-mm-dd` or relative `<n><d|w|m|y>`.
//!
//! Both resolve to a UTC calendar day (`yyyy-mm-dd`) so comparisons are plain string compares
//! against the first ten characters of a message's RFC 3339 `received`.

use crate::clock::{DAY, Timestamp};
use crate::model::format_rfc3339;

/// `yyyy-mm-dd` with a plausible month/day.
pub fn valid_absolute(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let (Ok(year), Ok(month), Ok(day)) = (
        s[..4].parse::<u32>(),
        s[5..7].parse::<u32>(),
        s[8..10].parse::<u32>(),
    ) else {
        return false;
    };
    s.bytes().filter(|c| *c != b'-').all(|c| c.is_ascii_digit())
        && year > 0
        && (1..=12).contains(&month)
        && (1..=31).contains(&day)
}

/// `(amount, unit)` for `7d`, `2w`, `3m`, `1y` (1-4 digits, unit lowercase).
pub fn parse_relative(s: &str) -> Option<(u32, char)> {
    let unit = s.chars().last()?;
    if !matches!(unit, 'd' | 'w' | 'm' | 'y') {
        return None;
    }
    let digits = &s[..s.len() - 1];
    if digits.is_empty() || digits.len() > 4 || !digits.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some((digits.parse().ok()?, unit))
}

pub fn is_valid(s: &str) -> bool {
    valid_absolute(s) || parse_relative(s).is_some()
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// The calendar day (`yyyy-mm-dd`, UTC) `value` denotes when "today" is `now`.
/// `None` if `value` is not a valid date operand. Months and years are calendar
/// months/years (day clamped to the target month's length).
pub fn resolve(value: &str, now: Timestamp) -> Option<String> {
    if valid_absolute(value) {
        return Some(value.to_owned());
    }
    let (n, unit) = parse_relative(value)?;
    let n = i64::from(n);
    match unit {
        'd' => Some(day_string(now - n * DAY)),
        'w' => Some(day_string(now - n * 7 * DAY)),
        _ => {
            let today = day_string(now);
            let year: i64 = today[..4].parse().ok()?;
            let month: i64 = today[5..7].parse().ok()?;
            let day: i64 = today[8..10].parse().ok()?;
            let months = year * 12 + (month - 1) - if unit == 'y' { n * 12 } else { n };
            if months < 12 {
                return Some("0001-01-01".into());
            }
            let (year, month) = (months / 12, months % 12 + 1);
            let day = day.min(days_in_month(year, month));
            Some(format!("{year:04}-{month:02}-{day:02}"))
        }
    }
}

fn day_string(ts: Timestamp) -> String {
    format_rfc3339(ts)[..10].to_owned()
}
