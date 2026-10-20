//! Triage mode's pure logic: queue building, tallies, skipping, summary, and the stub suggestion.

use mail_classifier::judge::{Answer, AnswerValue, QuestionKey, Suggestion};
use mail_classifier::model::{Location, Mailbox, TriageState};
use mail_classifier::triage_session::{
    Outcome, Run, SessionFilter, SessionOptions, SessionOrder, SessionSize, SetupView, build_queue,
};
use mail_classifier::triage_suggest::{SuggestedAction, suggest};
use mail_classifier::tz::{Now, Utc};
use std::rc::Rc;

const T0: i64 = 1_790_812_800;

fn now() -> Now {
    Now::new(T0, Rc::new(Utc))
}

/// `(id, sender, state)`; message `n` was received on day `n` of September 2026.
fn mailbox(specs: &[(u32, &str, TriageState)]) -> Mailbox {
    let values: Vec<_> = specs
        .iter()
        .map(|(id, from, state)| {
            serde_json::json!({"id":id,"thread_id":id,"from_name":from,"from_email":from,"to":"you@example.com",
                "subject":format!("m{id}"),"body":"body","received":format!("2026-09-{id:02}T09:00:00Z"),
                "state":state,"account":"personal"})
        })
        .collect();
    Mailbox::from_json(&serde_json::to_string(&values).unwrap()).unwrap()
}

fn tag(mb: &mut Mailbox, id: u32, key: QuestionKey, value: AnswerValue) {
    let answer = Answer { probabilities: vec![], value, confidence: 1.0 };
    assert!(mb.apply_auto(Suggestion { message: id, key, answer }, T0));
}

fn options() -> SessionOptions {
    SessionOptions::default()
}

fn inbox(n: u32) -> Vec<(u32, &'static str, TriageState)> {
    (1..=n).map(|i| (i, "a@x.io", TriageState::Inbox)).collect()
}

#[test]
fn the_queue_is_the_inbox_newest_first_by_default() {
    let mb = mailbox(&[(1, "a", TriageState::Inbox), (2, "a", TriageState::Archived), (3, "a", TriageState::Inbox)]);
    assert_eq!(build_queue(&mb, &options(), &now()), vec![3, 1]);
}

#[test]
fn order_and_size_shape_the_queue() {
    let mb = mailbox(&inbox(30));
    let oldest = SessionOptions { order: SessionOrder::OldestFirst, size: SessionSize::N25, ..options() };
    let ids = build_queue(&mb, &oldest, &now());
    assert_eq!((ids.len(), ids[0], ids[24]), (25, 1, 25), "the oldest 25, oldest first");
    let newest = SessionOptions { size: SessionSize::N25, ..options() };
    let ids = build_queue(&mb, &newest, &now());
    assert_eq!((ids[0], ids[24]), (30, 6), "the newest 25, newest first");
    assert_eq!(build_queue(&mb, &options(), &now()).len(), 30);
}

#[test]
fn the_filter_narrows_the_queue_and_chips_count_the_location() {
    let mut mb = mailbox(&inbox(4));
    tag(&mut mb, 2, QuestionKey::NeedsReply, AnswerValue::Bool(true));
    tag(&mut mb, 3, QuestionKey::NeedsReply, AnswerValue::Bool(true));
    tag(&mut mb, 3, QuestionKey::Urgency, AnswerValue::Score(4.0));
    tag(&mut mb, 4, QuestionKey::Spam, AnswerValue::Bool(true));
    let needs = SessionOptions { filter: SessionFilter::NeedsReply, ..options() };
    assert_eq!(build_queue(&mb, &needs, &now()), vec![3, 2]);
    let view = SetupView::new(&mb, needs, &now());
    assert_eq!((view.total, view.queue_len), (4, 2));
    let counts: Vec<usize> = view.filter_counts.iter().map(|(_, n)| *n).collect();
    assert_eq!(counts, vec![4, 2, 0, 1, 1]);
    let spam = SessionOptions { filter: SessionFilter::PossibleSpam, ..options() };
    assert_eq!(build_queue(&mb, &spam, &now()), vec![4]);
}

#[test]
fn a_location_scopes_the_queue() {
    let mb = mailbox(&inbox(3));
    let other = SessionOptions { location: Location::Inbox("work".to_owned()), ..options() };
    assert!(build_queue(&mb, &other, &now()).is_empty());
    let own = SessionOptions { location: Location::Inbox("personal".to_owned()), ..options() };
    assert_eq!(build_queue(&mb, &own, &now()).len(), 3);
}

#[test]
fn each_action_counts_once_and_advances() {
    let mut run = Run::new(options(), vec![5, 4, 3, 2, 1], 100);
    for outcome in [Outcome::Archived, Outcome::Filed, Outcome::Snoozed, Outcome::Deleted, Outcome::Replied] {
        run.handle(outcome, false);
    }
    assert!(run.is_finished());
    run.handle(Outcome::Archived, false);
    let s = run.summary(160, 7);
    assert_eq!((s.handled, s.elapsed_secs, s.remaining, s.avg_secs()), (5, 60, 7, 12));
    let t = s.tally;
    assert_eq!((t.archived, t.filed, t.snoozed, t.deleted, t.replied, t.skipped), (1, 1, 1, 1, 1, 0));
}

