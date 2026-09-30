//! Pure reader-tab state: preview replacement, pinning, activation and removal.

use mail_classifier::tabs::Tabs;

fn threads(t: &Tabs) -> Vec<(u32, bool)> {
    t.tabs().iter().map(|t| (t.thread, t.pinned)).collect()
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
