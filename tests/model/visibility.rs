use crate::helpers::{State, sample};
use mail_classifier::clock::Timestamp;
use mail_classifier::model::Location;
use mail_classifier::search::{Field, Query};

/// Instant the sample mailbox is judged at (its newest message is 2026-09-29).
const NOW: Timestamp = 1_790_000_000;
#[test]
fn block_move_is_one_undo_step_and_unblock_is_undoable() {
    let mut mb = sample();
    assert_eq!(mb.block_sender("a@x.test", Some(State::Archived), NOW), 1);
    assert_eq!(mb.state_of(1), Some(State::Archived));
    assert_eq!(mb.state_of(3), Some(State::Archived));
    assert_eq!(mb.blocked(), vec![("a@x.test".to_owned(), NOW)]);
    assert!(mb.undo());
    assert_eq!(mb.state_of(1), Some(State::Inbox));
    assert!(mb.blocked().is_empty());
    assert_eq!(mb.block_sender("a@x.test", None, NOW), 0);
    assert!(mb.unblock_sender("a@x.test"));
    assert!(mb.blocked().is_empty());
    assert!(mb.undo());
    assert_eq!(mb.blocked(), vec![("a@x.test".to_owned(), NOW)]);
}
#[test]
fn the_block_time_survives_unblock_and_undo_and_is_not_reset_by_blocking_again() {
    let mut mb = sample();
    mb.block_sender("b@x.test", None, NOW);
    mb.block_sender("A@x.test", None, NOW + 100);
    assert_eq!(
        mb.blocked(),
        vec![("a@x.test".to_owned(), NOW + 100), ("b@x.test".to_owned(), NOW)],
        "sorted by address, each with its own time"
    );
    assert_eq!(mb.block_sender("a@x.test", None, NOW + 999), 0, "already blocked");
    assert!(mb.unblock_sender("a@x.test"));
    assert!(mb.undo());
    assert_eq!(mb.blocked()[0], ("a@x.test".to_owned(), NOW + 100));
}
#[test]
fn mark_spam_deletes_and_optionally_blocks_in_one_step() {
    let mut mb = sample();
    assert_eq!(mb.mark_spam(&[1], false, NOW), 1);
    assert_eq!(mb.state_of(1), Some(State::Deleted));
    assert!(mb.blocked().is_empty());
    assert!(mb.undo());
    assert_eq!(mb.state_of(1), Some(State::Inbox));
    assert_eq!(mb.mark_spam(&[1], true, NOW), 1);
    assert_eq!(mb.blocked(), vec![("a@x.test".to_owned(), NOW)]);
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
fn filters_narrow_every_location_and_muted_mail_never_matches() {
    let mb = sample();
    let mut archive = Query::default();
    archive.add(Field::In, "archived");
    archive.add(Field::Account, "personal");
    assert_eq!(mb.ids_matching(&archive, NOW), vec![3]);
    // A sender pill is a filter like any other, and hides what it names.
    archive.add(Field::From, "a@x.test");
    assert_eq!(mb.ids_matching(&archive, NOW), vec![3], "message 3 is from a@x.test");
    archive.add(Field::From, "c@x.test");
    assert!(mb.ids_matching(&archive, NOW).is_empty(), "one of the two senders, not both");

    let mut mb = mb;
    assert!(mb.mute_thread(3));
    assert!(mb.ids_matching(&archive, NOW).is_empty(), "a muted thread is out of the list");
    assert_eq!(mb.ids_matching(&Query::default(), NOW).len(), 4, "hidden mail is not listed");
}
#[test]
fn a_location_query_round_trips_through_its_tokens() {
    let mut mb = sample();
    mb.apply_auto(
        crate::helpers::sug(
            1,
            mail_classifier::judge::QuestionKey::NeedsReply,
            mail_classifier::judge::AnswerValue::Bool(true),
        ),
        0,
    );
    let mut inbox = Query::default();
    inbox.add(Field::Tag, "needs-reply");
    inbox.add(Field::Account, "personal");
    inbox.add(Field::In, "inbox");
    assert_eq!(mb.ids_matching(&inbox, NOW), vec![1]);
    assert_eq!(
        mb.query_location(&inbox),
        Some(Location::Inbox("personal".into())),
        "in:inbox plus account: is that account's inbox"
    );
    let all = mb.location_query(&Location::AllInboxes);
    assert_eq!(mb.query_location(&all), Some(Location::AllInboxes), "a bare in:inbox is every inbox");

    mb.send_reply_at(1, "sent".into(), false, 0);
    mb.tick(10);
    let sent = mb.location_query(&Location::Sent("personal".into()));
    assert_eq!(mb.ids_matching(&sent, NOW).len(), 1);
    assert_eq!(mb.query_location(&sent), Some(Location::Sent("personal".into())));

    let folder = mb.create_folder("personal", "Zebras", None);
    mb.set_state(&[2], State::Filed(folder));
    let filed = mb.location_query(&Location::Folder(folder));
    assert_eq!(mb.ids_matching(&filed, NOW), vec![2]);
    assert_eq!(mb.query_location(&filed), Some(Location::Folder(folder)));
    // The folder's own path is the `in:` value, so a nested folder still resolves.
    assert_eq!(filed.values(Field::In), ["zebras"]);
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
