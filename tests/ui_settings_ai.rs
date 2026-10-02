//! The AI and Senders settings pages, driven through the real settings window.

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, ElementId, TestAppContext};
use mail_classifier::clock::DAY;
use mail_classifier::judge::{Confidence, Mode, QuestionKey};
use mail_classifier::model::blocked_ago;

#[path = "ui_ext/harness.rs"]
#[allow(dead_code)]
mod harness;
use harness::{Harness, harness_with, mailbox, msg};

/// Sidebar page indexes: Accounts, Appearance, Inbox, AI, Senders.
const AI: usize = 3;
const SENDERS: usize = 4;

/// Open the settings window and return its handle.
fn open_settings(h: &mut Harness<'_>) -> AnyWindowHandle {
    h.keys("cmd-,");
    h.cx.windows().into_iter().find(|w| *w != h.window).expect("a settings window")
}

/// Click the sidebar entry of `page`.
fn open_page(h: &mut Harness<'_>, settings: AnyWindowHandle, page: usize) {
    h.cx.update_window(settings, |_, window, cx| window.click(format!("0-{page}"), cx)).unwrap();
    h.cx.run_until_parked();
}

/// Click `target` inside item `item` of group `group` of the open page.
fn click_in(
    h: &mut Harness<'_>,
    settings: AnyWindowHandle,
    group: usize,
    item: usize,
    target: impl Into<ElementId>,
) {
    h.cx.update_window(settings, |_, window, cx| {
        window.within(format!("group-{group}")).within(format!("item-{item}")).click(target, cx)
    })
    .expect("settings window alive");
    h.cx.run_until_parked();
}

/// Open the dropdown of an item and choose its `option`-th entry.
fn pick_option(
    h: &mut Harness<'_>,
    settings: AnyWindowHandle,
    group: usize,
    item: usize,
    option: usize,
) {
    click_in(h, settings, group, item, "btn");
    h.cx.update_window(settings, |_, window, cx| {
        window.within("popup-menu").click(option, cx)
    })
    .expect("settings window alive");
    h.cx.run_until_parked();
}

fn spam(h: &mut Harness<'_>) -> Mode {
    h.read(|a| a.policy.mode(QuestionKey::Spam))
}

/// Off is the third segment of every row.
fn set_spam_mode(h: &mut Harness<'_>, settings: AnyWindowHandle, segment: usize) {
    // Group 1 is "Tagging"; its first row is Spam.
    click_in(h, settings, 1, 0, format!("ai-mode-Spam-{segment}"));
}

#[gpui_kit::gpui::test]
fn turning_spam_off_stops_it_producing_suggestions(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox(&[msg(1, 1, "deals@talentloop.com", "Is this a fit?", 1, "Inbox")]));
    let settings = open_settings(&mut h);
    open_page(&mut h, settings, AI);
    set_spam_mode(&mut h, settings, 2);
    h.keys("c");
    assert!(
        h.read(|a| a.mailbox.pending(1).is_empty()),
        "Off suggests nothing"
    );

    set_spam_mode(&mut h, settings, 1);
    assert_eq!(spam(&mut h), Mode::Review);
    h.keys("c");
    assert_eq!(
        h.read(|a| a.mailbox.pending(1).iter().map(|s| s.key).collect::<Vec<_>>()),
        vec![QuestionKey::Spam],
        "the same check does suggest when it is not off"
    );
}

