use mail_classifier::clock::{DAY};
use mail_classifier::judge::{AnswerValue, Kind, QuestionKey};
use mail_classifier::model::Tag;
use crate::helpers::{sample, threaded, sug, snapshot, T0};
use crate::helpers::State;

#[test]
fn accept_applies_each_kind_and_undoes_as_one_step() {
    let mut mb = sample();
    let before = snapshot(&mb);
    mb.add_suggestions(vec![
        sug(1, QuestionKey::SuggestedState, AnswerValue::Choice(2)),
        sug(1, QuestionKey::NeedsReply, AnswerValue::Bool(true)),
        sug(1, QuestionKey::Urgency, AnswerValue::Score(3.6)),
        sug(1, QuestionKey::Kind, AnswerValue::Choice(1)),
        sug(2, QuestionKey::Spam, AnswerValue::Bool(true)),
    ]);
    assert_eq!(mb.pending(1).len(), 4);
    assert_eq!(mb.accept_suggestions(1, T0), 4);
    assert!(mb.pending(1).is_empty());
    assert_eq!(mb.pending(2).len(), 1, "other messages untouched");
    assert_eq!(mb.state_of(1), Some(State::Later));
    let tags = mb.tags(1);
    assert!(tags.contains(&Tag::NeedsReply));
    assert!(tags.contains(&Tag::Urgent(4)));
    assert!(tags.contains(&Tag::Kind(Kind::Receipt)));
    assert_eq!(mb.accept_suggestions(1, T0), 0);

    assert!(mb.undo());
    assert_eq!(snapshot(&mb), before);
    assert_eq!(mb.pending(1).len(), 4, "undo restores the pending badges");
    assert!(!mb.undo(), "accept was exactly one step");

    assert_eq!(mb.accept_suggestions(2, T0), 1);
    assert_eq!(mb.state_of(2), Some(State::Done));
    assert_eq!(mb.tags(2), [Tag::Spam]);
}

#[test]
fn negative_answers_change_nothing_and_urgency_replaces() {
    let mut mb = sample();
    mb.add_suggestions(vec![
        sug(1, QuestionKey::Spam, AnswerValue::Bool(false)),
        sug(1, QuestionKey::Urgency, AnswerValue::Score(2.0)),
    ]);
    mb.accept_suggestions(1, T0);
    assert_eq!(mb.state_of(1), Some(State::Inbox));
    assert_eq!(mb.tags(1), [Tag::Urgent(2)]);
    assert!(mb.apply_auto(sug(1, QuestionKey::Urgency, AnswerValue::Score(5.0)), T0));
    assert_eq!(mb.tags(1), [Tag::Urgent(5)]);
    assert!(!mb.apply_auto(sug(1, QuestionKey::Urgency, AnswerValue::Score(5.0)), T0));
    assert!(mb.undo());
    assert_eq!(mb.tags(1), [Tag::Urgent(2)]);
}

#[test]
fn reject_drops_pending_and_undo_restores() {
    let mut mb = sample();
    mb.add_suggestions(vec![
        sug(1, QuestionKey::Spam, AnswerValue::Bool(true)),
        sug(2, QuestionKey::Spam, AnswerValue::Bool(true)),
    ]);
    assert_eq!(mb.reject_suggestions(1), 1);
    assert_eq!(mb.reject_suggestions(1), 0);
    assert!(mb.pending(1).is_empty());
    assert_eq!(mb.state_of(1), Some(State::Inbox));
    assert!(mb.undo());
    assert_eq!(mb.pending(1).len(), 1);
    assert_eq!(mb.pending(2).len(), 1);
}

#[test]
fn newer_suggestion_replaces_the_pending_one_for_same_question() {
    let mut mb = sample();
    mb.add_suggestions(vec![sug(1, QuestionKey::SuggestedState, AnswerValue::Choice(3))]);
    mb.add_suggestions(vec![sug(1, QuestionKey::SuggestedState, AnswerValue::Choice(2))]);
    assert_eq!(mb.pending(1).len(), 1);
    mb.accept_suggestions(1, T0);
    assert_eq!(mb.state_of(1), Some(State::Later));
}

#[test]
fn apply_auto_clears_matching_pending_and_undoes() {
    let mut mb = sample();
    mb.add_suggestions(vec![sug(1, QuestionKey::Spam, AnswerValue::Bool(true))]);
    assert!(mb.apply_auto(sug(1, QuestionKey::Spam, AnswerValue::Bool(true)), T0));
    assert!(mb.pending(1).is_empty());
    assert_eq!(mb.state_of(1), Some(State::Done));
    assert!(mb.undo());
    assert_eq!(mb.state_of(1), Some(State::Inbox));
    assert!(mb.tags(1).is_empty());
    assert_eq!(mb.pending(1).len(), 1);
    assert!(!mb.apply_auto(sug(99, QuestionKey::Spam, AnswerValue::Bool(true)), T0));
}

#[test]
fn out_of_range_choices_are_ignored() {
    let mut mb = sample();
    assert!(!mb.apply_auto(sug(1, QuestionKey::SuggestedState, AnswerValue::Choice(9)), T0));
    assert!(!mb.apply_auto(sug(1, QuestionKey::Kind, AnswerValue::Choice(9)), T0));
    assert!(!mb.undo());
}

#[test]
fn suggested_waiting_resurfaces_after_three_days() {
    let mut mb = threaded(&[
        (1, 1, "a@x.test", "2026-09-01T00:00:00Z", State::Inbox),
        (2, 2, "a@x.test", "2026-09-01T00:00:00Z", State::Inbox),
    ]);
    mb.add_suggestions(vec![sug(1, QuestionKey::SuggestedState, AnswerValue::Choice(1))]);
    mb.accept_suggestions(1, T0);
    assert!(mb.apply_auto(sug(2, QuestionKey::SuggestedState, AnswerValue::Choice(1)), T0));
    assert_eq!(mb.waiting_since(1), Some(T0));
    assert_eq!(mb.waiting_since(2), Some(T0));
    assert!(mb.tick(T0 + 3 * DAY - 1).resurfaced.is_empty());
    assert_eq!(mb.tick(T0 + 3 * DAY).resurfaced.len(), 2);
    assert_eq!(mb.state_of(1), Some(State::Inbox));
    assert!(mb.tags(2).contains(&Tag::NoReply));
}
