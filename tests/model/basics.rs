use crate::helpers::{State, assert_invariant, mailbox, sample};
use mail_classifier::model::{Location, Mailbox, View};

#[test]
fn set_state_and_sender_scope_are_undoable() {
    let mut mb = sample();
    assert_eq!(
        mb.set_state_for_sender("a@x.test", true, State::Archived),
        1
    );
    assert_eq!(mb.state_of(1), Some(State::Archived));
    assert_eq!(mb.state_of(3), Some(State::Archived));
    assert!(mb.undo());
    assert_eq!(mb.state_of(1), Some(State::Inbox));
    assert_eq!(mb.state_of(3), Some(State::Archived));
    assert_eq!(
        mb.set_state_for_sender("a@x.test", false, State::Deleted),
        2
    );
    assert_eq!(mb.state_of(3), Some(State::Deleted));
    assert_invariant(&mb);
}
#[test]
fn sender_set_moves_only_inbox_when_requested() {
    let mut mb = sample();
    assert_eq!(mb.set_state_for_sender("a@x.test", true, State::Deleted), 1);
    assert_eq!(mb.state_of(3), Some(State::Archived));
    assert_eq!(mb.state_of(5), Some(State::Deleted));
}
#[test]
fn folders_are_seeded_nested_and_create_undoes() {
    let mut mb = sample();
    let f = mb.folders("work");
    let parent = f.iter().find(|x| x.name == "Projects").unwrap();
    assert!(f.iter().any(|x| x.parent == Some(parent.id)));
    let id = mb.create_folder("personal", "Notes", None);
    assert_eq!(mb.folder(id).unwrap().name, "Notes");
    assert!(mb.undo());
    assert!(mb.folder(id).is_none());
}
#[test]
fn location_chip_filter_and_account_selection() {
    let mut mb = sample();
    mb.apply_auto(
        crate::helpers::sug(
            1,
            mail_classifier::judge::QuestionKey::NeedsReply,
            mail_classifier::judge::AnswerValue::Bool(true),
        ),
        0,
    );
    let v = View {
        location: Location::Inbox("personal".into()),
        ..View::default()
    };
    assert!(mb.ids_in_view(&v).contains(&1));
    assert_eq!(mb.count_at(&Location::Archive("personal".into())), 1);
    assert_eq!(mb.count_at(&Location::Folder(1)), 0);
}
#[test]
fn fixture_invariant_and_accounts() {
    let mb = Mailbox::load_default();
    assert_eq!(mb.messages().len(), 60);
    assert_eq!(mb.accounts().len(), 2);
    assert!(mb.folders("personal").len() >= 3);
    assert_invariant(&mb);
}
#[test]
fn empty_mailbox_views_are_empty() {
    let mb = mailbox(&[]);
    assert!(mb.ids_in_view(&View::default()).is_empty());
}
