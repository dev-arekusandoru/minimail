//! Settings window behaviour: its own window, the pages it shows, and the keyboard path.

use std::rc::Rc;

use gpui_kit::TestAppContext;
use mail_classifier::contacts::ContactStore;
use mail_classifier::model::Mailbox;

#[path = "ui_ext/harness.rs"]
#[allow(dead_code)]
mod harness;
use harness::{Harness, harness_with, harness_with_prefs, mailbox, msg};

/// Sidebar page indexes.
const ACCOUNTS: usize = 0;
const APPEARANCE: usize = 1;
const INBOX: usize = 2;
const AI: usize = 3;
const SENDERS: usize = 4;

fn app(cx: &mut TestAppContext) -> Harness<'_> {
    harness_with(cx, mailbox(&[msg(1, 1, "alice@example.com", "Question", 1, "Inbox")]))
}

/// An in-memory address book, shared by two apps to stand in for a restart. Never the real
/// database.
fn prefs_store() -> Rc<ContactStore> {
    Rc::new(ContactStore::open_in_memory().expect("in-memory address book"))
}

#[gpui_kit::gpui::test]
fn the_summaries_switch_enables_summaries_and_escape_closes(cx: &mut TestAppContext) {
    let mut h = app(cx);
    assert!(!h.read(|a| a.summaries_enabled));
    h.settings_open_page(AI);
    assert!(h.read(|a| a.settings_open()));
    h.settings_click_in(0, 0, "check");
    assert!(h.read(|a| a.summaries_enabled));
    h.settings_click_in(0, 0, "check");
    assert!(!h.read(|a| a.summaries_enabled));
    h.settings_keys("escape");
    assert!(!h.read(|a| a.settings_open()), "Esc closes the window");
    assert_eq!(h.settings_window(), None);
}

#[gpui_kit::gpui::test]
fn the_tab_avatar_switch_turns_the_setting_off_and_back_on(cx: &mut TestAppContext) {
    let mut h = app(cx);
    assert!(h.read(|a| a.tab_avatars), "on by default");
    h.settings_open_page(APPEARANCE);
    h.settings_click_in(0, 4, "check");
    assert!(!h.read(|a| a.tab_avatars));
    // The window reopens showing the current value, so the next click turns it back on.
    h.settings_keys("escape");
    h.settings_open_page(APPEARANCE);
    h.settings_click_in(0, 4, "check");
    assert!(h.read(|a| a.tab_avatars));
}

#[gpui_kit::gpui::test]
fn the_remote_image_switch_is_opt_in_and_survives_reopening_settings(cx: &mut TestAppContext) {
    let mut h = app(cx);
    assert!(!h.read(|a| a.block_remote_images), "remote images load by default");

    h.settings_open_page(INBOX);
    h.settings_click_in(0, 3, "check");
    assert!(h.read(|a| a.block_remote_images), "the switch opts into blocking");
    h.settings_keys("escape");
    assert!(!h.read(|a| a.settings_open()));

    // The window reopens showing the current value, so this click turns blocking back off
    // rather than on again.
    h.settings_open_page(INBOX);
    h.settings_click_in(0, 3, "check");
    assert!(!h.read(|a| a.block_remote_images), "the choice persisted across the reopen");
}

#[gpui_kit::gpui::test]
fn opening_settings_again_focuses_the_same_window(cx: &mut TestAppContext) {
    let mut h = app(cx);
    let first = h.open_settings();
    assert!(h.read(|a| a.settings_open()));

    h.keys("cmd-,");
    assert_eq!(h.settings_window(), Some(first), "the same window is focused, not a second one");
    assert!(h.read(|a| a.settings_open()));
    assert_eq!(h.settings_page(), ACCOUNTS, "and it still shows the accounts page");
}

