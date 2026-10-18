//! Day grouping: day keys, header labels, runs and the header/row interleaving.

use std::collections::HashSet;
use std::rc::Rc;

use mail_classifier::clock::{DAY, Timestamp};
use mail_classifier::day_groups::{Day, Item, day_key, days, items, label};
use mail_classifier::tz::{Now, Utc};

/// 2026-09-29 12:00 UTC.
const NOON: Timestamp = 1_790_683_200;

fn now() -> Now {
    Now::new(NOON, Rc::new(Utc))
}

#[test]
fn labels_today_yesterday_and_dates() {
    let n = now();
    assert_eq!(label(&n, NOON - 11 * 3600), "Today");
    assert_eq!(label(&n, NOON - 13 * 3600), "Yesterday");
    assert_eq!(label(&n, NOON - 10 * DAY), "Sep 19");
    assert_eq!(label(&n, NOON - 400 * DAY), "Aug 25, 2025");
}

#[test]
fn a_day_is_the_local_calendar_day() {
    let n = now();
    assert_eq!(day_key(&n, NOON), day_key(&n, NOON - 11 * 3600));
    assert_ne!(day_key(&n, NOON), day_key(&n, NOON - 13 * 3600));
}

#[test]
fn runs_start_a_header_whenever_the_day_changes() {
    let n = now();
    let stamps = [NOON, NOON - 3600, NOON - DAY, NOON - 2 * DAY, NOON - 2 * DAY - 60, NOON];
    let runs: Vec<usize> = days(&n, &stamps).iter().map(|d| d.count).collect();
    assert_eq!(runs, vec![2, 1, 2, 1], "an out-of-order day gets its own header");
}

fn day(key: i64) -> Day {
    Day { key, ts: 0, count: 1 }
}

#[test]
fn items_interleave_headers_and_rows() {
    let d = [day(3), day(2), day(1)];
    let got = items(&d, &[3, 3, 2, 1], &HashSet::new());
    assert_eq!(
        got,
        vec![
            Item::Header(0),
            Item::Row(0),
            Item::Row(1),
            Item::Header(1),
            Item::Row(2),
            Item::Header(2),
            Item::Row(3),
        ]
    );
}

#[test]
fn a_collapsed_day_keeps_its_header_and_drops_its_rows() {
    let d = [day(3), day(2), day(1)];
    // Row space already excludes the collapsed day's messages.
    let got = items(&d, &[3, 1], &HashSet::from([2]));
    assert_eq!(
        got,
        vec![Item::Header(0), Item::Row(0), Item::Header(1), Item::Header(2), Item::Row(1)]
    );
}

#[test]
fn a_day_without_rows_that_is_not_collapsed_has_no_header() {
    let d = [day(3), day(2)];
    assert_eq!(items(&d, &[3], &HashSet::new()), vec![Item::Header(0), Item::Row(0)]);
}
