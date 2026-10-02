//! Appearance and Inbox settings pages, driven through the real settings window.

use gpui_kit::component::ActiveTheme;
use gpui_kit::{AppContext, AnyWindowHandle, TestAppContext};
use mail_classifier::app::mail_app::panes::Orientation;
use mail_classifier::clock::DAY;
use mail_classifier::model::{Tag, TriageState};
use mail_classifier::theme::ThemeMode;

#[path = "ui_ext/harness.rs"]
#[allow(dead_code)]
mod harness;
use harness::{Harness, harness_with, mailbox, msg};

/// Sidebar page indexes.
const APPEARANCE: usize = 1;
const INBOX: usize = 2;

fn app(cx: &mut TestAppContext) -> Harness<'_> {
    harness_with(cx, mailbox(&[msg(1, 1, "alice@example.com", "Question", 1, "Inbox")]))
}

/// Name of the theme the app is showing right now.
fn theme_name(h: &mut Harness<'_>, window: AnyWindowHandle) -> String {
    h.cx
        .update_window(window, |_, _, cx| cx.theme().theme_name().to_string())
        .unwrap()
}

#[gpui_kit::gpui::test]
fn choosing_a_mode_applies_that_appearance_theme(cx: &mut TestAppContext) {
    let mut h = app(cx);
    // Built-in themes registered, as `main` does at startup.
    h.cx.update(mail_classifier::theme::init);
    let window = h.settings_open_page(APPEARANCE);
    let (_, light, dark) = h.read(|a| a.theme_preferences());

    h.settings_click("mode-light");
    assert_eq!(h.read(|a| a.theme_preferences()).0, ThemeMode::Light);
    assert_eq!(theme_name(&mut h, window), light, "the light theme is live");

    h.settings_click("mode-dark");
    assert_eq!(h.read(|a| a.theme_preferences()).0, ThemeMode::Dark);
    assert_eq!(theme_name(&mut h, window), dark, "the dark theme is live");

    h.settings_click("mode-system");
    assert_eq!(h.read(|a| a.theme_preferences()).0, ThemeMode::System);
    assert_eq!(h.read(|a| a.theme_preferences()).1, light, "the light pick survives the round trip");
}

#[gpui_kit::gpui::test]
fn the_picker_of_the_current_mode_offers_that_appearance_theme(cx: &mut TestAppContext) {
    let mut h = app(cx);
    // The app's own themes ship as built-ins; register them so the pickers are the ones a
    // user really sees.
    h.cx.update(mail_classifier::theme::init);
    let window = h.settings_open_page(APPEARANCE);
    let dark_names = h
        .cx
        .update_window(window, |_, _, cx| mail_classifier::theme::names_for(cx, false))
        .unwrap();
    let light_names = h
        .cx
        .update_window(window, |_, _, cx| mail_classifier::theme::names_for(cx, true))
        .unwrap();
    assert!(dark_names.len() > 1, "more than one dark theme to pick from");

    // In dark mode the first picker lists the dark themes, and picking one applies it.
    h.settings_click("mode-dark");
    h.settings_pick_option(0, 1, 1);
    assert_eq!(h.read(|a| a.theme_preferences()).2, dark_names[1].to_string());
    assert_eq!(theme_name(&mut h, window), dark_names[1].to_string());

    // The light mode shows the light themes instead, and the dark pick is remembered.
    h.settings_click("mode-light");
    assert_eq!(theme_name(&mut h, window), light_names[0].to_string());
    assert_eq!(h.read(|a| a.theme_preferences()).2, dark_names[1].to_string());
}

#[gpui_kit::gpui::test]
fn a_chosen_theme_survives_closing_the_settings_window(cx: &mut TestAppContext) {
    let mut h = app(cx);
    h.cx.update(mail_classifier::theme::init);
    let window = h.settings_open_page(APPEARANCE);
    let dark_names = h
        .cx
        .update_window(window, |_, _, cx| mail_classifier::theme::names_for(cx, false))
        .unwrap();
    h.settings_click("mode-dark");
    h.settings_pick_option(0, 1, 1);

    h.settings_click("settings-close");
    let window = h.settings_open_page(APPEARANCE);
    assert_eq!(h.read(|a| a.theme_preferences()).2, dark_names[1].to_string());
    assert_eq!(theme_name(&mut h, window), dark_names[1].to_string(), "still the chosen theme");
}

