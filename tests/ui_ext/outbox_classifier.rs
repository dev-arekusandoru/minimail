use gpui_kit::{TestAppContext};
use mail_classifier::model::{MessageId, Tag};
use mail_classifier::model::TriageState::*;
use crate::harness::{Harness, harness, harness_with};
use crate::snooze::snooze_box;

// ---------------------------------------------------------------- Undo send / outbox

#[gpui_kit::gpui::test]
pub fn send_goes_to_outbox_and_undo_recalls_within_window(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, snooze_box());
    let id = h.cursor().unwrap();
    h.send_reply("Does that work?");
    // Sending opens the post-send dialog; cancel it to keep the original where it is.
    assert!(h.read(|a| a.dialog_open()));
    h.keys("escape");
    assert_eq!(h.read(|a| a.mailbox.outbox().len()), 1);
    assert_eq!(h.read(|a| a.mailbox.sent().len()), 0);
    assert_eq!(h.state_of(id), Inbox, "sending does not move the original out of the Inbox");
    assert!(h.has_tag(id, Tag::AwaitingReply));
    assert!(!h.has_tag(id, Tag::NeedsReply));

    h.advance(9);
    assert_eq!(h.read(|a| a.mailbox.outbox().len()), 1, "still recallable at 9s");
    h.keys("u");
    assert!(h.read(|a| a.compose_open()), "recall reopens compose");
    assert_eq!(h.read(|a| a.mailbox.outbox().len()), 0);
    assert_eq!(h.read(|a| a.mailbox.sent().len()), 0);
    assert_eq!(h.state_of(id), Inbox);
    h.keys("escape");
    h.assert_invariant("recall");
}

#[gpui_kit::gpui::test]
pub fn outbox_flushes_to_sent_after_ten_seconds(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, snooze_box());
    let id = h.cursor().unwrap();
    h.send_reply("bye");
    h.keys("escape");
    h.advance(9);
    assert_eq!(h.read(|a| a.mailbox.sent().len()), 0);
    h.advance(1);
    assert_eq!(h.read(|a| a.mailbox.outbox().len()), 0);
    let sent = h.read(|a| a.mailbox.sent().to_vec());
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].in_reply_to, id);
    assert!(sent[0].body.contains("bye"));

    // Undo cancels the originating send, including the materialised sent message.
    h.keys("u");
    assert!(!h.read(|a| a.compose_open()));
    assert_eq!(h.state_of(id), Inbox);
    assert!(h.read(|a| a.mailbox.sent().is_empty()));
    assert!(h.read(|a| a.mailbox.messages().iter().all(|m| !m.outgoing)));
}

// ---------------------------------------------------------------- Classifier

pub fn tag_total(h: &mut Harness<'_>) -> usize {
    h.read(|a| a.mailbox.messages().iter().map(|m| a.mailbox.tags(m.id).len()).sum())
}

pub fn pending_total(h: &mut Harness<'_>) -> usize {
    h.read(|a| a.mailbox.messages().iter().map(|m| a.mailbox.pending(m.id).len()).sum())
}

/// Move the cursor down the current view until a message with pending suggestions is found.
pub fn goto_pending(h: &mut Harness<'_>) -> MessageId {
    h.click("nav-all-inboxes");
    for _ in 0..80 {
        let id = h.cursor().unwrap();
        if h.read(|a| !a.mailbox.pending(id).is_empty()) {
            return id;
        }
        h.keys("j");
    }
    panic!("no message with pending suggestions in the inbox");
}

#[gpui_kit::gpui::test]
pub fn classifier_runs_on_startup_and_c_reruns(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    assert!(
        tag_total(&mut h) + pending_total(&mut h) > 0,
        "startup classification produces badges"
    );
    h.keys("c");
    assert!(h.toast().contains("auto-applied"), "toast was {:?}", h.toast());
    assert!(h.toast().contains("to review"));
    h.assert_invariant("classify");
}

#[gpui_kit::gpui::test]
pub fn accept_applies_and_reject_discards_pending_suggestions(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let id = goto_pending(&mut h);
    let before_state = h.state_of(id);
    let tags_before = h.read(|a| a.mailbox.tags(id).len());
    let suggestions: Vec<_> = h.read(|a| {
        a.mailbox.pending(id).iter().map(|s| (s.key, format!("{:?}", s.answer.value))).collect()
    });
    assert!(!suggestions.is_empty());

    h.keys("y");
    assert!(h.read(|a| a.mailbox.pending(id).is_empty()), "accept clears the pending badges");
    let after_state = h.state_of(id);
    let tags_after = h.read(|a| a.mailbox.tags(id).len());
    assert!(
        after_state != before_state || tags_after > tags_before,
        "accepting must apply something ({suggestions:?})"
    );
    h.assert_invariant("accept");

    // One undo step brings the pending badges back.
    h.keys("u");
    assert!(!h.read(|a| a.mailbox.pending(id).is_empty()));
    assert_eq!(h.state_of(id), before_state);
    assert_eq!(h.read(|a| a.mailbox.tags(id).len()), tags_before);

    // Reject discards without applying.
    h.keys("n");
    assert!(h.read(|a| a.mailbox.pending(id).is_empty()));
    assert_eq!(h.state_of(id), before_state);
    assert_eq!(h.read(|a| a.mailbox.tags(id).len()), tags_before);
}

#[gpui_kit::gpui::test]
pub fn auto_applied_labels_can_be_undone(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let tags = tag_total(&mut h);
    assert!(tags > 0, "startup auto-applies at least one label");
    // Undo repeatedly until the auto-applied labels are gone; each undo removes a step, none adds.
    let mut last = tags;
    let mut dropped = false;
    for _ in 0..200 {
        h.keys("u");
        let now = tag_total(&mut h);
        assert!(now <= last, "undo never adds labels here");
        if now < last {
            dropped = true;
            break;
        }
        last = now;
    }
    assert!(dropped, "undo eventually removes an auto-applied label");
    h.assert_invariant("undo auto");
}
