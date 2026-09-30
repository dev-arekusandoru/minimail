//! Settings panel behaviour, driven through gpui-kit's `Settings` component in the real app.

use gpui_kit::{AppContext, TestAppContext};
use gpui_kit::test::TestWindowExt;
use mail_classifier::app::mail_app::panes::Orientation;
use mail_classifier::clock::DAY;
use mail_classifier::judge::{Mode, QuestionKey};
use mail_classifier::model::{Tag, TriageState};

#[path = "ui_ext/harness.rs"]
#[allow(dead_code)]
mod harness;
use harness::{Harness, harness_with, mailbox, msg};

/// Sidebar page indexes.
const APPEARANCE: usize = 1;
const INBOX: usize = 2;
const BLOCKED: usize = 3;
const CLASSIFIER: usize = 4;

fn app(cx: &mut TestAppContext) -> Harness<'_> {
    harness_with(cx, mailbox(&[msg(1, 1, "alice@example.com", "Question", 1, "Inbox")]))
}

fn open_page(h: &mut Harness<'_>, page: usize) {
    h.keys("cmd-,");
    if page != 0 {
        h.click(format!("0-{page}"));
    }
}

/// Click `target` inside item `item` of group `group` of the open page.
fn click_in(h: &mut Harness<'_>, group: usize, item: usize, target: &'static str) {
    h.cx.update_window(h.window, |_, window, cx| {
        window.within(format!("group-{group}")).within(format!("item-{item}")).click(target, cx)
    })
    .expect("window alive");
    h.cx.run_until_parked();
}

/// Open the dropdown of an item and choose its `option`-th entry.
fn pick_option(h: &mut Harness<'_>, group: usize, item: usize, option: usize) {
    click_in(h, group, item, "btn");
    h.cx.update_window(h.window, |_, window, cx| window.within("popup-menu").click(option, cx))
        .expect("window alive");
    h.cx.run_until_parked();
}

#[gpui_kit::gpui::test]
fn the_summaries_switch_enables_summaries_and_escape_closes(cx: &mut TestAppContext) {
    let mut h = app(cx);
    assert!(!h.read(|a| a.summaries_enabled));
    open_page(&mut h, 0);
    assert!(h.read(|a| a.settings_open()));
    click_in(&mut h, 0, 0, "check");
    assert!(h.read(|a| a.summaries_enabled));
    click_in(&mut h, 0, 0, "check");
    assert!(!h.read(|a| a.summaries_enabled));
    h.keys("escape");
    assert!(!h.read(|a| a.settings_open()));
}

#[gpui_kit::gpui::test]
fn the_tab_avatar_switch_turns_the_setting_off_and_back_on(cx: &mut TestAppContext) {
    let mut h = app(cx);
    assert!(h.read(|a| a.tab_avatars), "on by default");
    open_page(&mut h, APPEARANCE);
    click_in(&mut h, 0, 2, "check");
    assert!(!h.read(|a| a.tab_avatars));
    // The panel reopens showing the current value, so the next click turns it back on.
    h.keys("escape cmd-,");
    h.click(format!("0-{APPEARANCE}"));
    click_in(&mut h, 0, 2, "check");
    assert!(h.read(|a| a.tab_avatars));
}

#[gpui_kit::gpui::test]
fn the_pane_layout_dropdown_switches_the_orientation(cx: &mut TestAppContext) {
    let mut h = app(cx);
    open_page(&mut h, APPEARANCE);
    pick_option(&mut h, 0, 1, 1);
    assert_eq!(h.read(|a| a.panes.orientation()), Orientation::Stacked);
    pick_option(&mut h, 0, 1, 0);
    assert_eq!(h.read(|a| a.panes.orientation()), Orientation::SideBySide);
}

#[gpui_kit::gpui::test]
fn the_number_input_steps_the_follow_up_timeout(cx: &mut TestAppContext) {
    let mut h = app(cx);
    open_page(&mut h, INBOX);
    click_in(&mut h, 0, 2, "decrement");
    assert_eq!(h.read(|a| a.mailbox.follow_up_timeout()), 2 * DAY);
    click_in(&mut h, 0, 2, "increment");
    click_in(&mut h, 0, 2, "increment");
    assert_eq!(h.read(|a| a.mailbox.follow_up_timeout()), 4 * DAY);
}

#[gpui_kit::gpui::test]
fn lowering_follow_up_after_resurfaces_a_waiting_thread_on_tick(cx: &mut TestAppContext) {
    let mut h = app(cx);
    // Reply and expect an answer: the thread now waits on the default 3-day timeout.
    h.app.update(h.cx, |a, _| a.mailbox.send_reply_at(1, "on it".into(), true, harness::NOON));
    h.advance(DAY);
    assert!(!h.has_tag(1, Tag::FollowUp), "three days of patience: one day is not enough");

    open_page(&mut h, INBOX);
    click_in(&mut h, 0, 2, "decrement");
    click_in(&mut h, 0, 2, "decrement");
    assert_eq!(h.read(|a| a.mailbox.follow_up_timeout()), DAY);
    h.keys("escape");

    h.tick();
    assert_eq!(h.state_of(1), TriageState::Inbox);
    assert!(h.has_tag(1, Tag::FollowUp), "the overdue thread is flagged for follow-up");
    assert!(!h.has_tag(1, Tag::AwaitingReply), "resurfacing clears Awaiting Reply");
}

#[gpui_kit::gpui::test]
fn the_group_switch_and_preview_dropdown_reach_the_inbox(cx: &mut TestAppContext) {
    let mut h = app(cx);
    open_page(&mut h, INBOX);
    let grouped = h.read(|a| a.group_threads);
    click_in(&mut h, 0, 0, "check");
    assert_eq!(h.read(|a| a.group_threads), !grouped);
    pick_option(&mut h, 0, 1, 4);
    assert_eq!(h.read(|a| a.preview_lines), 4);
}

#[gpui_kit::gpui::test]
fn the_threshold_steps_in_auto_and_is_locked_in_review(cx: &mut TestAppContext) {
    let mut h = app(cx);
    let spam = |h: &mut Harness<'_>| h.read(|a| a.policy.threshold(QuestionKey::Spam));
    let before = spam(&mut h);
    open_page(&mut h, CLASSIFIER);
    click_in(&mut h, 0, 1, "decrement");
    assert!((spam(&mut h) - (before - 0.05)).abs() < 0.001, "step is 5 points");

    pick_option(&mut h, 0, 0, 1);
    assert!(matches!(h.read(|a| a.policy.mode(QuestionKey::Spam)), Mode::Review));
    let locked = spam(&mut h);
    click_in(&mut h, 0, 1, "decrement");
    assert_eq!(spam(&mut h), locked, "the threshold input is disabled in Review");

    pick_option(&mut h, 0, 0, 0);
    assert!(matches!(h.read(|a| a.policy.mode(QuestionKey::Spam)), Mode::Auto { .. }));
}

#[gpui_kit::gpui::test]
fn unblocking_a_sender_from_settings_is_one_undoable_step(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox(&[msg(1, 1, "spam@example.com", "Deal", 1, "Inbox")]));
    h.app.update(h.cx, |a, _| {
        a.mailbox.block_sender("spam@example.com", None);
    });
    open_page(&mut h, BLOCKED);
    h.click(("blocked-unblock", 0usize));
    assert!(h.read(|a| a.mailbox.blocked().is_empty()), "Unblock removes the sender");

    h.keys("escape");
    h.keys("u");
    assert!(
        h.read(|a| a.mailbox.blocked().contains(&"spam@example.com".to_owned())),
        "undo restores the block"
    );
}
