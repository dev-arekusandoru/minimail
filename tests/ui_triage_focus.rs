//! Triage mode's flow: the Start screen, skipping, taking the suggestion, and the Summary.

use gpui_kit::{AppContext, TestAppContext};
use gpui_kit::test::TestWindowExt;
use mail_classifier::model::TriageState::*;
use mail_classifier::triage_session::{SessionOrder, SessionSize};
use mail_classifier::triage_suggest::SuggestedAction;

#[path = "ui_ext/harness.rs"]
#[allow(dead_code)]
mod harness;
use harness::{harness_with, mailbox, msg};

/// Inbox newest first: 4, 3, 2.
fn inbox() -> mail_classifier::model::Mailbox {
    mailbox(&[
        msg(2, 2, "b@a.io", "two", 22, "Inbox"),
        msg(3, 3, "c@a.io", "three", 23, "Inbox"),
        msg(4, 4, "d@a.io", "four", 24, "Inbox"),
    ])
}

/// Sender `s@x.io` archived three times before; their newest mail (id 9) and one other (id 8).
fn habit() -> mail_classifier::model::Mailbox {
    mailbox(&[
        msg(1, 1, "s@x.io", "old", 1, "Archived"),
        msg(2, 2, "s@x.io", "old", 2, "Archived"),
        msg(3, 3, "s@x.io", "old", 3, "Archived"),
        msg(8, 8, "o@x.io", "other", 20, "Inbox"),
        msg(9, 9, "s@x.io", "new", 21, "Inbox"),
    ])
}

#[gpui_kit::gpui::test]
fn the_start_screen_options_shape_the_session(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, inbox());
    h.keys("t");
    let view = h.read(|a| a.session_setup().cloned()).expect("start screen");
    assert_eq!((view.total, view.queue_len), (3, 3));
    h.app.update(h.cx, |a, cx| {
        a.setup_set_order(SessionOrder::OldestFirst, cx);
        a.setup_set_size(SessionSize::N25, cx);
    });
    assert_eq!(h.read(|a| a.session_setup().map(|v| v.options.order)), Some(SessionOrder::OldestFirst));
    h.keys("enter");
    assert_eq!(h.read(|a| a.opened()), Some(2), "oldest first");
    assert_eq!(h.read(|a| a.session_progress()), Some((1, 3)));
}

#[gpui_kit::gpui::test]
fn skip_defers_a_message_without_a_state_change_or_undo_step(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, inbox());
    h.keys("t enter");
    let depth = h.read(|a| a.mailbox.undo_depth());
    h.keys("space");
    assert_eq!(h.read(|a| a.opened()), Some(3), "the next message is up");
    assert_eq!(h.read(|a| a.session_progress()), Some((1, 3)), "the total stays the queue length");
    assert_eq!(h.state_of(4), Inbox);
    assert_eq!(h.read(|a| a.mailbox.undo_depth()), depth, "skipping is not an undo step");

    h.keys("e e");
    assert_eq!(h.read(|a| a.opened()), Some(4), "the skipped message comes round again");
    assert_eq!(h.read(|a| a.session_progress()), Some((3, 3)));
    h.keys("e");
    let s = h.read(|a| a.session_summary().copied()).expect("summary");
    assert_eq!((s.handled, s.tally.skipped, s.tally.archived), (3, 1, 3));
    assert_eq!(s.remaining, 0);
}

#[gpui_kit::gpui::test]
fn accepting_the_suggestion_is_one_undo_step_and_is_tallied(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, habit());
    h.keys("t enter");
    assert_eq!(h.read(|a| a.opened()), Some(9));
    let suggestion = h.read(|a| a.session_suggestion().cloned()).expect("history says archive");
    assert_eq!(suggestion.action, SuggestedAction::Archive);
    assert_eq!(suggestion.reason, "you've archived 3 of 3 from this sender");

    let depth = h.read(|a| a.mailbox.undo_depth());
    h.keys("enter");
    assert_eq!(h.state_of(9), Archived);
    assert_eq!(h.read(|a| a.mailbox.undo_depth()), depth + 1, "one undo step");
    assert_eq!(h.read(|a| a.opened()), Some(8));
    assert_eq!(h.read(|a| a.session_suggestion().cloned()), None, "no history for the next sender: no card");
    h.keys("enter");
    assert_eq!(h.state_of(8), Inbox, "enter without a suggestion does nothing");
    h.keys("d");
    let s = h.read(|a| a.session_summary().copied()).expect("summary");
    assert_eq!((s.handled, s.tally.archived, s.tally.deleted, s.tally.suggestions_taken), (2, 1, 1, 1));
    h.keys("escape");
    h.keys("u");
    assert_eq!(h.state_of(8), Inbox);
    h.assert_invariant("suggestion");
}

#[gpui_kit::gpui::test]
fn the_sender_history_suggestion_is_visible_in_the_triage_dock(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, habit());
    h.keys("t enter");
    h.cx.update_window(h.window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("triage-accept").is_some(), "suggestion card accept control is rendered");
    }).expect("window alive");
}

#[gpui_kit::gpui::test]
fn the_ai_toggle_hides_the_suggestion(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, habit());
    h.keys("t");
    h.app.update(h.cx, |a, cx| a.setup_set_suggestions(false, cx));
    h.keys("enter");
    assert_eq!(h.read(|a| a.session_suggestion().cloned()), None);
    h.keys("enter");
    assert_eq!(h.state_of(9), Inbox, "nothing to accept");
}

#[gpui_kit::gpui::test]
fn keep_going_restarts_with_the_same_options_and_a_sent_reply_counts(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, inbox());
    h.keys("t");
    h.app.update(h.cx, |a, cx| a.setup_set_order(SessionOrder::OldestFirst, cx));
    h.keys("enter");
    h.send_reply("thanks");
    assert_eq!(h.read(|a| a.session_progress()), Some((2, 3)), "the reply handled message 2");
    h.keys("escape");
    assert!(h.read(|a| !a.modal_open()), "the post-send dialog closes first");
    h.keys("escape");
    let s = h.read(|a| a.session_summary().copied()).expect("summary");
    assert_eq!((s.handled, s.tally.replied), (1, 1));

    h.keys("enter");
    assert_eq!(h.read(|a| a.session_summary().copied()), None);
    assert_eq!(h.read(|a| a.session_options().map(|o| o.order)), Some(SessionOrder::OldestFirst));
    assert_eq!(h.read(|a| a.session_progress()), Some((1, 3)), "a new session over what is still inbox");
}
