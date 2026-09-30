use mail_classifier::model::{Mailbox, Triage};
use crate::helpers::{assert_invariant, mailbox, sample};
use crate::helpers::State;

// ------------------------------------------------------------------ cursor

#[test]
fn move_cursor_clamps_at_both_ends() {
    let mb = sample();
    let mut t = Triage::new(State::Inbox);
    assert_eq!(t.cursor_index(), 0);
    assert_eq!(t.cursor(&mb), Some(2));

    t.move_cursor(&mb, 99);
    assert_eq!(t.cursor_index(), 1);
    assert_eq!(t.cursor(&mb), Some(1));

    t.move_cursor(&mb, -99);
    assert_eq!(t.cursor_index(), 0);
    assert_eq!(t.cursor(&mb), Some(2));
}

#[test]
fn an_empty_view_has_no_cursor_and_moving_is_safe() {
    let empty = mailbox(&[]);
    let mut t = Triage::new(State::Done);
    assert_eq!(t.cursor(&empty), None);
    t.move_cursor(&empty, 3);
    t.extend(&empty, -2);
    t.toggle_select(&empty);
    assert_eq!(t.cursor_index(), 0);
    assert!(t.selected().is_empty());
    assert!(t.targets(&empty).is_empty());
}

#[test]
fn the_cursor_clamps_when_the_view_shrinks_under_it() {
    let mut mb = sample();
    let mut t = Triage::new(State::Inbox);
    t.move_cursor(&mb, 1); // index 1: the oldest Inbox message, id 1
    assert_eq!(t.cursor(&mb), Some(1));

    // The message under the cursor leaves the view; the position holds.
    mb.set_state(&[1], State::Done);
    assert_eq!(t.cursor_index(), 1, "the raw index is not rewritten");
    assert_eq!(t.cursor(&mb), Some(2), "the cursor message is clamped");
    assert_eq!(t.cursor(&mb), mb.ids_in(State::Inbox).last().copied());

    // Emptying the view must not panic.
    mb.set_state(&[2], State::Done);
    assert!(mb.ids_in(State::Inbox).is_empty());
    assert_eq!(t.cursor(&mb), None);
    assert_eq!(t.cursor_index(), 1);
}

// ------------------------------------------------------------------ selection

/// Four Inbox messages; the view is ordered 4, 3, 2, 1.
fn four() -> Mailbox {
    mailbox(&[
        (1, "a@x.test", "2026-09-01T10:00:00Z", State::Inbox),
        (2, "b@x.test", "2026-09-02T10:00:00Z", State::Inbox),
        (3, "c@x.test", "2026-09-03T10:00:00Z", State::Inbox),
        (4, "d@x.test", "2026-09-04T10:00:00Z", State::Inbox),
    ])
}

#[test]
fn extend_selects_an_inclusive_range_from_the_anchor() {
    let mb = four();
    let mut t = Triage::new(State::Inbox);
    t.extend(&mb, 1);
    assert_eq!(t.cursor_index(), 1);
    assert_eq!(t.selected(), vec![4, 3], "anchor and cursor, inclusive");

    t.extend(&mb, 1);
    assert_eq!(t.selected(), vec![4, 3, 2]);

    t.extend(&mb, -2);
    assert_eq!(t.selected(), vec![4], "the anchor never moves");
}

#[test]
fn extending_backwards_keeps_the_selection_in_view_order() {
    let mb = four();
    let mut t = Triage::new(State::Inbox);
    t.move_cursor(&mb, 2);
    t.extend(&mb, -1);
    assert_eq!(t.selected(), vec![3, 2]);
    assert!(t.is_selected(3) && t.is_selected(2) && !t.is_selected(1));
}

#[test]
fn clearing_the_selection_resets_the_anchor() {
    let mb = four();
    let mut t = Triage::new(State::Inbox);
    t.extend(&mb, 2);
    assert_eq!(t.selected(), vec![4, 3, 2]);
    t.clear_selection();
    assert!(t.selected().is_empty());

    t.move_cursor(&mb, -1);
    t.extend(&mb, 1);
    assert_eq!(t.selected(), vec![3, 2], "a new range, not the old one");
}