#[gpui_kit::gpui::test]
fn a_changed_setting_survives_a_restart_through_the_shared_store(cx: &mut TestAppContext) {
    let store = prefs_store();
    let mut h = harness_with_prefs(cx, Mailbox::load_default_with(store.clone()), store.clone());
    h.settings_open_page(AI);
    h.settings_click_in(0, 0, "check");
    assert!(h.read(|a| a.summaries_enabled));
    drop(h);

    // A second app over the same database is what a restart amounts to here.
    let mut restarted = harness_with_prefs(cx, Mailbox::load_default_with(store.clone()), store);
    assert!(restarted.read(|a| a.summaries_enabled), "the stored preference comes back");
    assert!(restarted.read(|a| a.tab_avatars), "settings left alone keep their defaults");
}

#[gpui_kit::gpui::test]
fn reset_all_restores_the_defaults_and_drops_the_stored_values(cx: &mut TestAppContext) {
    let store = prefs_store();
    let scope = mail_classifier::prefs::Scope::Global;
    let mut h = harness_with_prefs(cx, Mailbox::load_default_with(store.clone()), store.clone());
    h.settings_open_page(INBOX);
    let grouped = h.read(|a| a.group_threads);
    h.settings_click_in(0, 0, "check");
    assert_eq!(h.read(|a| a.group_threads), !grouped);
    assert!(mail_classifier::app_settings::GROUP_THREADS.is_modified(store.as_ref(), &scope));

    h.settings_click("settings-reset-all");
    h.settings_click("ok");
    assert_eq!(h.read(|a| a.group_threads), grouped, "Reset all puts the default back");
    assert!(!mail_classifier::app_settings::GROUP_THREADS.is_modified(store.as_ref(), &scope));
    assert!(!mail_classifier::app_settings::TAB_AVATARS.is_modified(store.as_ref(), &scope));
}

#[gpui_kit::gpui::test]
fn the_arrows_walk_the_sidebar(cx: &mut TestAppContext) {
    let mut h = app(cx);
    h.open_settings();
    assert_eq!(h.settings_page(), ACCOUNTS);

    h.settings_keys("down");
    assert_eq!(h.settings_page(), APPEARANCE, "down moves to the next page");
    assert!(!h.settings_has(("account-card", 0usize)), "the accounts page is gone");
    h.settings_keys("down");
    assert_eq!(h.settings_page(), INBOX);
    h.settings_keys("down");
    assert_eq!(h.settings_page(), AI);

    h.settings_keys("up up up");
    assert_eq!(h.settings_page(), ACCOUNTS, "up walks back to the first page");
    h.settings_keys("up");
    assert_eq!(h.settings_page(), SENDERS, "and wraps around to the last one");
}

#[gpui_kit::gpui::test]
fn cmd_f_filters_the_pages_and_escape_clears_it_before_closing(cx: &mut TestAppContext) {
    let mut h = app(cx);
    h.open_settings();

    h.settings_keys("cmd-f");
    h.settings_type("senders");
    assert!(!h.settings_has("0-4"), "the pages that do not match leave the sidebar");
    assert!(h.settings_has("0-0"), "and the one that does is the only one left");

    // Esc clears the query and puts every page back; the next Esc closes the window.
    h.settings_keys("escape");
    assert!(h.read(|a| a.settings_open()), "Esc clears the search");
    assert!(h.settings_has("0-4"), "and the sidebar is whole again");
    h.settings_keys("escape");
    assert!(!h.read(|a| a.settings_open()), "Esc on an empty search closes the window");
}

#[gpui_kit::gpui::test]
fn right_steps_out_of_the_sidebar_into_the_search_field(cx: &mut TestAppContext) {
    let mut h = app(cx);
    h.open_settings();
    // `→` steps out of the sidebar into the body, which starts at the search field.
    h.settings_keys("right");
    h.settings_type("blocked");
    assert!(!h.settings_has("0-4"), "the typed query reached the search field");
    assert!(h.settings_has("0-0"), "and left the one matching page");
}

#[gpui_kit::gpui::test]
fn slash_reaches_the_search_field_from_the_sidebar(cx: &mut TestAppContext) {
    let mut h = app(cx);
    h.open_settings();
    h.settings_keys("/");
    h.settings_type("blocked");
    assert!(!h.settings_has("0-4"), "`/` focused the field and the query filtered the pages");
    assert!(h.settings_has("0-0"));
    h.settings_keys("escape");
    assert!(h.read(|a| a.settings_open()), "Esc clears the query it typed");
}
