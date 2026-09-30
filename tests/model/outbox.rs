use mail_classifier::model::{OUTBOX_DELAY};
use crate::helpers::{sample, T0};
use crate::helpers::State;

#[test]
fn outbox_holds_reply_until_due_and_tick_flushes_once() {
    let mut mb = sample();
    mb.send_reply_at(1, "hi".into(), T0);
    assert_eq!(mb.state_of(1), Some(State::Waiting));
    assert_eq!(mb.waiting_since(1), Some(T0));
    assert_eq!(mb.outbox().len(), 1);
    assert_eq!(mb.outbox()[0].due, T0 + OUTBOX_DELAY);
    assert!(mb.sent().is_empty());

    assert_eq!(mb.tick(T0 + OUTBOX_DELAY - 1).flushed, 0);
    assert_eq!(mb.tick(T0 + OUTBOX_DELAY).flushed, 1);
    assert_eq!(mb.tick(T0 + OUTBOX_DELAY).flushed, 0);
    assert!(mb.outbox().is_empty());
    assert_eq!(mb.sent().len(), 1);
    assert_eq!(mb.sent()[0].body, "hi");
}

#[test]
fn undo_after_flush_reverts_state_only() {
    let mut mb = sample();
    mb.send_reply_at(1, "hi".into(), T0);
    mb.tick(T0 + 60);
    assert!(mb.undo());
    assert_eq!(mb.state_of(1), Some(State::Inbox));
    assert_eq!(mb.sent().len(), 1, "the sent mail already left");
}

#[test]
fn undo_before_flush_pulls_the_reply_back() {
    let mut mb = sample();
    mb.send_reply_at(1, "hi".into(), T0);
    assert!(mb.undo());
    assert!(mb.outbox().is_empty());
    assert_eq!(mb.state_of(1), Some(State::Inbox));
    assert_eq!(mb.tick(T0 + 60).flushed, 0);
    assert!(mb.sent().is_empty());
}

#[test]
fn recall_last_takes_the_newest_and_restores_prior_state() {
    let mut mb = sample();
    assert_eq!(mb.recall_last(T0), None);
    mb.send_reply_at(1, "first".into(), T0);
    mb.send_reply_at(3, "second".into(), T0 + 1);
    let r = mb.recall_last(T0 + 2).unwrap();
    assert_eq!((r.in_reply_to, r.body.as_str()), (3, "second"));
    assert_eq!(mb.state_of(3), Some(State::Later), "prior state restored");
    assert_eq!(mb.state_of(1), Some(State::Waiting));
    assert_eq!(mb.outbox().len(), 1);
    // Its undo step is consumed: undo now reverts the *first* send.
    assert!(mb.undo());
    assert_eq!(mb.state_of(1), Some(State::Inbox));
    assert!(mb.outbox().is_empty());
    assert!(!mb.undo());
}

#[test]
fn recall_keeps_unrelated_later_undo_steps_intact() {
    let mut mb = sample();
    mb.send_reply_at(1, "x".into(), T0);
    mb.set_state(&[2], State::Done);
    assert!(mb.recall_last(T0).is_some());
    assert_eq!(mb.state_of(2), Some(State::Done));
    assert!(mb.undo());
    assert_eq!(mb.state_of(2), Some(State::Inbox));
}

#[test]
fn immediate_send_reply_still_undoes_sent() {
    let mut mb = sample();
    mb.send_reply(1, "now".into());
    assert_eq!(mb.sent().len(), 1);
    assert!(mb.outbox().is_empty());
    assert!(mb.undo());
    assert!(mb.sent().is_empty());
    assert_eq!(mb.state_of(1), Some(State::Inbox));
}
