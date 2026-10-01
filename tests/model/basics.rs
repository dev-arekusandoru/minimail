use crate::helpers::{State, assert_invariant, mailbox, sample};
use mail_classifier::model::{Location, Mailbox};
use mail_classifier::search::Query;

#[test]
fn set_state_and_sender_scope_are_undoable() {
    let mut mb = sample();
    assert_eq!(
        mb.set_state_for_sender("a@x.test", State::Inbox, State::Archived),
        1
    );
    assert_eq!(mb.state_of(1), Some(State::Archived));
    assert_eq!(mb.state_of(3), Some(State::Archived));
    assert!(mb.undo());
    assert_eq!(mb.state_of(1), Some(State::Inbox));
    assert_eq!(mb.state_of(3), Some(State::Archived));
    assert_invariant(&mb);
}
#[test]
fn sender_set_moves_only_messages_in_the_source_state() {
    let mut mb = sample();
    assert_eq!(mb.set_state_for_sender("a@x.test", State::Inbox, State::Deleted), 1);
    assert_eq!(mb.state_of(3), Some(State::Archived));
    assert_eq!(mb.state_of(5), Some(State::Deleted));
    assert_eq!(mb.set_state_for_sender("a@x.test", State::Archived, State::Inbox), 1);
    assert_eq!(mb.state_of(3), Some(State::Inbox));
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
fn a_location_query_and_its_count_select_the_same_mail() {
    const NOW: mail_classifier::clock::Timestamp = 1_790_000_000;
    let mut mb = sample();
    mb.apply_auto(
        crate::helpers::sug(
            1,
            mail_classifier::judge::QuestionKey::NeedsReply,
            mail_classifier::judge::AnswerValue::Bool(true),
        ),
        0,
    );
    let inbox = mb.location_query(&Location::Inbox("personal".into()));
    assert!(mb.ids_matching(&inbox, NOW).contains(&1));
    assert_eq!(mb.count_at(&Location::Archive("personal".into()), NOW), 1);
    assert_eq!(mb.count_at(&Location::Folder(1), NOW), 0);
    assert_eq!(mb.query_location(&inbox), Some(Location::Inbox("personal".into())));
}
#[test]
fn fixture_invariant_and_accounts() {
    let mb = Mailbox::load_default();
    assert_eq!(mb.accounts().len(), 2);
    assert!(mb.folders("personal").len() >= 3);
    assert_invariant(&mb);
}
#[test]
fn fixture_covers_every_triage_state_with_real_folders_and_wake_times() {
    let mb = Mailbox::load_default();
    for state in [State::Inbox, State::Snoozed, State::Archived, State::Deleted] {
        assert!(
            mb.messages().iter().any(|m| m.state == state),
            "no fixture message is {state:?}"
        );
    }
    let filed: Vec<_> = mb
        .messages()
        .iter()
        .filter_map(|m| match m.state {
            State::Filed(folder) => Some((m.account.as_str(), folder)),
            _ => None,
        })
        .collect();
    assert!(filed.len() >= 2, "filed fixtures: {filed:?}");
    for (account, folder) in filed {
        assert!(
            mb.folders(account).iter().any(|f| f.id == folder),
            "folder {folder} is not one of {account}'s folders"
        );
    }
    for m in mb.messages().iter().filter(|m| m.state == State::Snoozed) {
        assert!(mb.snoozed_until(m.id).is_some(), "snoozed message {} has no wake time", m.id);
    }
    assert!(mb.messages().iter().any(|m| m.outgoing && !m.bcc.is_empty()), "no sent mail with a Bcc");
}
#[test]
fn empty_mailbox_views_are_empty() {
    let mb = mailbox(&[]);
    assert!(mb.ids_matching(&Query::default(), 0).is_empty());
}
