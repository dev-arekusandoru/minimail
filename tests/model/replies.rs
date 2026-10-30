use crate::helpers::{State, T0, sample};
use mail_classifier::model::{Mailbox, MessageId, Tag};

fn delivered(mb: &mut Mailbox, original: MessageId, awaiting: bool) -> MessageId {
    let mut message = mb.get(original).unwrap().clone();
    message.body = "Confirmed reply".into();
    message.html = Some("<p>Confirmed reply</p>".into());
    message.to = message.from_email.clone();
    message.from_email = "me@example.com".into();
    mb.record_sent(message, Some(original), awaiting, T0)
}

#[test]
fn confirmed_reply_updates_thread_bookkeeping_and_retains_payload() {
    let mut mb = sample();
    mb.apply_auto(
        crate::helpers::sug(
            1,
            mail_classifier::judge::QuestionKey::NeedsReply,
            mail_classifier::judge::AnswerValue::Bool(true),
        ),
        T0,
    );
    let id = delivered(&mut mb, 1, true);
    assert!(mb.tags(1).contains(&Tag::AwaitingReply));
    assert!(!mb.tags(1).contains(&Tag::NeedsReply));
    assert_eq!(mb.awaiting_since(1), Some(T0));
    let message = mb.get(id).unwrap();
    assert_eq!(message.body, "Confirmed reply");
    assert_eq!(message.html.as_deref(), Some("<p>Confirmed reply</p>"));
    assert_eq!(message.thread_id, mb.get(1).unwrap().thread_id);
    assert_eq!(message.state, State::Inbox);
}

#[test]
fn filing_confirmed_reply_is_one_reversible_action_not_delivery_recall() {
    let mut mb = sample();
    let id = delivered(&mut mb, 1, false);
    assert_eq!(mb.file_after_reply(1, State::Archived), 2);
    assert_eq!(mb.state_of(1), Some(State::Archived));
    assert_eq!(mb.state_of(id), Some(State::Archived));
    assert!(mb.undo());
    assert_eq!(mb.state_of(1), Some(State::Inbox));
    assert_eq!(mb.state_of(id), Some(State::Inbox));
    assert!(!mb.undo(), "delivery is irreversible once confirmed");
    assert!(mb.get(id).unwrap().outgoing);
}

#[test]
fn confirmed_reply_inherits_current_filing_and_snooze_wake_time() {
    let mut mb = sample();
    mb.set_state(&[1], State::Deleted);
    let deleted = delivered(&mut mb, 1, false);
    assert_eq!(mb.state_of(deleted), Some(State::Deleted));
    mb.snooze(&[1], T0 + 100, T0);
    let snoozed = delivered(&mut mb, 1, false);
    assert_eq!(mb.state_of(snoozed), Some(State::Snoozed));
    assert_eq!(mb.snoozed_until(snoozed), Some(T0 + 100));
    assert!(mb.tick(T0 + 100).woken.contains(&snoozed));
    assert_eq!(mb.state_of(snoozed), Some(State::Inbox));
}

#[test]
fn archive_intent_files_the_thread_in_one_undo_step() {
    use mail_classifier::outgoing::SendIntent;
    let mut mb = sample();
    let id = delivered(&mut mb, 1, false);
    mb.apply_send_intent(1, SendIntent::Archive, T0);
    assert_eq!(mb.state_of(1), Some(State::Archived));
    assert_eq!(mb.state_of(id), Some(State::Archived));
    assert!(mb.undo());
    assert_eq!(mb.state_of(1), Some(State::Inbox));
    assert!(!mb.undo());
}

#[test]
fn remind_intent_resurfaces_after_its_days_unless_a_reply_arrives() {
    use mail_classifier::outgoing::SendIntent;
    let day = 86_400;
    let mut mb = sample();
    mb.set_state(&[1], State::Archived);
    delivered(&mut mb, 1, false);
    mb.apply_send_intent(1, SendIntent::RemindIfNoReply { days: 3 }, T0);
    assert!(mb.tags(1).contains(&Tag::AwaitingReply));
    let depth = mb.undo_depth();
    assert!(mb.tick(T0 + 2 * day).followed_up.is_empty(), "not due yet");
    assert_eq!(mb.tick(T0 + 3 * day).followed_up, vec![1]);
    assert_eq!(mb.state_of(1), Some(State::Inbox));
    assert!(mb.tags(1).contains(&Tag::FollowUp));
    assert_eq!(mb.undo_depth(), depth, "tick pushes no undo step");
}
