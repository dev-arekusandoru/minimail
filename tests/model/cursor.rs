use crate::helpers::{State, sample};
use mail_classifier::model::{Location, Triage, View};
#[test]
fn cursor_selection_and_view_switch_use_view_ids() {
    let mb = sample();
    let view = View {
        location: Location::Inbox("personal".into()),
        ..View::default()
    };
    let mut t = Triage::new(view.clone());
    assert_eq!(t.cursor(&mb), Some(1));
    t.extend(&mb, 1);
    assert_eq!(t.selected(), vec![1, 2]);
    t.switch_view(View {
        location: Location::Archive("personal".into()),
        ..View::default()
    });
    assert_eq!(t.cursor_index(), 0);
    assert!(t.selected().is_empty());
    assert_eq!(t.cursor(&mb), Some(3));
}
#[test]
fn apply_moves_selection_in_one_undo_step() {
    let mut mb = sample();
    let mut t = Triage::new(View {
        location: Location::Inbox("personal".into()),
        ..View::default()
    });
    t.set_selection(vec![1, 4]);
    assert_eq!(t.apply(&mut mb, State::Archived), 2);
    assert_eq!(mb.state_of(1), Some(State::Archived));
    assert!(mb.undo());
    assert_eq!(mb.state_of(1), Some(State::Inbox));
}
#[test]
fn sender_apply_only_changes_inbox_messages() {
    let mut mb = sample();
    let mut t = Triage::new(View {
        location: Location::Inbox("personal".into()),
        ..View::default()
    });
    assert_eq!(t.apply_to_sender(&mut mb, State::Deleted), 1);
    assert_eq!(mb.state_of(3), Some(State::Archived));
}
