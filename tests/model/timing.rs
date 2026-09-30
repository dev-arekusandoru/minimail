use crate::helpers::{State, T0, threaded};
use mail_classifier::clock::{DAY, HOUR};
use mail_classifier::model::{Tag, TriageState};
#[test]
fn snooze_wakes_into_inbox_with_reminder_and_undo() {
    let mut mb = threaded(&[(1, 1, "a@x.test", "2026-09-01T00:00:00Z", State::Inbox)]);
    let until = T0 + HOUR;
    assert_eq!(mb.snooze(&[1], until, T0), 1);
    assert_eq!(mb.snoozed_until(1), Some(until));
    assert!(mb.tick(until - 1).woken.is_empty());
    assert_eq!(mb.tick(until).woken, vec![1]);
    assert_eq!(mb.state_of(1), Some(State::Inbox));
    assert!(mb.tags(1).contains(&Tag::Reminder));
    assert!(mb.undo());
    assert_eq!(mb.state_of(1), Some(State::Inbox));
    assert_eq!(mb.snoozed_until(1), None);
    assert!(!mb.tags(1).contains(&Tag::Reminder));
}
#[test]
fn snoozed_state_without_a_wake_seed_loads_as_inbox() {
    let json = r#"[{"id":1,"thread_id":1,"from_name":"A","from_email":"a@x.test","to":"you@example.com","subject":"s","body":"b","received":"2026-09-01T00:00:00Z","state":"Snoozed"}]"#;
    let mb = mail_classifier::model::Mailbox::from_json(json).unwrap();
    assert_eq!(mb.state_of(1), Some(State::Inbox));
    assert_eq!(mb.snoozed_until(1), None);
}
#[test]
fn follow_up_timeout_returns_original_to_inbox_and_clears_on_newer_incoming() {
    let mut mb = threaded(&[(1, 7, "a@x.test", "2026-09-01T00:00:00Z", State::Archived)]);
    mb.send_reply_at(1, "question".into(), true, T0);
    assert!(mb.tick(T0 + 3 * DAY - 1).followed_up.is_empty());
    assert_eq!(mb.tick(T0 + 3 * DAY).followed_up, vec![1]);
    assert_eq!(mb.state_of(1), Some(State::Inbox));
    assert!(mb.tags(1).contains(&Tag::FollowUp));
    let mut answered = threaded(&[
        (1, 7, "a@x.test", "2026-09-01T00:00:00Z", State::Archived),
        (2, 7, "a@x.test", "2026-10-02T00:00:00Z", State::Inbox),
    ]);
    answered.send_reply_at(1, "question".into(), true, T0);
    answered.tick(T0 + 4 * DAY);
    assert!(!answered.tags(1).contains(&Tag::AwaitingReply));
    assert!(!answered.tags(1).contains(&Tag::FollowUp));
}
#[test]
fn follow_up_timeout_can_be_configured() {
    let mut mb = threaded(&[(1, 1, "a@x.test", "2026-09-01T00:00:00Z", State::Inbox)]);
    mb.set_follow_up_timeout(HOUR);
    mb.send_reply_at(1, "x".into(), true, T0);
    assert!(mb.tick(T0 + HOUR - 1).followed_up.is_empty());
    assert_eq!(mb.tick(T0 + HOUR).followed_up, vec![1]);
}
#[test]
fn snoozed_fixture_requires_wake_seed() {
    let json = r#"[{"id":1,"thread_id":1,"from_name":"A","from_email":"a@x.test","to":"you@example.com","subject":"s","body":"b","received":"2026-09-01T00:00:00Z","state":"Snoozed","snooze":"2026-10-02T00:00:00Z"}]"#;
    let mb = mail_classifier::model::Mailbox::from_json(json).unwrap();
    assert_eq!(mb.state_of(1), Some(TriageState::Snoozed));
    assert_eq!(mb.snoozed_until(1), Some(T0 + DAY));
}
