//! Pure day grouping for the message list: which local calendar day a message falls on, the
//! header label for that day, and how day headers interleave with the list's rows.
//!
//! Everything is resolved against a [`Now`] (instant + zone); no system clock is read here.

use std::collections::HashSet;

use crate::clock::Timestamp;
use crate::tz::{MONTHS, Now};

/// A local calendar day: days since the epoch on the wall clock.
pub type DayKey = i64;

/// The local day `ts` falls on.
pub fn day_key(now: &Now, ts: Timestamp) -> DayKey {
    now.parts(ts).0
}

/// Header text for the day of `ts`: `Today`, `Yesterday`, `Oct 13`, or `Oct 13, 2025` when not
/// in the current year.
pub fn label(now: &Now, ts: Timestamp) -> String {
    let today = day_key(now, now.at());
    match day_key(now, ts) {
        d if d == today => "Today".to_owned(),
        d if d == today - 1 => "Yesterday".to_owned(),
        _ => {
            let (year, month, day) = now.civil(ts);
            let name = MONTHS[(month - 1) as usize];
            if year == now.civil(now.at()).0 {
                format!("{name} {day}")
            } else {
                format!("{name} {day}, {year}")
            }
        }
    }
}

/// One day header: a run of consecutive messages that fall on the same day.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Day {
    pub key: DayKey,
    /// An instant inside the day (the run's first message), for [`label`].
    pub ts: Timestamp,
    /// Messages in the run.
    pub count: usize,
}

/// Day runs of `stamps` in their given order; a new run starts every time the day changes.
pub fn days(now: &Now, stamps: &[Timestamp]) -> Vec<Day> {
    let mut out: Vec<Day> = Vec::new();
    for &ts in stamps {
        let key = day_key(now, ts);
        match out.last_mut() {
            Some(d) if d.key == key => d.count += 1,
            _ => out.push(Day { key, ts, count: 1 }),
        }
    }
    out
}

/// One list item: a day header (index into the days) or a row (index into the rows).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Header(usize),
    Row(usize),
}

/// Interleave headers with rows. `row_days` is the day of each shown row, in order (a thread's
/// children take their header's day). A collapsed day keeps its header and shows no rows; a
/// day with no shown rows and not collapsed gets no header.
pub fn items(days: &[Day], row_days: &[DayKey], collapsed: &HashSet<DayKey>) -> Vec<Item> {
    let mut out = Vec::new();
    let mut next = 0;
    for (i, day) in days.iter().enumerate() {
        let start = next;
        while next < row_days.len() && row_days[next] == day.key {
            next += 1;
        }
        let hidden = collapsed.contains(&day.key);
        if hidden || next > start {
            out.push(Item::Header(i));
        }
        if !hidden {
            out.extend((start..next).map(Item::Row));
        }
    }
    // Rows that matched no day run keep their place at the end rather than vanishing.
    out.extend((next..row_days.len()).map(Item::Row));
    out
}
