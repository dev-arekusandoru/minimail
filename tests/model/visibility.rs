use crate::helpers::{State, sample};
use mail_classifier::model::{Chip, Filter, Location, TagFilter, View};
#[test]
fn block_move_is_one_undo_step_and_unblock_is_undoable() {
    let mut mb = sample();
    assert_eq!(mb.block_sender("a@x.test", Some(State::Archived)), 1);
    assert_eq!(mb.state_of(1), Some(State::Archived));
    assert_eq!(mb.state_of(3), Some(State::Archived));
    assert_eq!(mb.blocked(), vec!["a@x.test"]);
    assert!(mb.undo());
    assert_eq!(mb.state_of(1), Some(State::Inbox));
    assert!(mb.blocked().is_empty());
    assert_eq!(mb.block_sender("a@x.test", None), 0);
    assert!(mb.unblock_sender("a@x.test"));
    assert!(mb.blocked().is_empty());
    assert!(mb.undo());
    assert_eq!(mb.blocked(), vec!["a@x.test"]);
}
#[test]
fn mark_spam_deletes_and_optionally_blocks_in_one_step() {
    let mut mb = sample();
    assert_eq!(mb.mark_spam(&[1], false), 1);
    assert_eq!(mb.state_of(1), Some(State::Deleted));
    assert!(mb.blocked().is_empty());
    assert!(mb.undo());
    assert_eq!(mb.state_of(1), Some(State::Inbox));
    assert_eq!(mb.mark_spam(&[1], true), 1);
    assert_eq!(mb.blocked(), vec!["a@x.test"]);
    assert!(mb.undo());
    assert!(mb.blocked().is_empty());
    assert_eq!(mb.state_of(1), Some(State::Inbox));
}
#[test]
fn unsubscribe_moves_inbox_and_undoes() {
    let mut mb = sample();
    assert_eq!(mb.unsubscribe("a@x.test", Some(State::Deleted)), 1);
    assert_eq!(mb.state_of(1), Some(State::Deleted));
    assert_eq!(mb.unsubscribed(), &[String::from("a@x.test")]);
    assert!(mb.undo());
    assert_eq!(mb.state_of(1), Some(State::Inbox));
    assert!(mb.unsubscribed().is_empty());
}
#[test]
fn chips_are_ignored_outside_inbox_locations() {
    let mb = sample();
    let archive = View {
        location: Location::Archive("personal".into()),
        chip: Chip::PossibleSpam,
        ..View::default()
    };
    assert_eq!(mb.ids_in_view(&archive).len(), 1);
}
#[test]
fn sent_folder_and_tag_filter_locations_select_expected_messages() {
    let mut mb = sample();
    mb.apply_auto(
        crate::helpers::sug(
            1,
            mail_classifier::judge::QuestionKey::NeedsReply,
            mail_classifier::judge::AnswerValue::Bool(true),
        ),
        0,
    );
    let inbox = View {
        location: Location::AllInboxes,
        chip: Chip::NeedsReply,
        filter: Filter {
            tags: vec![TagFilter::NeedsReply],
            kind: None,
            account: Some("personal".into()),
        },
    };
    assert_eq!(mb.ids_in_view(&inbox), vec![1]);
    mb.send_reply_at(1, "sent".into(), false, 0);
    mb.tick(10);
    assert_eq!(
        mb.ids_in_view(&View {
            location: Location::Sent("personal".into()),
            ..View::default()
        })
        .len(),
        1
    );
    let folder = mb.create_folder("personal", "Receipts", None);
    mb.set_state(&[2], State::Filed(folder));
    assert_eq!(
        mb.ids_in_view(&View {
            location: Location::Folder(folder),
            ..View::default()
        }),
        vec![2]
    );
}

#[test]
fn grouped_folds_several_steps_into_one_undo() {
    let mut mb = sample();
    mb.grouped(|mb| {
        mb.set_state(&[1], State::Archived);
        mb.set_state(&[2], State::Deleted);
    });
    assert!(mb.undo());
    assert_eq!(mb.state_of(1), Some(State::Inbox));
    assert_eq!(mb.state_of(2), Some(State::Inbox));
}

#[test]
fn creating_a_folder_and_filing_undoes_as_one_step() {
    let mut mb = sample();
    let (folder, moved) = mb.create_folder_and_file("personal", "Zebra", None, &[1, 2]);
    assert_eq!(moved, 2);
    assert_eq!(mb.state_of(1), Some(State::Filed(folder)));
    assert_eq!(mb.folder(folder).map(|f| f.name.as_str()), Some("Zebra"));

    assert!(mb.undo());
    assert_eq!(mb.state_of(1), Some(State::Inbox));
    assert_eq!(mb.state_of(2), Some(State::Inbox));
    assert!(mb.folder(folder).is_none(), "the folder goes with the move");
    assert!(!mb.undo(), "one step covered both");
}