#[test]
fn skipping_removes_the_message_without_handling_it_and_never_brings_it_back() {
    let mut run = Run::new(options(), vec![3, 2, 1], 0);
    assert_eq!((run.current(), run.progress()), (Some(3), (1, 3)));
    run.skip();
    assert_eq!((run.current(), run.progress(), run.handled(), run.skipped()), (Some(2), (1, 2), 0, 1));
    run.handle(Outcome::Archived, true);
    run.handle(Outcome::Archived, false);
    assert!(run.is_finished(), "the skipped message does not come round again");
    let s = run.summary(10, 0);
    assert_eq!((s.handled, s.tally.skipped), (2, 1));
    let t = s.tally;
    assert_eq!((t.archived, t.deleted, t.suggestions_taken), (2, 0, 1));
    assert_eq!(s.handled + t.skipped, 3, "handled and skipped add up to the session size");
}

#[test]
fn skipping_every_message_finishes_the_session_and_counts_each_once() {
    let mut run = Run::new(options(), vec![3, 2, 1], 0);
    for _ in 0..3 {
        run.skip();
    }
    assert!(run.is_finished());
    assert_eq!(run.current(), None);
    let s = run.summary(10, 3);
    assert_eq!((s.handled, s.tally.skipped), (0, 3));
    assert_eq!(s.handled + s.tally.skipped, 3);
    assert_eq!(run.summary(10, 3).tally.skipped, 3, "no message is skipped twice");
}

#[test]
fn other_outcomes_are_handled_without_a_tally() {
    let mut run = Run::new(options(), vec![1], 0);
    run.handle(Outcome::of(TriageState::Inbox), false);
    let s = run.summary(0, 0);
    assert_eq!(s.handled, 1);
    assert_eq!(s.tally, Default::default());
}

fn history(archived: u32, others: &[TriageState]) -> Vec<(u32, &'static str, TriageState)> {
    let mut specs = vec![(1, "s@x.io", TriageState::Inbox)];
    for (i, state) in std::iter::repeat_n(TriageState::Archived, archived as usize).chain(others.iter().copied()).enumerate() {
        specs.push((10 + i as u32, "s@x.io", state));
    }
    specs
}

#[test]
fn a_sender_you_always_archive_suggests_archive() {
    let mb = mailbox(&history(9, &[]));
    let s = suggest(&mb, 1).expect("suggestion");
    assert_eq!(s.action, SuggestedAction::Archive);
    assert_eq!(s.reason, "you've archived 9 of 9 from this sender");
}

#[test]
fn mostly_archived_still_suggests_and_too_little_history_does_not() {
    let mb = mailbox(&history(4, &[TriageState::Deleted]));
    assert_eq!(suggest(&mb, 1).unwrap().reason, "you've archived 4 of 5 from this sender");
    assert_eq!(suggest(&mailbox(&history(2, &[])), 1), None, "two earlier messages are not a habit");
    assert_eq!(suggest(&mailbox(&history(2, &[TriageState::Deleted, TriageState::Deleted])), 1), None, "no majority");
    assert_eq!(suggest(&mailbox(&history(0, &[])), 1), None);
}

#[test]
fn undecided_mail_is_not_history() {
    let mb = mailbox(&history(2, &[TriageState::Inbox, TriageState::Inbox, TriageState::Snoozed]));
    assert_eq!(suggest(&mb, 1), None);
}

#[test]
fn possible_spam_suggests_delete() {
    let mut mb = mailbox(&inbox(2));
    tag(&mut mb, 1, QuestionKey::Spam, AnswerValue::Bool(true));
    assert_eq!(suggest(&mb, 1).unwrap().action, SuggestedAction::Delete);
    assert_eq!(suggest(&mb, 2), None);
}

#[test]
fn a_sender_you_file_to_one_folder_suggests_that_folder() {
    let mut specs = history(0, &[]);
    specs.extend((0..4).map(|i| (20 + i, "s@x.io", TriageState::Archived)));
    let mut mb = mailbox(&specs);
    let folder = mb.create_folder("personal", "Receipts", None);
    mb.set_state(&[20, 21, 22, 23], TriageState::Filed(folder));
    let s = suggest(&mb, 1).expect("suggestion");
    assert_eq!(s.action, SuggestedAction::File(folder));
    assert_eq!(s.reason, "you've filed 4 of 4 from this sender to Receipts");
    let other = mb.create_folder("personal", "Other", None);
    mb.set_state(&[20, 21], TriageState::Filed(other));
    assert_eq!(suggest(&mb, 1), None, "split between folders: no advice");
}
