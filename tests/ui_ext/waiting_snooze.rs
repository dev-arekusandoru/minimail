use gpui_kit::{TestAppContext};
use mail_classifier::clock::{DAY, HOUR, Timestamp};
use mail_classifier::model::{Mailbox, Tag};
use mail_classifier::model::TriageState::*;
use crate::harness::{MIDNIGHT, NOON, harness_with, mailbox, msg};

// ---------------------------------------------------------------- Waiting resurfacing

#[gpui_kit::gpui::test]
pub fn waiting_resurfaces_after_three_days_with_no_reply_tag(cx: &mut TestAppContext) {
    let mut h = harness_with(
        cx,
        mailbox(&[
            msg(1, 1, "carol@a.io", "alpha", 20, "Inbox"),
            msg(2, 2, "dana@a.io", "beta", 19, "Inbox"),
            // A reply from someone else in thread 2 that arrives after msg 2 started waiting.
            msg(3, 2, "erin@a.io", "Re: beta", 30, "Done"),
        ]),
    );
    h.goto(1);
    h.keys("w");
    h.goto(2);
    h.keys("w");
    assert_eq!(h.state_of(1), Waiting);
    assert_eq!(h.state_of(2), Waiting);

    h.advance(3 * DAY - 1);
    assert_eq!(h.state_of(1), Waiting, "must not resurface before 3 days");

    h.advance(1);
    assert_eq!(h.state_of(1), Inbox, "resurfaces at exactly 3 days");
    assert!(h.has_tag(1, Tag::NoReply));
    assert_eq!(
        h.state_of(2),
        Waiting,
        "a newer message from someone else keeps the thread waiting"
    );
    assert!(!h.has_tag(2, Tag::NoReply));

    // Idempotent.
    h.advance(DAY);
    assert_eq!(h.state_of(1), Inbox);
    assert_eq!(h.state_of(2), Waiting);
    h.assert_invariant("waiting resurfacing");
}

// ---------------------------------------------------------------- Snooze

pub fn snooze_box() -> Mailbox {
    mailbox(&[
        msg(1, 1, "a@a.io", "one", 25, "Inbox"),
        msg(2, 2, "b@a.io", "two", 24, "Inbox"),
        msg(3, 3, "c@a.io", "three", 23, "Inbox"),
    ])
}

#[gpui_kit::gpui::test]
pub fn snooze_presets_hide_until_time_then_return(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, snooze_box());
    let cases: [(&str, Timestamp); 3] = [
        ("1", MIDNIGHT + 18 * HOUR),
        ("2", MIDNIGHT + DAY + 8 * HOUR),
        ("3", MIDNIGHT + 6 * DAY + 8 * HOUR), // next Monday (Tue + 6)
    ];
    for (i, (key, until)) in cases.iter().enumerate() {
        let id = (i + 1) as u32;
        h.clock.set(NOON);
        h.goto(id);
        h.keys("l");
        assert!(h.read(|a| a.snooze_open()), "l opens the picker");
        h.keys(key);
        assert!(!h.read(|a| a.snooze_open()));
        assert_eq!(h.state_of(id), Later, "preset {key}");
        assert_eq!(h.read(|a| a.mailbox.snoozed_until(id)), Some(*until), "preset {key}");
        assert!(!h.visible().contains(&id));

        h.clock.set(*until - 1);
        h.tick();
        assert_eq!(h.state_of(id), Later, "still asleep 1s early (preset {key})");
        h.clock.set(*until);
        h.tick();
        assert_eq!(h.state_of(id), Inbox, "wakes at the return time (preset {key})");
    }
    h.assert_invariant("snooze presets");
}

#[gpui_kit::gpui::test]
pub fn snooze_custom_duration_and_escape(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, snooze_box());
    let id = h.cursor().unwrap();

    h.keys("l escape");
    assert!(!h.read(|a| a.snooze_open()));
    assert_eq!(h.state_of(id), Inbox, "escape cancels the picker");

    h.keys("l 4");
    h.type_text("3h");
    h.keys("enter");
    assert_eq!(h.state_of(id), Later);
    assert_eq!(h.read(|a| a.mailbox.snoozed_until(id)), Some(NOON + 3 * HOUR));

    h.advance(3 * HOUR - 1);
    assert_eq!(h.state_of(id), Later);
    h.advance(1);
    assert_eq!(h.state_of(id), Inbox);

    // Undo of a snooze restores the message immediately.
    h.keys("l 4");
    h.type_text("30m");
    h.keys("enter");
    assert_eq!(h.state_of(id), Later);
    h.keys("u");
    assert_eq!(h.state_of(id), Inbox);
    assert_eq!(h.read(|a| a.mailbox.snoozed_until(id)), None);
}

#[gpui_kit::gpui::test]
pub fn later_view_lists_snoozed_message_with_return_time(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, snooze_box());
    let id = h.cursor().unwrap();
    h.keys("l 2 3");
    assert_eq!(h.view_state(), Later);
    assert_eq!(h.visible(), vec![id]);
    assert!(h.read(|a| a.mailbox.snoozed_until(id)).is_some());
}
