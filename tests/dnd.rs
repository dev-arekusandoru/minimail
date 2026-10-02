//! Drag payload rules: which messages a drag out of the list carries, and what a drop opens.

use mail_classifier::dnd::{MailDrag, payload_ids};

fn drag(selected: &[u32], row: &[u32]) -> MailDrag {
    MailDrag::for_row(selected, row, None).expect("a row with messages is draggable")
}

#[test]
fn a_row_inside_the_selection_drags_the_whole_selection() {
    let d = drag(&[7, 8, 9], &[8]);
    assert_eq!(d.ids, vec![7, 8, 9]);
    assert_eq!(d.count(), 3);
}

#[test]
fn a_row_outside_the_selection_drags_only_itself() {
    let d = drag(&[7, 8], &[42]);
    assert_eq!(d.ids, vec![42]);
}

#[test]
fn nothing_selected_drags_the_row() {
    let d = drag(&[], &[42]);
    assert_eq!(d.ids, vec![42]);
}

#[test]
fn a_thread_header_drags_every_message_it_stands_for() {
    let d = drag(&[], &[11, 12, 13]);
    assert_eq!(d.ids, vec![11, 12, 13]);
}

#[test]
fn a_partly_selected_thread_header_still_drags_the_whole_selection() {
    // The header is one of the selected rows, so the drag carries what the user selected rather
    // than the thread the pointer happened to be over.
    let d = drag(&[11, 99], &[11, 12, 13]);
    assert_eq!(d.ids, vec![11, 99]);
}

#[test]
fn the_anchor_is_the_grabbed_row_not_the_selection() {
    // The anchor decides which tab a drop on the reader strip opens.
    let d = drag(&[7, 8, 9], &[8]);
    assert_eq!(d.anchor, 8);
}

#[test]
fn an_empty_row_is_not_draggable() {
    assert!(MailDrag::for_row(&[7], &[], None).is_none());
}

#[test]
fn the_thread_rides_along_for_the_tab_strip() {
    assert_eq!(MailDrag::for_row(&[], &[42], Some(5)).unwrap().thread, Some(5));
    assert_eq!(MailDrag::for_row(&[], &[42], None).unwrap().thread, None);
}

#[test]
fn payload_ids_keeps_the_selection_order() {
    assert_eq!(payload_ids(&[9, 7, 8], &[7]), vec![9, 7, 8]);
}
