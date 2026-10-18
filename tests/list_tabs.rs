//! List tab behaviour: what opens a tab, what replaces the preview, and what closing leaves.

use mail_classifier::list_tabs::ListTabs;
use mail_classifier::search::Query;

fn q(view: &str) -> Query {
    Query::parse(view)
}

fn views(tabs: &ListTabs) -> Vec<(String, bool)> {
    tabs.tabs().iter().map(|tab| (tab.query.to_string(), tab.pinned)).collect()
}

fn strip() -> ListTabs {
    ListTabs::new(q("in:inbox"))
}

#[test]
fn a_new_view_replaces_the_preview_in_place() {
    let mut t = strip();
    t.open_pinned(q("in:archived"));
    t.open(q("in:deleted"));
    t.open(q("in:sent"));
    assert_eq!(views(&t), vec![
        ("in:inbox".to_owned(), true),
        ("in:archived".to_owned(), true),
        ("in:sent".to_owned(), false),
    ], "the preview was replaced where it sat, not appended");
    assert_eq!(t.active_index(), 2);
}

#[test]
fn an_open_view_is_activated_instead_of_opened_twice() {
    let mut t = strip();
    t.open_pinned(q("in:archived"));
    t.open(q("in:deleted"));
    assert_eq!(t.len(), 3, "the preview was appended, since both tabs were pinned");
    assert_eq!(t.open(q("in:archived")), 1);
    assert_eq!(t.len(), 3, "no second tab for a view that is already open");
    assert_eq!(t.active_index(), 1);
}

#[test]
fn opening_pinned_promotes_a_preview_and_appends_when_new() {
    let mut t = strip();
    t.open(q("in:deleted"));
    assert_eq!(t.open_pinned(q("in:deleted")), 1);
    assert_eq!(views(&t), vec![
        ("in:inbox".to_owned(), true),
        ("in:deleted".to_owned(), true),
    ]);
    assert_eq!(t.open_pinned(q("in:sent")), 2);
    assert_eq!(t.len(), 3);
}

#[test]
fn filters_make_a_view_of_its_own() {
    let mut t = strip();
    t.open_pinned(q("in:inbox is:unread"));
    assert_eq!(t.len(), 2, "a filtered inbox is a different tab from the inbox");
    t.open(q("in:inbox"));
    assert_eq!(t.len(), 2, "and clicking the plain inbox activates its own tab");
    assert_eq!(t.active_index(), 0);
}

#[test]
fn the_last_tab_cannot_be_closed() {
    let mut t = strip();
    assert!(!t.close(0));
    assert_eq!(t.len(), 1);
    t.open_pinned(q("in:archived"));
    assert!(t.close(0));
    assert_eq!(views(&t), vec![("in:archived".to_owned(), true)]);
    assert!(!t.close(0), "the strip is never empty");
}

#[test]
fn closing_the_active_tab_falls_to_its_right_neighbour() {
    let mut t = strip();
    t.open_pinned(q("in:archived"));
    t.open_pinned(q("in:sent"));
    t.activate(1);
    assert!(t.close(1));
    assert_eq!(t.active().map(|tab| tab.query.to_string()), Some("in:sent".to_owned()));
}

#[test]
fn cycling_wraps_and_needs_two_tabs() {
    let mut t = strip();
    assert!(!t.cycle(1), "one tab has nowhere to go");
    t.open_pinned(q("in:archived"));
    assert!(t.cycle(1));
    assert_eq!(t.active_index(), 0, "wrapped round");
    assert!(t.cycle(-1));
    assert_eq!(t.active_index(), 1);
}

#[test]
fn reordering_takes_the_open_tab_along_and_carries_pins() {
    let mut t = strip();
    t.open_pinned(q("in:archived"));
    t.open(q("in:sent"));
    t.activate(1);
    assert!(t.reorder(2, 0));
    assert_eq!(views(&t), vec![
        ("in:sent".to_owned(), false),
        ("in:inbox".to_owned(), true),
        ("in:archived".to_owned(), true),
    ]);
    assert_eq!(t.active_index(), 2, "the tab that was showing is still showing, moved");
    assert_eq!(t.active().map(|tab| tab.query.to_string()), Some("in:archived".to_owned()));
}

#[test]
fn an_order_that_is_not_exactly_these_tabs_is_refused() {
    let mut t = strip();
    t.open_pinned(q("in:archived"));
    assert!(!t.reorder_to(&[q("in:archived")]), "short order");
    assert!(!t.reorder_to(&[q("in:archived"), q("in:inbox"), q("in:sent")]), "long order");
    assert!(!t.reorder_to(&[q("in:sent"), q("in:inbox")]), "a view that is not open");
    assert_eq!(views(&t), vec![
        ("in:inbox".to_owned(), true),
        ("in:archived".to_owned(), true),
    ]);
    assert!(t.reorder_to(&[q("in:archived"), q("in:inbox")]));
    assert_eq!(views(&t)[0].0, "in:archived");
}

#[test]
fn retargeting_changes_a_tab_in_place_but_never_duplicates_a_view() {
    let mut t = strip();
    t.open_pinned(q("in:archived"));
    t.open_pinned(q("in:sent"));
    t.activate(1);

    assert!(t.retarget(1, q("in:deleted")));
    assert_eq!(views(&t)[1], ("in:deleted".to_owned(), true), "same slot, same pin");
    assert_eq!(t.active_index(), 1);

    assert!(!t.retarget(1, q("in:sent")), "another tab already shows it");
    assert!(!t.retarget(9, q("in:inbox")), "unknown slot");
    assert_eq!(views(&t)[1].0, "in:deleted");
}