#[gpui_kit::gpui::test]
fn the_pane_layout_control_switches_the_panes(cx: &mut TestAppContext) {
    let mut h = app(cx);
    h.settings_open_page(APPEARANCE);
    h.settings_click("layout-stacked");
    assert_eq!(h.read(|a| a.panes.orientation()), Orientation::Stacked);
    h.settings_click("layout-side-by-side");
    assert_eq!(h.read(|a| a.panes.orientation()), Orientation::SideBySide);
}

#[gpui_kit::gpui::test]
fn the_sender_avatar_switch_flips_the_setting(cx: &mut TestAppContext) {
    let mut h = app(cx);
    h.settings_open_page(APPEARANCE);
    assert!(h.read(|a| a.tab_avatars), "on by default");
    h.settings_click_in(0, 4, "check");
    assert!(!h.read(|a| a.tab_avatars));
}

#[gpui_kit::gpui::test]
fn preview_lines_stop_at_zero_and_five(cx: &mut TestAppContext) {
    let mut h = app(cx);
    h.settings_open_page(INBOX);
    assert_eq!(h.read(|a| a.preview_lines), 2);

    for _ in 0..5 {
        h.settings_click_in(0, 1, "increment");
    }
    assert_eq!(h.read(|a| a.preview_lines), 5);
    h.settings_click_in(0, 1, "increment");
    assert_eq!(h.read(|a| a.preview_lines), 5, "five lines is the ceiling");

    for _ in 0..8 {
        h.settings_click_in(0, 1, "decrement");
    }
    assert_eq!(h.read(|a| a.preview_lines), 0, "zero lines is the floor");
    h.settings_click_in(0, 1, "decrement");
    assert_eq!(h.read(|a| a.preview_lines), 0);
}

#[gpui_kit::gpui::test]
fn the_follow_up_row_sets_the_timeout(cx: &mut TestAppContext) {
    let mut h = app(cx);
    h.settings_open_page(INBOX);
    assert_eq!(h.read(|a| a.mailbox.follow_up_timeout()), 3 * DAY);

    h.settings_click_in(0, 2, "decrement");
    assert_eq!(h.read(|a| a.mailbox.follow_up_timeout()), 2 * DAY);

    h.settings_click_in(0, 2, "increment");
    h.settings_click_in(0, 2, "increment");
    assert_eq!(h.read(|a| a.mailbox.follow_up_timeout()), 4 * DAY);
}

#[gpui_kit::gpui::test]
fn the_group_switch_reaches_the_inbox(cx: &mut TestAppContext) {
    let mut h = app(cx);
    h.settings_open_page(INBOX);
    h.settings_click_in(0, 0, "check");
    assert!(h.read(|a| a.group_threads));
}

#[gpui_kit::gpui::test]
fn lowering_the_follow_up_row_resurfaces_a_waiting_thread_on_tick(cx: &mut TestAppContext) {
    let mut h = app(cx);
    // Reply and expect an answer: the thread now waits on the default 3-day timeout.
    h.app.update(h.cx, |a, _| a.mailbox.send_reply_at(1, "on it".into(), true, harness::NOON));
    h.advance(DAY);
    assert!(!h.has_tag(1, Tag::FollowUp), "three days of patience: one day is not enough");

    h.settings_open_page(INBOX);
    h.settings_click_in(0, 2, "decrement");
    h.settings_click_in(0, 2, "decrement");
    assert_eq!(h.read(|a| a.mailbox.follow_up_timeout()), DAY);
    h.settings_click("settings-close");

    h.tick();
    assert_eq!(h.state_of(1), TriageState::Inbox);
    assert!(h.has_tag(1, Tag::FollowUp), "the overdue thread is flagged for follow-up");
    assert!(!h.has_tag(1, Tag::AwaitingReply), "resurfacing clears Awaiting Reply");
}