#[test]
fn toggle_select_adds_then_removes_and_keeps_view_order() {
    let mb = four();
    let mut t = Triage::new(State::Inbox);
    t.toggle_select(&mb);
    t.move_cursor(&mb, 2);
    t.toggle_select(&mb);
    assert_eq!(t.selected(), vec![4, 2]);
    t.toggle_select(&mb);
    assert_eq!(t.selected(), vec![4]);
    assert!(!t.is_selected(2));
}

#[test]
fn move_cursor_does_not_disturb_the_selection() {
    let mb = four();
    let mut t = Triage::new(State::Inbox);
    t.toggle_select(&mb);
    t.move_cursor(&mb, 1);
    assert_eq!(t.selected(), vec![4]);
    assert_eq!(t.cursor(&mb), Some(3));
}

#[test]
fn targets_prefer_the_selection_and_fall_back_to_the_cursor() {
    let mb = four();
    let mut t = Triage::new(State::Inbox);
    assert_eq!(t.targets(&mb), vec![4], "no selection: just the cursor");
    t.toggle_select(&mb);
    t.move_cursor(&mb, 1);
    assert_eq!(t.targets(&mb), vec![4], "the selection wins");
}

#[test]
fn apply_moves_the_targets_clears_the_selection_and_clamps() {
    let mut mb = four();
    let mut t = Triage::new(State::Inbox);
    t.extend(&mb, 1); // selects 4 and 3
    assert_eq!(t.apply(&mut mb, State::Done), 2);
    assert!(t.selected().is_empty());
    assert_eq!(t.cursor_index(), 1, "the cursor keeps its position");
    assert_eq!(t.cursor(&mb), Some(1), "the cursor now sits on the last remaining message");
    assert_invariant(&mb);

    // One undo rewinds the whole batch.
    assert!(mb.undo());
    assert_eq!(mb.count(State::Inbox), 4);
    assert_eq!(mb.count(State::Done), 0);
}

#[test]
fn apply_on_an_empty_view_changes_nothing() {
    let mut mb = four();
    let mut t = Triage::new(State::Later);
    t.move_cursor(&mb, 0);
    assert_eq!(t.apply(&mut mb, State::Done), 0);
    assert!(!mb.undo());
    assert_eq!(t.apply_to_sender(&mut mb, State::Done), 0);
    assert_invariant(&mb);
}

#[test]
fn apply_to_sender_moves_every_message_from_that_sender() {
    let mut mb = sample();
    let mut t = Triage::new(State::Inbox);
    t.move_cursor(&mb, 1); // id 1, from a@x.test
    assert_eq!(t.apply_to_sender(&mut mb, State::Done), 3);
    assert_eq!(mb.ids_in(State::Inbox), vec![2]);
    assert_eq!(mb.count(State::Done), 4);
    assert_invariant(&mb);

    assert!(mb.undo());
    assert_eq!(mb.count(State::Done), 1);
    assert_eq!(mb.state_of(3), Some(State::Later));
}

#[test]
fn switch_view_resets_the_cursor_and_the_selection() {
    let mb = mailbox(&[
        (1, "a@x.test", "2026-09-01T10:00:00Z", State::Inbox),
        (2, "b@x.test", "2026-09-02T10:00:00Z", State::Inbox),
        (3, "c@x.test", "2026-09-03T10:00:00Z", State::Later),
    ]);
    let mut t = Triage::new(State::Inbox);
    t.extend(&mb, 1);
    t.switch_view(State::Later);
    assert_eq!(t.view, State::Later);
    assert_eq!(t.cursor_index(), 0);
    assert!(t.selected().is_empty());
    assert_eq!(t.cursor(&mb), Some(3));
    assert_eq!(t.targets(&mb), vec![3]);
}
