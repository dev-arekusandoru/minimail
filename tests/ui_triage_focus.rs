//! Triage mode's flow: the Start screen, skipping, taking the suggestion, and the Summary.

use gpui_kit::{AppContext, TestAppContext};
use gpui_kit::test::TestWindowExt;
use mail_classifier::judge::{Answer, AnswerValue, QuestionKey, Suggestion};
use mail_classifier::model::{Mailbox, MessageId};
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
fn habit() -> Mailbox {
    mailbox(&[
        msg(1, 1, "s@x.io", "old", 1, "Archived"),
        msg(2, 2, "s@x.io", "old", 2, "Archived"),
        msg(3, 3, "s@x.io", "old", 3, "Archived"),
        msg(8, 8, "o@x.io", "other", 20, "Inbox"),
        msg(9, 9, "s@x.io", "new", 21, "Inbox"),
    ])
}

/// Tag `id` PossibleSpam through the classifier's own path.
fn tag_possible_spam(mb: &mut Mailbox, id: MessageId) {
    mb.add_suggestions(vec![Suggestion {
        message: id,
        key: QuestionKey::Spam,
        answer: Answer { probabilities: vec![], value: AnswerValue::Bool(true), confidence: 1.0 },
    }]);
    mb.accept_suggestions(id, 0);
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
fn skip_drops_the_message_from_the_session_without_a_state_change_or_undo_step(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, inbox());
    h.keys("t enter");
    let depth = h.read(|a| a.mailbox.undo_depth());
    h.keys("space");
    assert_eq!(h.read(|a| a.opened()), Some(3), "the next message is up");
    assert_eq!(h.read(|a| a.session_progress()), Some((1, 2)), "the queue no longer holds it");
    assert_eq!(h.state_of(4), Inbox);
    assert_eq!(h.read(|a| a.mailbox.undo_depth()), depth, "skipping is not an undo step");

    h.keys("e e");
    let s = h.read(|a| a.session_summary().copied()).expect("summary");
    assert_eq!((s.handled, s.tally.skipped, s.tally.archived), (2, 1, 2));
    assert_eq!(s.remaining, 1, "the skipped message is still in the Inbox");
}

#[gpui_kit::gpui::test]
fn skipping_every_message_finishes_with_a_summary_of_distinct_skips(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, inbox());
    h.keys("t enter");
    let depth = h.read(|a| a.mailbox.undo_depth());
    h.keys("space space space");
    let s = h.read(|a| a.session_summary().copied()).expect("all skipped: summary");
    assert_eq!((s.handled, s.tally.skipped, s.remaining), (0, 3, 3));
    assert_eq!(s.handled + s.tally.skipped, 3, "counts add up to the session size");
    assert_eq!(h.read(|a| a.session_progress()), None);
    assert_eq!(h.read(|a| a.mailbox.undo_depth()), depth, "skipping changes no mailbox state");
    assert_eq!(h.visible(), vec![4, 3, 2], "skipped mail stays in the Inbox");

    h.keys("escape");
    assert_eq!(h.visible(), vec![4, 3, 2]);
}

#[gpui_kit::gpui::test]
fn archived_and_filed_messages_leave_the_inbox_after_a_session(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, inbox());
    h.keys("t enter");
    h.keys("e");
    assert_eq!(h.state_of(4), Archived);
    h.keys("f enter");
    assert_eq!(h.state_of(3), Filed(1));
    h.keys("e");
    let s = h.read(|a| a.session_summary().copied()).expect("summary");
    assert_eq!((s.handled, s.tally.archived, s.tally.filed, s.remaining), (3, 2, 1, 0));

    h.keys("escape");
    assert!(h.visible().is_empty(), "the triaged mail no longer lists in the Inbox");
}

/// Sending a reply handles the message; filing the original afterwards must not also handle
/// the next one, or the session finishes with a message nobody decided still in the Inbox.
#[gpui_kit::gpui::test]
fn filing_the_original_after_a_reply_does_not_handle_the_next_message(cx: &mut TestAppContext) {
    let mut h = harness_with(
        cx,
        mailbox(&[msg(3, 3, "c@a.io", "three", 23, "Inbox"), msg(4, 4, "d@a.io", "four", 24, "Inbox")]),
    );
    h.keys("t enter");
    assert_eq!(h.read(|a| a.opened()), Some(4));
    h.send_reply("thanks");
    assert_eq!(h.read(|a| a.opened()), Some(3), "the reply handled message 4");
    h.keys("2");
    h.keys("enter");
    assert_eq!(h.state_of(4), Filed(1), "the original and the reply are filed");

    assert_eq!(h.read(|a| a.session_progress()), Some((2, 2)), "message 3 is still the session's own");
    assert_eq!(h.state_of(3), Inbox, "the post-send filing did not handle it");
    h.keys("e");
    let s = h.read(|a| a.session_summary().copied()).expect("summary");
    assert_eq!((s.handled, s.tally.replied, s.tally.archived, s.remaining), (2, 1, 1, 0));
    h.keys("escape");
    assert!(h.visible().is_empty(), "only decisions that were applied left the Inbox");
}

