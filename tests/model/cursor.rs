use crate::helpers::{State, sample};
use mail_classifier::clock::Timestamp;
use mail_classifier::model::{Location, Triage};
/// The sample mailbox is judged at: its newest message is 2026-09-29.
const NOW: Timestamp = 1_790_000_000;

#[test]
fn cursor_selection_and_query_switch_use_the_query_ids() {
    let mb = sample();
    let mut t = Triage::new(mb.location_query(&Location::Inbox("personal".into())));
    assert_eq!(t.cursor(&mb, NOW), Some(1));
    t.extend(&mb, NOW, 1);
    assert_eq!(t.selected(), vec![1, 2]);
    t.set_query(mb.location_query(&Location::Archive("personal".into())));
    assert_eq!(t.cursor_index(), 0);
    assert!(t.selected().is_empty());
    assert_eq!(t.cursor(&mb, NOW), Some(3));
}
#[test]
fn apply_moves_selection_in_one_undo_step() {
    let mut mb = sample();
    let mut t = Triage::new(mb.location_query(&Location::Inbox("personal".into())));
    t.set_selection(vec![1, 4]);
    assert_eq!(t.apply(&mut mb, NOW, State::Archived), 2);
    assert_eq!(mb.state_of(1), Some(State::Archived));
    assert!(mb.undo());
    assert_eq!(mb.state_of(1), Some(State::Inbox));
}
#[test]
fn sender_apply_only_changes_inbox_messages() {
    let mut mb = sample();
    let mut t = Triage::new(mb.location_query(&Location::Inbox("personal".into())));
    assert_eq!(t.apply_to_sender(&mut mb, NOW, State::Deleted), 1);
    assert_eq!(mb.state_of(3), Some(State::Archived));
}
