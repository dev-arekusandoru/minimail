use crate::helpers::{T0, sample, sug};
use mail_classifier::judge::{AnswerValue, Kind, QuestionKey};
use mail_classifier::model::{Tag, TriageState};
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

#[test]
fn accepting_a_folder_suggestion_files_the_message() {
    let mut mb = sample();
    let options = mb.folder_options("personal");
    assert!(!options.is_empty(), "the fixture account has folders");
    let target = options[0].0;
    let mut s = sug(1, QuestionKey::Folder, AnswerValue::Choice(0));
    s.answer.probabilities = vec![1.0; options.len() + 1];
    mb.add_suggestions(vec![s]);
    assert_eq!(mb.accept_suggestions(1, T0), 1);
    assert_eq!(mb.state_of(1), Some(TriageState::Filed(target)));
    // One undo step reverts the filing.
    assert!(mb.undo());
    assert_eq!(mb.state_of(1), Some(TriageState::Inbox));
}

#[test]
fn a_hand_set_label_sticks_against_reclassification() {
    let mut mb = sample();
    assert!(mb.set_label(1, QuestionKey::Kind, Some(Tag::Kind(Kind::Newsletter))));
    assert!(mb.is_overridden(1, QuestionKey::Kind));
    assert!(mb.tags(1).contains(&Tag::Kind(Kind::Newsletter)));

    // A later classification never overwrites it.
    assert!(!mb.apply_auto(sug(1, QuestionKey::Kind, AnswerValue::Choice(1)), T0));
    assert!(mb.tags(1).contains(&Tag::Kind(Kind::Newsletter)));
    assert!(!mb.tags(1).contains(&Tag::Kind(Kind::Receipt)));
    // Other questions are untouched by the override.
    assert!(mb.apply_auto(sug(1, QuestionKey::Spam, AnswerValue::Bool(true)), T0));
    assert!(mb.tags(1).contains(&Tag::PossibleSpam));
}

#[test]
fn setting_a_label_is_one_undo_step() {
    let mut mb = sample();
    assert!(mb.set_label(1, QuestionKey::Urgency, Some(Tag::Urgent(5))));
    assert!(mb.set_label(1, QuestionKey::Spam, Some(Tag::PossibleSpam)));
    assert!(mb.undo(), "the spam label reverts");
    assert!(!mb.tags(1).contains(&Tag::PossibleSpam));
    assert!(mb.tags(1).contains(&Tag::Urgent(5)), "the urgency label stays");
    assert!(!mb.is_overridden(1, QuestionKey::Spam));
    assert!(mb.is_overridden(1, QuestionKey::Urgency));
    assert!(mb.undo());
    assert!(!mb.is_overridden(1, QuestionKey::Urgency));
    assert!(mb.tags(1).is_empty());
}

#[test]
fn clearing_a_label_is_sticky_too() {
    let mut mb = sample();
    mb.apply_auto(sug(1, QuestionKey::Spam, AnswerValue::Bool(true)), T0);
    assert!(mb.tags(1).contains(&Tag::PossibleSpam));
    assert!(mb.set_label(1, QuestionKey::Spam, None));
    assert!(!mb.tags(1).contains(&Tag::PossibleSpam));
    assert!(!mb.apply_auto(sug(1, QuestionKey::Spam, AnswerValue::Bool(true)), T0));
    assert!(mb.tags(1).is_empty(), "cleared by hand, so it stays cleared");
}

#[test]
fn resetting_a_label_puts_the_classifier_value_back() {
    let mut mb = sample();
    assert!(mb.set_label(1, QuestionKey::Kind, Some(Tag::Kind(Kind::Person))));
    assert!(mb.reset_label(1, QuestionKey::Kind, Some(Tag::Kind(Kind::Receipt))));
    assert!(!mb.is_overridden(1, QuestionKey::Kind));
    assert!(mb.tags(1).contains(&Tag::Kind(Kind::Receipt)));
    assert!(!mb.tags(1).contains(&Tag::Kind(Kind::Person)));

    // Back to sticky: the classifier can label it again.
    assert!(mb.apply_auto(sug(1, QuestionKey::Kind, AnswerValue::Choice(2)), T0));
    assert!(mb.tags(1).contains(&Tag::Kind(Kind::Newsletter)));

    // One undo step restores the hand-set value.
    assert!(mb.undo());
    assert!(mb.tags(1).contains(&Tag::Kind(Kind::Person)));
    assert!(mb.is_overridden(1, QuestionKey::Kind));
}

#[test]
fn resetting_a_label_the_user_never_set_changes_nothing() {
    let mut mb = sample();
    assert!(!mb.reset_label(1, QuestionKey::Kind, Some(Tag::Kind(Kind::Receipt))));
    assert!(mb.tags(1).is_empty());
    assert!(!mb.set_label(99, QuestionKey::Kind, Some(Tag::Kind(Kind::Person))), "no such message");
}
