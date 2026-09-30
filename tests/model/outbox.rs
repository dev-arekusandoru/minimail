use crate::helpers::{State, T0, sample};
use mail_classifier::model::{OUTBOX_DELAY, Tag};
#[test]
fn queued_reply_sets_awaiting_and_flushes_message() {
    let mut mb = sample();
    mb.apply_auto(
        crate::helpers::sug(
            1,
            mail_classifier::judge::QuestionKey::NeedsReply,
            mail_classifier::judge::AnswerValue::Bool(true),
        ),
        T0,
    );
    mb.send_reply_at(1, "reply".into(), true, T0);
    assert!(mb.tags(1).contains(&Tag::AwaitingReply));
    assert!(!mb.tags(1).contains(&Tag::NeedsReply));
    assert_eq!(mb.outbox()[0].due, T0 + OUTBOX_DELAY);
    assert_eq!(mb.tick(T0 + OUTBOX_DELAY).flushed, 1);
    let outgoing = mb.messages().iter().find(|m| m.outgoing).unwrap();
    assert_eq!(outgoing.state, State::Inbox);
    assert_eq!(outgoing.thread_id, mb.get(1).unwrap().thread_id);
}
#[test]
fn file_after_reply_updates_flushed_message_and_undoes_once() {
    let mut mb = sample();
    mb.send_reply_at(1, "reply".into(), false, T0);
    mb.tick(T0 + OUTBOX_DELAY);
    assert_eq!(mb.file_after_reply(1, State::Archived), 2);
    assert!(
        mb.messages()
            .iter()
            .filter(|m| m.outgoing)
            .all(|m| m.state == State::Archived)
    );
    assert!(mb.undo());
    assert_eq!(mb.state_of(1), Some(State::Inbox));
    assert!(
        mb.messages()
            .iter()
            .filter(|m| m.outgoing)
            .all(|m| m.state == State::Inbox)
    );
}
#[test]
fn pending_reply_inherits_file_state_at_flush() {
    let mut mb = sample();
    mb.send_reply_at(1, "reply".into(), false, T0);
    assert_eq!(mb.file_after_reply(1, State::Deleted), 1);
    mb.tick(T0 + OUTBOX_DELAY);
    assert!(mb.messages().iter().find(|m| m.outgoing).unwrap().state == State::Deleted);
}
#[test]
fn undo_send_retracts_queue_and_restores_tags() {
    let mut mb = sample();
    mb.apply_auto(
        crate::helpers::sug(
            1,
            mail_classifier::judge::QuestionKey::NeedsReply,
            mail_classifier::judge::AnswerValue::Bool(true),
        ),
        T0,
    );
    mb.send_reply_at(1, "reply".into(), true, T0);
    assert!(mb.undo());
    assert!(mb.outbox().is_empty());
    assert!(mb.tags(1).contains(&Tag::NeedsReply));
    assert!(!mb.tags(1).contains(&Tag::AwaitingReply));
}
#[test]
fn undo_send_after_flush_removes_sent_reply_and_materialised_message() {
    let mut mb = sample();
    mb.send_reply_at(1, "reply".into(), true, T0);
    assert_eq!(mb.tick(T0 + OUTBOX_DELAY).flushed, 1);
    assert_eq!(mb.sent().len(), 1);
    assert_eq!(mb.messages().iter().filter(|m| m.outgoing).count(), 1);

    assert!(mb.undo());
    assert!(mb.outbox().is_empty());
    assert!(mb.sent().is_empty());
    assert!(mb.messages().iter().all(|m| !m.outgoing));
    assert!(!mb.tags(1).contains(&Tag::AwaitingReply));
    assert_eq!(mb.awaiting_since(1), None);
}
