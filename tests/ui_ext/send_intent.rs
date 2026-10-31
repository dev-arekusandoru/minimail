use crate::harness::{Harness, NOON, harness_with};
use crate::snooze::snooze_box;
use gpui_kit::TestAppContext;
use mail_classifier::clock::{DAY, HOUR, MINUTE};
use mail_classifier::model::Tag;
use mail_classifier::model::TriageState::*;
use mail_classifier::outgoing::{DraftState, HOLD_SECONDS, SendIntent};

/// The intent the open draft carries.
fn intent(h: &mut Harness<'_>) -> SendIntent {
    h.read(|a| {
        let id = a.inline_draft().or_else(|| a.tabs.active_draft())
            .expect("an open composer");
        a.composer.record(id).expect("the draft").intent()
    })
}

/// Send the sentence as it already reads: ⌘↵ opens the menu, Enter takes the lit row.
fn send(h: &mut Harness<'_>) {
    h.keys("cmd-enter");
    assert!(h.has("compose-intent-menu"), "⌘↵ asks what happens after");
    h.keys("enter");
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
    send(&mut h);
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
    send(&mut h);
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
    send(&mut h);
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
    send(&mut h);
    h.await_holds(1);
    assert!(h.has("compose-send-pill"), "the pill sits inside the frozen composer");
    assert!(h.has("compose-hold-undo"));
    h.click("compose-hold-undo");
    h.await_composer("the draft is editable again", |a| {
        a.composer.holds.is_empty()
            && a.composer
                .record(a.inline_draft().expect("still inline"))
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

#[gpui_kit::gpui::test]
pub fn cmd_enter_opens_the_send_menu_instead_of_sending(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, snooze_box());
    h.compose_reply("ask first");
    h.settle();
    h.keys("cmd-enter");
    assert!(h.has("compose-intent-menu"), "⌘↵ opens the same menu as Send");
    assert_eq!(h.hold_count(), 0, "nothing goes out before a row is picked");
    h.keys("escape");
    assert!(!h.has("compose-intent-menu"), "escape leaves the message alone");
    assert_eq!(h.hold_count(), 0);
}

#[gpui_kit::gpui::test]
pub fn a_typed_send_later_time_lands_on_the_draft(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, snooze_box());
    h.compose_reply("typed time");
    h.settle();
    // Send later asks when: `3`, then `4` for a typed date and time.
    h.keys("cmd-shift-enter");
    h.keys("3");
    h.keys("4");
    h.type_text("14:30");
    h.keys("enter");
    assert_eq!(
        intent(&mut h),
        SendIntent::SendLater {
            at: NOON + 2 * HOUR + 30 * MINUTE
        },
        "14:30 today is the intent the draft carries"
    );
    assert!(!h.has("compose-intent-menu"), "the choice closes the menu");
}

#[gpui_kit::gpui::test]
pub fn a_typed_time_in_the_past_is_refused(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, snooze_box());
    h.compose_reply("too late");
    h.settle();
    h.keys("cmd-shift-enter");
    h.keys("3");
    h.keys("4");
    h.type_text("09:00");
    h.keys("enter");
    assert!(h.has("compose-intent-menu"), "the menu stays open on a bad time");
    assert_eq!(intent(&mut h), SendIntent::Archive, "the sentence is unchanged");
}

#[gpui_kit::gpui::test]
pub fn the_menu_hangs_directly_under_the_then_token(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, snooze_box());
    h.compose_reply("where does it go");
    h.settle();
    h.keys("cmd-shift-enter");
    assert!(h.has("compose-intent-menu"), "the menu opens");
    let (token, menu) = {
        let mut view = gpui_kit::VisualTestContext::from_window(h.window, h.cx);
        (
            view.debug_bounds("compose-intent").expect("the token is laid out"),
            view.debug_bounds("compose-intent-menu").expect("the menu is laid out"),
        )
    };
    let below = f32::from(menu.top() - token.bottom());
    assert!((0. ..=16.).contains(&below), "the menu starts under the token, {below}px away");
    assert!(
        menu.left() <= token.right() && menu.right() >= token.left(),
        "and hangs from the token's column (token {token:?}, menu {menu:?})"
    );
}

#[gpui_kit::gpui::test]
pub fn the_send_menu_hangs_off_the_send_control(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, snooze_box());
    h.compose_reply("anchored to Send");
    h.settle();
    h.click("compose-send");
    assert!(h.has("compose-intent-menu"), "Send opens the menu");
    let (send, menu) = {
        let mut view = gpui_kit::VisualTestContext::from_window(h.window, h.cx);
        (
            view.debug_bounds("compose-send").expect("Send is laid out"),
            view.debug_bounds("compose-intent-menu").expect("the menu is laid out"),
        )
    };
    // The footer is at the bottom edge, so the menu turns up: its bottom meets Send's top.
    let gap = f32::from(menu.bottom() - send.top()).abs();
    assert!(gap <= 16., "the menu meets Send's edge, {gap}px apart");
    assert!(
        menu.left() <= send.right() && menu.right() >= send.left(),
        "and hangs from its column (send {send:?}, menu {menu:?})"
    );
}

#[gpui_kit::gpui::test]
pub fn a_reply_already_reads_archive_and_sending_takes_it(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, snooze_box());
    let original = h.cursor().expect("message");
    h.compose_reply("see you then");
    h.settle();
    assert_eq!(intent(&mut h), SendIntent::Archive, "a reply leaves the inbox once it is sent");
    send(&mut h);
    h.await_holds(1);
    h.deliver_held();
    assert_eq!(h.state_of(original), Archived);
}

#[gpui_kit::gpui::test]
pub fn a_new_message_has_no_archive_to_pick_and_never_starts_on_one(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, snooze_box());
    h.new_draft();
    assert_eq!(intent(&mut h), SendIntent::JustSend, "a new conversation just leaves");
    h.keys("cmd-enter");
    assert!(h.has("compose-intent-menu"), "the menu opens");
    // Archive used to be row `1`: without a thread, that key is the reminder instead.
    h.keys("1");
    assert_eq!(
        intent(&mut h),
        SendIntent::RemindIfNoReply { days: 3 },
        "there is no archive row to take"
    );
}
