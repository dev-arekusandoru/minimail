use crate::helpers::{T0, sample, sug};
use mail_classifier::judge::{AnswerValue, Kind, QuestionKey};
use mail_classifier::model::Tag;
#[test]
fn suggestions_apply_tags_without_state_changes() {
    let mut mb = sample();
    let before = mb.state_of(1);
    mb.add_suggestions(vec![
        sug(1, QuestionKey::NeedsReply, AnswerValue::Bool(true)),
        sug(1, QuestionKey::Spam, AnswerValue::Bool(true)),
        sug(1, QuestionKey::Urgency, AnswerValue::Score(4.0)),
        sug(1, QuestionKey::Kind, AnswerValue::Choice(1)),
    ]);
    assert_eq!(mb.accept_suggestions(1, T0), 4);
    assert_eq!(mb.state_of(1), before);
    for tag in [
        Tag::NeedsReply,
        Tag::PossibleSpam,
        Tag::Urgent(4),
        Tag::Kind(Kind::Receipt),
    ] {
        assert!(mb.tags(1).contains(&tag));
    }
    assert!(mb.undo());
    assert_eq!(mb.state_of(1), before);
    assert_eq!(mb.pending(1).len(), 4);
}
#[test]
fn expects_reply_is_not_stored_as_suggestion() {
    let mut mb = sample();
    assert!(!mb.apply_auto(
        sug(1, QuestionKey::ExpectsReply, AnswerValue::Bool(true)),
        T0
    ));
    assert!(mb.tags(1).is_empty());
}
#[test]
fn urgency_and_kind_replace_prior_tag() {
    let mut mb = sample();
    mb.apply_auto(sug(1, QuestionKey::Urgency, AnswerValue::Score(2.0)), T0);
    mb.apply_auto(sug(1, QuestionKey::Urgency, AnswerValue::Score(8.0)), T0);
    assert!(mb.tags(1).contains(&Tag::Urgent(8)));
    assert!(!mb.tags(1).contains(&Tag::Urgent(2)));
}
