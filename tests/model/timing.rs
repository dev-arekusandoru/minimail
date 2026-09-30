use mail_classifier::clock::{DAY, HOUR};
use mail_classifier::model::Tag;
use crate::helpers::{sample, threaded, snapshot, T0};
use crate::helpers::State;

#[test]
fn set_state_at_stamps_waiting_and_clears_it_on_leave() {
    let mut mb = sample();
    assert_eq!(mb.set_state_at(&[1], State::Waiting, 500), 1);
    assert_eq!(mb.waiting_since(1), Some(500));
    mb.set_state(&[2], State::Waiting);
    assert_eq!(mb.waiting_since(2), None, "plain set_state records no time");
    mb.set_state_at(&[1], State::Done, 600);
    assert_eq!(mb.waiting_since(1), None);
    assert!(mb.undo());
    assert_eq!(mb.waiting_since(1), Some(500));
    assert!(mb.undo() && mb.undo());
    assert_eq!(mb.waiting_since(1), None);
    assert_eq!(mb.state_of(1), Some(State::Inbox));
}

#[test]
fn waiting_resurfaces_after_three_days_and_tick_is_idempotent() {
    let mut mb = threaded(&[(1, 1, "a@x.test", "2026-09-01T00:00:00Z", State::Inbox)]);
    mb.set_state_at(&[1], State::Waiting, T0);
    let undo_depth_probe = snapshot(&mb);

    let r = mb.tick(T0 + 3 * DAY - 1);
    assert!(r.resurfaced.is_empty());
    assert_eq!(mb.state_of(1), Some(State::Waiting));

    let r = mb.tick(T0 + 3 * DAY);
    assert_eq!(r.resurfaced, vec![1]);
    assert_eq!(mb.state_of(1), Some(State::Inbox));
    assert_eq!(mb.tags(1), [Tag::NoReply]);
    assert_eq!(mb.waiting_since(1), None);

    let after = snapshot(&mb);
    let again = mb.tick(T0 + 3 * DAY);
    assert_eq!(again, Default::default());
    assert_eq!(snapshot(&mb), after);
    assert_ne!(after, undo_depth_probe);

    // tick pushed no undo step: undo reverts the earlier set_state_at.
    assert!(mb.undo());
    assert_eq!(mb.state_of(1), Some(State::Inbox));
    assert!(!mb.undo());
}

#[test]
fn waiting_does_not_resurface_when_someone_replied_after() {
    // Thread 1: reply arrives 2026-10-02, after we started waiting on T0.
    let mut mb = threaded(&[
        (1, 1, "a@x.test", "2026-09-01T00:00:00Z", State::Inbox),
        (2, 1, "a@x.test", "2026-10-01T12:00:00Z", State::Inbox),
        // Thread 2: the only other message is older than waiting_since.
        (3, 2, "a@x.test", "2026-09-01T00:00:00Z", State::Inbox),
        (4, 2, "a@x.test", "2026-09-02T00:00:00Z", State::Inbox),
    ]);
    mb.set_state_at(&[1, 3], State::Waiting, T0);
    let r = mb.tick(T0 + 4 * DAY);
    assert_eq!(r.resurfaced, vec![3]);
    assert_eq!(mb.state_of(1), Some(State::Waiting));
}

#[test]
fn leaving_and_reentering_waiting_drops_the_no_reply_tag() {
    let mut mb = threaded(&[(1, 1, "a@x.test", "2026-09-01T00:00:00Z", State::Inbox)]);
    mb.set_state_at(&[1], State::Waiting, T0);
    mb.tick(T0 + 3 * DAY);
    mb.set_state_at(&[1], State::Waiting, T0 + 3 * DAY);
    assert!(mb.tags(1).is_empty());
    assert_eq!(mb.waiting_since(1), Some(T0 + 3 * DAY));
}

#[test]
fn snooze_wakes_exactly_at_the_return_time_and_undoes() {
    let mut mb = sample();
    assert_eq!(mb.snooze(&[1, 2, 3], T0 + HOUR, T0), 3);
    assert_eq!(mb.state_of(1), Some(State::Later));
    assert_eq!(mb.snoozed_until(3), Some(T0 + HOUR));
    assert_eq!(mb.snooze(&[1], T0 + HOUR, T0), 0, "same time is a no-op");

    assert!(mb.tick(T0 + HOUR - 1).woken.is_empty());
    let r = mb.tick(T0 + HOUR);
    assert_eq!(r.woken.len(), 3);
    assert_eq!(mb.state_of(3), Some(State::Inbox), "message 3 was Later, now Inbox");
    assert_eq!(mb.snoozed_until(3), None);
    assert!(mb.tick(T0 + HOUR).woken.is_empty());

    assert!(mb.undo());
    assert_eq!(mb.state_of(1), Some(State::Inbox));
    assert_eq!(mb.state_of(3), Some(State::Later));
    assert_eq!(mb.snoozed_until(3), None);
}

#[test]
fn manually_leaving_later_clears_the_snooze() {
    let mut mb = sample();
    mb.snooze(&[1], T0 + HOUR, T0);
    mb.set_state(&[1], State::Done);
    assert_eq!(mb.snoozed_until(1), None);
    mb.tick(T0 + 2 * HOUR);
    assert_eq!(mb.state_of(1), Some(State::Done));
}

#[test]
fn snooze_presets_roll_over_and_land_on_the_right_weekday() {
    let mb = sample();
    // T0 = Thursday 00:00.
    let [tonight, tomorrow, monday] = mb.snooze_presets(T0 + 9 * HOUR);
    assert_eq!(tonight, ("Tonight", T0 + 18 * HOUR));
    assert_eq!(tomorrow, ("Tomorrow", T0 + DAY + 8 * HOUR));
    assert_eq!(monday, ("Monday", T0 + 4 * DAY + 8 * HOUR));
    // Past 18:00 (and exactly 18:00): Tonight becomes tomorrow evening.
    assert_eq!(mb.snooze_presets(T0 + 18 * HOUR)[0].1, T0 + DAY + 18 * HOUR);
    assert_eq!(mb.snooze_presets(T0 + 20 * HOUR)[0].1, T0 + DAY + 18 * HOUR);
    // On a Monday, "Monday" means next week.
    let mon = T0 + 4 * DAY + 10 * HOUR;
    assert_eq!(mb.snooze_presets(mon)[2].1, T0 + 11 * DAY + 8 * HOUR);
    // Sunday -> next day.
    assert_eq!(mb.snooze_presets(T0 + 3 * DAY)[2].1, T0 + 4 * DAY + 8 * HOUR);
}

#[test]
fn parse_snooze_accepts_units_and_rejects_junk() {
    let mb = sample();
    let _ = &mb;
    use mail_classifier::model::parse_snooze;
    assert_eq!(parse_snooze("30m", T0), Some(T0 + 1800));
    assert_eq!(parse_snooze(" 3H ", T0), Some(T0 + 3 * HOUR));
    assert_eq!(parse_snooze("2d", T0), Some(T0 + 2 * DAY));
    for bad in ["", "0m", "m", "3", "3w", "-1h", "1.5h", "3 h", "99999999999999999999d"] {
        assert_eq!(parse_snooze(bad, T0), None, "{bad:?}");
    }
}
