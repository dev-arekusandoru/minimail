use crate::harness::{Harness, harness_with};
use crate::snooze::snooze_box;
use gpui_kit::TestAppContext;
use mail_classifier::clock::DAY;
use mail_classifier::model::Tag;
use mail_classifier::model::TriageState::*;
use mail_classifier::outgoing::{DraftState, HOLD_SECONDS, SendIntent};

/// The intent the open draft carries.
fn intent(h: &mut Harness<'_>) -> SendIntent {
    h.read(|a| {
        let id = a.inline_composer.expect("an inline composer");
        a.composer.record(id).expect("the draft").intent
    })
}

/// Reply to the first message, open the intent menu from its chord and press `pick`.
fn reply_with_intent(h: &mut Harness<'_>, pick: &str) {
    h.compose_reply("see you then");
    h.settle();
    h.keys("cmd-shift-enter");
    assert!(h.has("compose-intent-menu"), "the menu opens");
    h.keys(pick);
}

#[gpui_kit::gpui::test]
pub fn archive_intent_files_the_thread_after_the_send_is_confirmed(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, snooze_box());
    let original = h.cursor().expect("message");
    reply_with_intent(&mut h, "1");
    assert_eq!(intent(&mut h), SendIntent::Archive);
    assert!(!h.has("compose-intent-menu"), "picking closes the menu");
    h.keys("cmd-enter");
    h.await_holds(1);
    assert_eq!(h.state_of(original), Inbox, "nothing is filed inside the hold");
    h.deliver_held();
    assert_eq!(h.state_of(original), Archived);
    h.keys("u");
    assert_eq!(h.state_of(original), Inbox, "one undo step puts the thread back");
}

#[gpui_kit::gpui::test]
pub fn remind_intent_flags_the_thread_and_resurfaces_it_without_a_reply(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, snooze_box());
    let original = h.cursor().expect("message");
    reply_with_intent(&mut h, "2");
    assert_eq!(intent(&mut h), SendIntent::RemindIfNoReply { days: 3 });
    h.keys("cmd-enter");
    h.await_holds(1);
    h.deliver_held();
    assert!(h.has_tag(original, Tag::AwaitingReply));
    h.advance(2 * DAY);
    assert!(!h.has_tag(original, Tag::FollowUp), "not yet");
    h.advance(DAY);
    assert!(h.has_tag(original, Tag::FollowUp), "three days on, it comes back");
    assert_eq!(h.state_of(original), Inbox);
}

#[gpui_kit::gpui::test]
pub fn send_later_holds_until_its_time_and_then_sends(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, snooze_box());
    // Send later asks when: `3`, then the second preset.
    reply_with_intent(&mut h, "3");
    assert!(h.has("compose-intent-menu"), "the presets page is open");
    h.keys("2");
    let SendIntent::SendLater { at } = intent(&mut h) else {
        panic!("send later is chosen");
    };
    h.keys("cmd-enter");
    h.await_holds(1);
    assert_eq!(
        h.read(|a| a.composer.holds.values().next().map(|hold| hold.hold_until)),
        Some(at)
    );
    h.advance(HOLD_SECONDS + 5);
    assert!(h.sent_bodies().iter().all(|b| !b.contains("see you then")), "still waiting");
    assert!(h.has("compose-send-pill"), "the composer shows the held message");
    assert!(!h.hold_pending(), "and `u` does not recall a scheduled send");
    let now = h.read(|a| a.now());
    h.advance(at - now);
    h.await_composer("the scheduled message goes out", |a| {
        a.mailbox.messages().iter().any(|m| m.outgoing && m.body.contains("see you then"))
    });
}

#[gpui_kit::gpui::test]
pub fn undo_during_the_hold_restores_the_draft_in_place_with_a_notice(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, snooze_box());
    h.compose_reply("still mine");
    h.settle();
    h.keys("cmd-enter");
    h.await_holds(1);
    assert!(h.has("compose-send-pill"), "the pill sits inside the frozen composer");
    assert!(h.has("compose-hold-undo"));
    h.click("compose-hold-undo");
    h.await_composer("the draft is editable again", |a| {
        a.composer.holds.is_empty()
            && a.composer
                .record(a.inline_composer.expect("still inline"))
                .is_some_and(|r| r.state == DraftState::Editing)
    });
    assert!(!h.has("compose-send-pill"));
    assert!(h.has("compose-restored"), "one line says nothing was sent");
    assert_eq!(h.draft_fields().3, "still mine");
}

#[gpui_kit::gpui::test]
pub fn the_send_button_asks_what_happens_after_and_picking_also_sends(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, snooze_box());
    let original = h.cursor().expect("message");
    h.compose_reply("via the button");
    h.settle();
    h.click("compose-send");
    assert!(h.has("compose-intent-menu"), "Send opens the menu");
    assert_eq!(h.hold_count(), 0, "nothing is held until a row is picked");
    h.keys("1");
    h.await_holds(1);
    assert_eq!(intent(&mut h), SendIntent::Archive);
    h.deliver_held();
    assert_eq!(h.state_of(original), Archived);
}
