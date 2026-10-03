//! The AI and Senders settings pages, driven through the real settings window.

use gpui_kit::TestAppContext;
use mail_classifier::clock::DAY;
use mail_classifier::judge::{Confidence, Mode, QuestionKey};

#[path = "ui_ext/harness.rs"]
#[allow(dead_code)]
mod harness;
use harness::{Harness, harness_with, mailbox, msg};

/// Sidebar page indexes: Accounts, Appearance, Inbox, AI, Senders.
const AI: usize = 3;
const SENDERS: usize = 4;

fn spam(h: &mut Harness<'_>) -> Mode {
    h.read(|a| a.policy.mode(QuestionKey::Spam))
}

/// Off is the third segment of every row.
fn set_spam_mode(h: &mut Harness<'_>, segment: usize) {
    // Group 1 is "Tagging"; its first row is Spam.
    h.settings_click_in(1, 0, format!("ai-mode-Spam-{segment}"));
}

#[gpui_kit::gpui::test]
fn turning_spam_off_stops_it_producing_suggestions(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox(&[msg(1, 1, "deals@talentloop.com", "Is this a fit?", 1, "Inbox")]));
    h.settings_open_page(AI);
    set_spam_mode(&mut h, 2);
    h.keys("c");
    assert!(
        h.read(|a| a.mailbox.pending(1).is_empty()),
        "Off suggests nothing"
    );

    set_spam_mode(&mut h, 1);
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
    h.settings_open_page(AI);
    assert_eq!(spam(&mut h), Mode::Auto(Confidence::High), "the shipped default");

    // Row 1 of "Tagging" is Spam; row 2 is its confidence, shown while Auto.
    h.settings_pick_option(1, 1, 2);
    assert_eq!(spam(&mut h), Mode::Auto(Confidence::Low));

    set_spam_mode(&mut h, 2);
    assert_eq!(spam(&mut h), Mode::Off);

    h.keys("cmd-,");
    h.settings_open_page(AI);
    assert_eq!(spam(&mut h), Mode::Off, "the policy outlives the row that is hidden");
    set_spam_mode(&mut h, 0);
    assert_eq!(spam(&mut h), Mode::Auto(Confidence::Medium), "back to Auto, at the middle preset");
}

#[gpui_kit::gpui::test]
fn the_filter_narrows_the_blocked_senders(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox(&[msg(1, 1, "alice@example.com", "Question", 1, "Inbox")]));
    h.app.update(h.cx, |a, _| {
        a.mailbox.block_sender("spammer@example.com", None, harness::NOON - 4 * DAY);
        a.mailbox.block_sender("boss@example.com", None, harness::NOON - DAY);
    });
    h.settings_open_page(SENDERS);

    h.settings_click("blocked-filter");
    h.settings_type("spammer");
    assert!(h.settings_has("blocked-unblock-spammer@example.com"));
    assert!(!h.settings_has("blocked-unblock-boss@example.com"), "the filter hides the rest");

    h.settings_keys("cmd-a");
    h.settings_type("nobody");
    assert!(
        !h.settings_has("blocked-unblock-spammer@example.com"),
        "a filter nothing matches leaves no rows"
    );
}

#[gpui_kit::gpui::test]
fn unblocking_a_sender_then_undoing_restores_it_with_its_block_time(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox(&[msg(1, 1, "spammer@example.com", "Deal", 1, "Inbox")]));
    let blocked_at = harness::NOON - 3 * DAY;
    h.app.update(h.cx, |a, _| {
        a.mailbox.block_sender("spammer@example.com", None, blocked_at);
    });
    h.settings_open_page(SENDERS);
    assert!(h.settings_has("blocked-unblock-spammer@example.com"));

    h.settings_click("blocked-unblock-spammer@example.com");
    assert!(h.read(|a| a.mailbox.blocked().is_empty()), "Unblock removes the sender");
    assert!(h.toast().contains("undo"), "the toast offers Undo: {}", h.toast());

    h.keys("u");
    assert_eq!(
        h.read(|a| a.mailbox.blocked()),
        vec![("spammer@example.com".to_owned(), blocked_at)],
        "undo restores the block, with the time it was blocked"
    );
}