#[gpui_kit::gpui::test]
fn the_confidence_row_picks_how_sure_auto_has_to_be(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox(&[msg(1, 1, "alice@example.com", "Question", 1, "Inbox")]));
    let settings = open_settings(&mut h);
    open_page(&mut h, settings, AI);
    assert_eq!(spam(&mut h), Mode::Auto(Confidence::High), "the shipped default");

    // Row 1 of "Tagging" is Spam; row 2 is its confidence, shown while Auto.
    pick_option(&mut h, settings, 1, 1, 2);
    assert_eq!(spam(&mut h), Mode::Auto(Confidence::Low));

    set_spam_mode(&mut h, settings, 2);
    assert_eq!(spam(&mut h), Mode::Off);

    h.keys("cmd-,");
    let reopened = open_settings(&mut h);
    open_page(&mut h, reopened, AI);
    assert_eq!(spam(&mut h), Mode::Off, "the policy outlives the row that is hidden");
    set_spam_mode(&mut h, reopened, 0);
    assert_eq!(spam(&mut h), Mode::Auto(Confidence::Medium), "back to Auto, at the middle preset");
}

#[gpui_kit::gpui::test]
fn the_filter_narrows_the_blocked_senders(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox(&[msg(1, 1, "alice@example.com", "Question", 1, "Inbox")]));
    h.app.update(h.cx, |a, _| {
        a.mailbox.block_sender("spammer@example.com", None, harness::NOON - 4 * DAY);
        a.mailbox.block_sender("boss@example.com", None, harness::NOON - DAY);
    });
    let settings = open_settings(&mut h);
    open_page(&mut h, settings, SENDERS);

    h.cx.update_window(settings, |_, window, cx| {
        window.click("blocked-filter", cx);
        window.input("spammer", cx);
    })
    .expect("settings window alive");
    h.cx.run_until_parked();
    h.cx.update_window(settings, |_, window, cx| window.render_frame(cx)).unwrap();

    assert!(find(&mut h, settings, "blocked-unblock-spammer@example.com"));
    assert!(!find(&mut h, settings, "blocked-unblock-boss@example.com"), "the filter hides the rest");

    h.cx.update_window(settings, |_, window, cx| {
        window.click("blocked-filter", cx);
        window.press("cmd-a", cx);
        window.input("nobody", cx);
        window.render_frame(cx);
    })
    .expect("settings window alive");
    h.cx.run_until_parked();
    assert!(
        !find(&mut h, settings, "blocked-unblock-spammer@example.com"),
        "a filter nothing matches leaves no rows"
    );
}

fn find(h: &mut Harness<'_>, settings: AnyWindowHandle, id: impl Into<ElementId>) -> bool {
    h.cx.update_window(settings, |_, window, _| window.try_find(id).is_some()).unwrap_or(false)
}

#[gpui_kit::gpui::test]
fn unblocking_a_sender_then_undoing_restores_it_with_its_block_time(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox(&[msg(1, 1, "spammer@example.com", "Deal", 1, "Inbox")]));
    let blocked_at = harness::NOON - 3 * DAY;
    h.app.update(h.cx, |a, _| {
        a.mailbox.block_sender("spammer@example.com", None, blocked_at);
    });
    let settings = open_settings(&mut h);
    open_page(&mut h, settings, SENDERS);
    assert!(find(&mut h, settings, "blocked-unblock-spammer@example.com"));

    h.cx.update_window(settings, |_, window, cx| {
        window.click("blocked-unblock-spammer@example.com", cx);
    })
    .expect("settings window alive");
    h.cx.run_until_parked();
    assert!(h.read(|a| a.mailbox.blocked().is_empty()), "Unblock removes the sender");
    assert!(h.toast().contains("undo"), "the toast offers Undo: {}", h.toast());

    h.keys("u");
    assert_eq!(
        h.read(|a| a.mailbox.blocked()),
        vec![("spammer@example.com".to_owned(), blocked_at)],
        "undo restores the block, with the time it was blocked"
    );
}

#[test]
fn the_blocked_line_counts_days_from_the_app_clock() {
    let now = harness::NOON;
    assert_eq!(blocked_ago(now, now), "Blocked today");
    assert_eq!(blocked_ago(now, now - DAY), "Blocked yesterday");
    assert_eq!(blocked_ago(now, now - 4 * DAY), "Blocked 4 days ago");
    assert_eq!(blocked_ago(now, now + DAY), "Blocked today", "clock skew reads as today");
}

