//! Pure thread grouping and navigation.

use std::collections::HashSet;

use mail_classifier::model::{Mailbox, MessageId};
use mail_classifier::threads::{Row, group, participants, rows, step, thread_order};

fn msg(id: u32, thread: u32, name: &str, day: u32) -> String {
    format!(
        r#"{{"id":{id},"thread_id":{thread},"from_name":"{name}","from_email":"{name}@x.com","to":"me","subject":"s","body":"b","received":"2026-09-{day:02}T09:00:00Z"}}"#
    )
}

fn mailbox() -> Mailbox {
    let m = [msg(1, 10, "ann", 1), msg(2, 10, "bob", 2), msg(3, 20, "cy", 3), msg(4, 10, "ann", 4)];
    Mailbox::from_json(&format!("[{}]", m.join(","))).unwrap()
}

#[test]
fn groups_are_ordered_by_newest_message_and_keep_input_order() {
    let mb = mailbox();
    let ids: Vec<MessageId> = vec![4, 3, 2, 1];
    let groups = group(&ids, |id| mb.get(id));
    assert_eq!(groups.len(), 2);
    assert_eq!((groups[0].thread_id, groups[0].ids.clone()), (10, vec![4, 2, 1]));
    assert_eq!((groups[1].thread_id, groups[1].ids.clone()), (20, vec![3]));
    assert_eq!(groups[0].latest(), 4);
}

#[test]
fn rows_collapse_expand_and_single_messages_stay_plain() {
    let mb = mailbox();
    let groups = group(&[4, 3, 2, 1], |id| mb.get(id));
    let collapsed = rows(&groups, &HashSet::new());
    assert_eq!(collapsed.len(), 2);
    assert!(matches!(&collapsed[0], Row::Header { expanded: false, ids, .. } if ids == &[4, 2, 1]));
    assert_eq!(collapsed[1], Row::Single(3));
    assert_eq!(collapsed[0].primary(), 4);
    assert_eq!(collapsed[0].ids(), vec![4, 2, 1]);

    let open = rows(&groups, &HashSet::from([10]));
    assert_eq!(open.len(), 5);
    assert_eq!(open[1], Row::Child { thread_id: 10, id: 4 });
    assert_eq!(open[3], Row::Child { thread_id: 10, id: 1 });
}

#[test]
fn a_panel_only_groups_the_ids_it_is_given() {
    let mb = mailbox();
    // Message 2 lives in another panel.
    let groups = group(&[4, 3, 1], |id| mb.get(id));
    assert_eq!(groups[0].ids, vec![4, 1]);
    let solo = group(&[2], |id| mb.get(id));
    assert_eq!(rows(&solo, &HashSet::new()), vec![Row::Single(2)]);
}

#[test]
fn participants_are_distinct_oldest_first() {
    let mb = mailbox();
    assert_eq!(participants(&[4, 2, 1], |id| mb.get(id)), vec!["ann", "bob"]);
}

#[test]
fn thread_order_is_by_date_and_step_stops_at_the_ends() {
    let mb = mailbox();
    let order = thread_order(mb.messages(), 10);
    assert_eq!(order, vec![1, 2, 4]);
    assert_eq!(step(&order, 2, 1), Some(4));
    assert_eq!(step(&order, 2, -1), Some(1));
    assert_eq!(step(&order, 1, -1), None);
    assert_eq!(step(&order, 4, 1), None);
    assert_eq!(step(&order, 99, 1), None);
}
