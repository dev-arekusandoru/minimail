//! Pure reader-tab state: preview replacement, pinning, activation and removal.

use mail_classifier::tabs::Tabs;

fn threads(t: &Tabs) -> Vec<(u32, bool)> {
    t.tabs().iter().map(|t| (t.thread, t.pinned)).collect()
}

#[test]
fn reordering_moves_the_tab_and_keeps_it_open() {
    let mut t = Tabs::default();
    for th in 1..=3 {
        t.open_pinned(th, th * 10);
    }
    t.activate(0);
    assert!(t.reorder(0, 2));
    assert_eq!(threads(&t), vec![(2, true), (3, true), (1, true)]);
    assert_eq!(t.active_index(), Some(2), "the active tab follows its thread");
    assert_eq!(t.opened(), Some(10));
}

#[test]
fn reordering_a_tab_left_of_the_active_one_leaves_it_active() {
    let mut t = Tabs::default();
    for th in 1..=3 {
        t.open_pinned(th, th * 10);
    }
    assert_eq!(t.active_index(), Some(2));
    assert!(t.reorder(2, 0));
    assert_eq!(threads(&t), vec![(3, true), (1, true), (2, true)]);
    assert_eq!(t.opened(), Some(30));
}

#[test]
fn reordering_carries_the_pin_state_and_the_opened_message() {
    let mut t = Tabs::default();
    t.open_pinned(1, 10);
    t.open(2, 20);
    assert!(t.reorder(1, 0));
    assert_eq!(threads(&t), vec![(2, false), (1, true)]);
    assert_eq!(t.tab_for(1), Some(&mail_classifier::tabs::Tab { thread: 1, msg: 10, pinned: true }));
}

#[test]
fn reordering_an_unknown_slot_changes_nothing() {
    let mut t = Tabs::default();
    t.open_pinned(1, 10);
    assert!(!t.reorder(1, 0), "target past the end");
    assert!(!t.reorder(0, 1), "target past the end");
    assert!(!t.reorder(3, 0));
    assert_eq!(threads(&t), vec![(1, true)]);
    assert_eq!(t.active_index(), Some(0));
    let mut empty = Tabs::default();
    assert!(!empty.reorder(0, 0), "no tabs at all");
}

#[test]
fn opening_replaces_the_preview_in_place() {
    let mut t = Tabs::default();
    assert_eq!(t.open(1, 10), None);
    assert_eq!(t.open(2, 20), Some(1), "the replaced preview's thread is reported");
    assert_eq!(threads(&t), vec![(2, false)]);
    assert_eq!(t.opened(), Some(20));
}

#[test]
fn a_pinned_tab_survives_and_the_next_open_makes_a_second_tab() {
    let mut t = Tabs::default();
    t.open(1, 10);
    t.pin(1);
    assert_eq!(t.open(2, 20), None);
    assert_eq!(threads(&t), vec![(1, true), (2, false)]);
    assert_eq!(t.active_index(), Some(1));
    // The preview slot is reused wherever it sits.
    t.open(3, 30);
    assert_eq!(threads(&t), vec![(1, true), (3, false)]);
    t.pin_active();
    t.open(4, 40);
    assert_eq!(threads(&t), vec![(1, true), (3, true), (4, false)]);
}

#[test]
fn opening_a_message_of_a_tabbed_thread_activates_that_tab() {
    let mut t = Tabs::default();
    t.open_pinned(1, 10);
    t.open(2, 20);
    assert_eq!(t.open(1, 11), None);
    assert_eq!(t.active_index(), Some(0));
    assert_eq!(t.opened(), Some(11), "the tab now shows the message that was opened");
    assert_eq!(threads(&t), vec![(1, true), (2, false)], "pin state is kept");

    // Re-opening the preview's thread does not pin it.
    t.open(2, 21);
    assert_eq!(threads(&t), vec![(1, true), (2, false)]);
}

#[test]
fn closing_the_active_tab_falls_to_the_right_then_the_left() {
    let mut t = Tabs::default();
    for th in 1..=3 {
        t.open_pinned(th, th * 10);
    }
    t.activate(1);
    assert_eq!(t.close(1), Some(2));
    assert_eq!(t.opened(), Some(30), "right neighbour");
    assert_eq!(t.close(1), Some(3));
    assert_eq!(t.opened(), Some(10), "left neighbour when there is none on the right");
    assert_eq!(t.close(0), Some(1));
    assert!(t.is_empty() && t.active().is_none() && t.opened().is_none());
    assert_eq!(t.close(0), None);
}

#[test]
fn closing_a_tab_left_of_the_active_one_keeps_it_active() {
    let mut t = Tabs::default();
    for th in 1..=3 {
        t.open_pinned(th, th * 10);
    }
    assert_eq!(t.active_index(), Some(2));
    t.close(0);
    assert_eq!(t.opened(), Some(30));
    assert_eq!(t.active_index(), Some(1));
}

#[test]
fn cycling_wraps_and_needs_two_tabs() {
    let mut t = Tabs::default();
    assert!(!t.cycle(1));
    t.open_pinned(1, 10);
    assert!(!t.cycle(1));
    t.open_pinned(2, 20);
    t.open_pinned(3, 30);
    assert!(t.cycle(1));
    assert_eq!(t.active_index(), Some(0));
    assert!(t.cycle(-1));
    assert_eq!(t.active_index(), Some(2));
}

#[test]
fn a_preview_leaving_the_view_closes_but_pinned_tabs_stay() {
    let mut t = Tabs::default();
    t.open_pinned(1, 10);
    t.open(2, 20);
    assert_eq!(t.retain_previews(|th| th != 1), Vec::<u32>::new(), "pinned tab 1 stays though rejected");
    assert_eq!(threads(&t), vec![(1, true), (2, false)]);

    assert_eq!(t.retain_previews(|th| th != 2), vec![2]);
    assert_eq!(threads(&t), vec![(1, true)]);
    assert_eq!(t.opened(), Some(10), "active falls back to the remaining tab");
    assert!(!t.has_preview());
}

#[test]
fn removing_a_preview_keeps_an_active_pinned_tab_active() {
    let mut t = Tabs::default();
    t.open(1, 10);
    t.open_pinned(2, 20); // preview slot is reused: tab 0 becomes thread 2
    t.open(3, 30);
    t.activate(0);
    t.retain_previews(|_| false);
    assert_eq!(threads(&t), vec![(2, true)]);
    assert_eq!(t.opened(), Some(20));
}

#[test]
fn pinning_and_set_msg_ignore_unknown_threads() {
    let mut t = Tabs::default();
    t.open(1, 10);
    t.pin(9);
    t.set_msg(9, 99);
    t.set_msg(1, 11);
    assert_eq!(threads(&t), vec![(1, false)]);
    assert_eq!(t.opened(), Some(11));
}

#[test]
fn titles_are_trimmed_truncated_and_never_blank() {
    use mail_classifier::tabs::title;
    assert_eq!(title("  Lunch?  ", 20), "Lunch?");
    assert_eq!(title("   ", 20), "(no subject)");
    assert_eq!(title("abcdefghij", 10), "abcdefghij");
    assert_eq!(title("abcdefghijk", 10), "abcdefghi…");
    assert_eq!(title("ééééééééééé", 5), "éééé…", "cuts on characters, not bytes");
}
