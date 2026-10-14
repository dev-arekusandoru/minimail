use gpui_kit::{TestAppContext};
use mail_classifier::judge::{Answer, AnswerValue, QuestionKey, Suggestion};
use mail_classifier::model::{MessageId, Tag};
use mail_classifier::model::TriageState::*;
use crate::harness::{Harness, harness, harness_with};
use crate::misc::spam_box;
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

#[gpui_kit::gpui::test]
pub fn classifier_runs_on_startup(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    assert!(
        tag_total(&mut h) + pending_total(&mut h) > 0,
        "startup classification produces badges"
    );
    h.assert_invariant("classify");
}

/// A message with a folder suggestion waiting, seeded by hand: the offline stub never files
/// mail, so it is the only way a suggestion pends.
fn seed_folder_suggestion(h: &mut Harness<'_>) -> MessageId {
    let id = h.read(|a| a.visible_ids()[0]);
    h.app.update(h.cx, |a, _| {
        let options = a.mailbox.folder_options("personal");
        a.mailbox.add_suggestions(vec![Suggestion {
            message: id,
            key: QuestionKey::Folder,
            answer: Answer {
                probabilities: vec![1.0; options.len() + 1],
                value: AnswerValue::Choice(0),
                confidence: 0.9,
            },
        }]);
    });
    id
}

#[gpui_kit::gpui::test]
pub fn accept_applies_and_reject_discards_pending_suggestions(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let id = seed_folder_suggestion(&mut h);
    let before_state = h.state_of(id);
    let tags_before = h.read(|a| a.mailbox.tags(id).len());
    assert!(!h.read(|a| a.mailbox.pending(id).is_empty()));

    h.keys("y");
    assert!(h.read(|a| a.mailbox.pending(id).is_empty()), "accept clears the pending badges");
    assert_ne!(h.state_of(id), before_state, "accepting the folder suggestion files it");
    h.assert_invariant("accept");

    // One undo step brings the pending badge back.
    h.keys("u");
    assert!(!h.read(|a| a.mailbox.pending(id).is_empty()));
    assert_eq!(h.state_of(id), before_state);

    // Reject discards without applying.
    h.keys("n");
    assert!(h.read(|a| a.mailbox.pending(id).is_empty()));
    assert_eq!(h.state_of(id), before_state);
    assert_eq!(h.read(|a| a.mailbox.tags(id).len()), tags_before);
}

#[gpui_kit::gpui::test]
pub fn background_labels_are_not_undoable(cx: &mut TestAppContext) {
    // spam_box's message 1 is auto-applied `PossibleSpam` during startup classification.
    let mut h = harness_with(cx, spam_box());
    assert!(h.has_tag(1, Tag::PossibleSpam), "startup auto-applies a label");

    // The user's own action: archive the cursor message.
    let id = h.cursor().unwrap();
    h.keys("e");
    assert_eq!(h.state_of(id), Archived, "the user archived the message");

    // `u` undoes the archive; the background label was never on the undo stack.
    h.keys("u");
    assert_eq!(h.state_of(id), Inbox, "u undoes the user's action");
    assert!(h.has_tag(1, Tag::PossibleSpam), "u does not undo the background label");
    h.assert_invariant("undo background label");
}
