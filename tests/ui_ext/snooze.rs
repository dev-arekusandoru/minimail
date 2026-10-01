use gpui_kit::{TestAppContext};
use mail_classifier::clock::{DAY, HOUR, Timestamp};
use mail_classifier::model::Mailbox;
use mail_classifier::model::TriageState::*;
use crate::harness::{MIDNIGHT, NOON, harness_with, mailbox, msg};

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
        h.keys("s");
        assert!(h.read(|a| a.snooze_open()), "s opens the picker");
        h.keys(key);
        assert!(!h.read(|a| a.snooze_open()));
        assert_eq!(h.state_of(id), Snoozed, "preset {key}");
        assert_eq!(h.read(|a| a.mailbox.snoozed_until(id)), Some(*until), "preset {key}");
        assert!(!h.visible().contains(&id));

        h.clock.set(*until - 1);
        h.tick();
        assert_eq!(h.state_of(id), Snoozed, "still asleep 1s early (preset {key})");
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

    h.keys("s escape");
    assert!(!h.read(|a| a.snooze_open()));
    assert_eq!(h.state_of(id), Inbox, "escape cancels the picker");

    h.keys("s");
    h.keys("4");
    h.type_text("3h");
    h.keys("enter");
    assert_eq!(h.state_of(id), Snoozed);
    assert_eq!(h.read(|a| a.mailbox.snoozed_until(id)), Some(NOON + 3 * HOUR));

    h.advance(3 * HOUR - 1);
    assert_eq!(h.state_of(id), Snoozed);
    h.advance(1);
    assert_eq!(h.state_of(id), Inbox);

    // Undo of a snooze restores the message immediately.
    h.keys("s");
    h.keys("4");
    h.type_text("30m");
    h.keys("enter");
    assert_eq!(h.state_of(id), Snoozed);
    h.keys("u");
    assert_eq!(h.state_of(id), Inbox);
    assert_eq!(h.read(|a| a.mailbox.snoozed_until(id)), None);
}

#[gpui_kit::gpui::test]
pub fn snoozed_view_lists_message_with_return_time(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, snooze_box());
    let id = h.cursor().unwrap();
    h.keys("s 2");
    h.keys("g s");
    assert_eq!(h.read(|a| a.triage.view.location.clone()), mail_classifier::model::Location::Snoozed("personal".into()));
    assert_eq!(h.visible(), vec![id]);
    assert!(h.read(|a| a.mailbox.snoozed_until(id)).is_some());
}