/// Deleting a spam message and then sweeping the sender's other mail must not handle the
/// session's next, unrelated message: it was never decided and must stay in the Inbox.
#[gpui_kit::gpui::test]
fn the_spam_sweep_after_a_delete_does_not_handle_the_next_message(cx: &mut TestAppContext) {
    let mut mb = mailbox(&[
        msg(2, 2, "s@x.io", "older", 22, "Inbox"),
        msg(3, 3, "o@x.io", "other", 23, "Inbox"),
        msg(4, 4, "s@x.io", "spam", 24, "Inbox"),
    ]);
    tag_possible_spam(&mut mb, 4);
    let mut h = harness_with(cx, mb);
    h.keys("t enter");
    assert_eq!(h.read(|a| a.opened()), Some(4));
    h.keys("d");
    h.keys("enter");
    assert_eq!(h.state_of(2), Deleted, "the sender's other mail is swept");
    h.keys("escape");
    assert_eq!(h.read(|a| a.opened()), Some(3), "message 3 is still the session's own");
    assert_eq!(h.state_of(3), Inbox);

    h.keys("d d");
    let s = h.read(|a| a.session_summary().copied()).expect("summary");
    assert_eq!((s.handled, s.tally.deleted, s.remaining), (3, 3, 0));
    h.keys("escape");
    assert!(h.visible().is_empty());
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

/// Sender `s@x.io` filed three earlier messages into folder 1; their newest message is in the
/// Inbox, alone, so taking the suggestion for it finishes the session.
fn filed_habit() -> Mailbox {
    let filed = |id: u32, day: u32| {
        msg(id, id, "s@x.io", "old", day, "Inbox")
            .replace(r#""state":"Inbox""#, r#""state":{"Filed":1}"#)
    };
    mailbox(&[filed(1, 1), filed(2, 2), filed(3, 3), msg(9, 9, "s@x.io", "new", 21, "Inbox")])
}

#[gpui_kit::gpui::test]
fn y_files_a_filed_history_suggestion_advances_the_session_and_undoes_once(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, filed_habit());
    h.keys("t enter");
    assert_eq!(h.read(|a| a.opened()), Some(9));
    let suggestion = h.read(|a| a.session_suggestion().cloned()).expect("history says file");
    assert_eq!(suggestion.action, SuggestedAction::File(1));

    let depth = h.read(|a| a.mailbox.undo_depth());
    h.keys("y");
    assert_eq!(h.state_of(9), Filed(1), "y takes the card's suggestion");
    assert_eq!(h.read(|a| a.mailbox.undo_depth()), depth + 1, "one undo step");
    assert_eq!(h.read(|a| a.session_progress()), None);
    let s = h.read(|a| a.session_summary().copied()).expect("the last message finishes the session");
    assert_eq!((s.handled, s.tally.filed, s.tally.suggestions_taken, s.remaining), (1, 1, 1, 0));

    h.keys("escape");
    h.keys("u");
    assert_eq!(h.state_of(9), Inbox, "one undo puts it back");
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

/// `s` in triage asks when to snooze, like the list; picking a time handles the current
/// message and moves on, cancelling changes nothing.
#[gpui_kit::gpui::test]
fn snoozing_from_triage_asks_when_and_advances_the_session(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, inbox());
    h.keys("t enter");
    assert_eq!(h.read(|a| a.opened()), Some(4));
    let until = h.read(|a| a.mailbox.snooze_presets(&a.local_now())[0].1);
    let depth = h.read(|a| a.mailbox.undo_depth());

    h.keys("s");
    assert!(h.read(|a| a.snooze_open()), "s asks when, as in the list");
    h.keys("escape");
    assert!(!h.read(|a| a.snooze_open()));
    assert_eq!(h.state_of(4), Inbox, "cancelling changes nothing");
    assert_eq!(h.read(|a| a.opened()), Some(4), "and keeps the message current");

    h.keys("s 1");
    assert_eq!(h.state_of(4), Snoozed);
    assert_eq!(h.read(|a| a.mailbox.snoozed_until(4)), Some(until));
    assert_eq!(h.read(|a| a.opened()), Some(3), "the session moved on");
    assert_eq!(h.read(|a| a.session_progress()), Some((2, 3)));
    assert_eq!(h.read(|a| a.mailbox.undo_depth()), depth + 1, "one undo step");

    h.keys("u");
    assert_eq!(h.state_of(4), Inbox);
    assert_eq!(h.read(|a| a.mailbox.snoozed_until(4)), None, "one undo restores the message");
}

/// Inside a session the undo offer lives in the top row, not a bottom toast over the toolbar.
/// Clicking it runs the same Undo `u` does and clears the inline offer.
#[gpui_kit::gpui::test]
fn archiving_in_a_session_undoes_inline_without_a_toast(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, inbox());
    h.keys("t enter");
    assert_eq!(h.read(|a| a.opened()), Some(4));

    h.keys("e");
    assert_eq!(h.state_of(4), Archived);
    assert_eq!(h.toast(), "", "no toast covers the triage toolbar");
    h.cx.update_window(h.window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("triage-undo").is_some(), "the top row offers Undo");
    }).expect("window alive");

    h.click("triage-undo");
    assert_eq!(h.state_of(4), Inbox, "Undo restores the message");
    assert!(h.read(|a| a.session_progress()).is_some(), "the session keeps running");
    h.cx.update_window(h.window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("triage-undo").is_none(), "the inline Undo clears");
    }).expect("window alive");
}

#[gpui_kit::gpui::test]
fn archiving_outside_a_session_still_shows_the_undo_toast(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, inbox());
    h.keys("e");
    assert!(h.toast().contains("Marked 1 archived"), "toast: {}", h.toast());
}
