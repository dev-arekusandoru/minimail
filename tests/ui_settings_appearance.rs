//! Appearance and Inbox settings pages, driven through the real settings window.

use gpui_kit::component::ActiveTheme;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, AnyWindowHandle, ElementId, TestAppContext};
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

/// Open the settings window and select one of its sidebar pages.
fn open_page(h: &mut Harness<'_>, page: usize) -> AnyWindowHandle {
    h.keys("cmd-,");
    let window = h.read(|a| a.settings_window()).expect("settings window open");
    if page != 0 {
        h.cx.update_window(window, |_, window, cx| window.click(format!("0-{page}"), cx)).unwrap();
    }
    h.cx.run_until_parked();
    window
}

/// Click an element of the settings window and let the change settle.
fn click(h: &mut Harness<'_>, window: AnyWindowHandle, id: impl Into<ElementId>) {
    h.cx.update_window(window, |_, window, cx| window.click(id, cx)).unwrap();
    h.cx.run_until_parked();
}

/// Click `target` inside item `item` of the page's first group.
fn click_in_item(h: &mut Harness<'_>, window: AnyWindowHandle, item: usize, target: &'static str) {
    h.cx
        .update_window(window, |_, window, cx| {
            window.within("group-0").within(format!("item-{item}")).click(target, cx)
        })
        .unwrap();
    h.cx.run_until_parked();
}

/// Open the dropdown of an item and choose its `option`-th entry.
fn pick_option(h: &mut Harness<'_>, window: AnyWindowHandle, item: usize, option: usize) {
    click_in_item(h, window, item, "btn");
    h.cx
        .update_window(window, |_, window, cx| window.within("popup-menu").click(option, cx))
        .unwrap();
    h.cx.run_until_parked();
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
    let window = open_page(&mut h, APPEARANCE);
    let (_, light, dark) = h.read(|a| a.theme_preferences());

    click(&mut h, window, "mode-light");
    assert_eq!(h.read(|a| a.theme_preferences()).0, ThemeMode::Light);
    assert_eq!(theme_name(&mut h, window), light, "the light theme is live");

    click(&mut h, window, "mode-dark");
    assert_eq!(h.read(|a| a.theme_preferences()).0, ThemeMode::Dark);
    assert_eq!(theme_name(&mut h, window), dark, "the dark theme is live");

    click(&mut h, window, "mode-system");
    assert_eq!(h.read(|a| a.theme_preferences()).0, ThemeMode::System);
    assert_eq!(h.read(|a| a.theme_preferences()).1, light, "the light pick survives the round trip");
}

#[gpui_kit::gpui::test]
fn the_picker_of_the_current_mode_offers_that_appearance_theme(cx: &mut TestAppContext) {
    let mut h = app(cx);
    // The app's own themes ship as built-ins; register them so the pickers are the ones a
    // user really sees.
    h.cx.update(mail_classifier::theme::init);
    let window = open_page(&mut h, APPEARANCE);
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
    click(&mut h, window, "mode-dark");
    pick_option(&mut h, window, 1, 1);
    assert_eq!(h.read(|a| a.theme_preferences()).2, dark_names[1].to_string());
    assert_eq!(theme_name(&mut h, window), dark_names[1].to_string());

    // The light mode shows the light themes instead, and the dark pick is remembered.
    click(&mut h, window, "mode-light");
    assert_eq!(theme_name(&mut h, window), light_names[0].to_string());
    assert_eq!(h.read(|a| a.theme_preferences()).2, dark_names[1].to_string());
}

#[gpui_kit::gpui::test]
fn a_chosen_theme_survives_closing_the_settings_window(cx: &mut TestAppContext) {
    let mut h = app(cx);
    h.cx.update(mail_classifier::theme::init);
    let window = open_page(&mut h, APPEARANCE);
    let dark_names = h
        .cx
        .update_window(window, |_, _, cx| mail_classifier::theme::names_for(cx, false))
        .unwrap();
    click(&mut h, window, "mode-dark");
    pick_option(&mut h, window, 1, 1);

    click(&mut h, window, "settings-close");
    let window = open_page(&mut h, APPEARANCE);
    assert_eq!(h.read(|a| a.theme_preferences()).2, dark_names[1].to_string());
    assert_eq!(theme_name(&mut h, window), dark_names[1].to_string(), "still the chosen theme");
}

#[gpui_kit::gpui::test]
fn the_pane_layout_control_switches_the_panes(cx: &mut TestAppContext) {
    let mut h = app(cx);
    let window = open_page(&mut h, APPEARANCE);
    click(&mut h, window, "layout-stacked");
    assert_eq!(h.read(|a| a.panes.orientation()), Orientation::Stacked);
    click(&mut h, window, "layout-side-by-side");
    assert_eq!(h.read(|a| a.panes.orientation()), Orientation::SideBySide);
}

#[gpui_kit::gpui::test]
fn the_sender_avatar_switch_flips_the_setting(cx: &mut TestAppContext) {
    let mut h = app(cx);
    let window = open_page(&mut h, APPEARANCE);
    assert!(h.read(|a| a.tab_avatars), "on by default");
    click_in_item(&mut h, window, 4, "check");
    assert!(!h.read(|a| a.tab_avatars));
}

#[gpui_kit::gpui::test]
fn preview_lines_stop_at_zero_and_five(cx: &mut TestAppContext) {
    let mut h = app(cx);
    let window = open_page(&mut h, INBOX);
    assert_eq!(h.read(|a| a.preview_lines), 2);

    for _ in 0..5 {
        click_in_item(&mut h, window, 1, "increment");
    }
    assert_eq!(h.read(|a| a.preview_lines), 5);
    click_in_item(&mut h, window, 1, "increment");
    assert_eq!(h.read(|a| a.preview_lines), 5, "five lines is the ceiling");

    for _ in 0..8 {
        click_in_item(&mut h, window, 1, "decrement");
    }
    assert_eq!(h.read(|a| a.preview_lines), 0, "zero lines is the floor");
    click_in_item(&mut h, window, 1, "decrement");
    assert_eq!(h.read(|a| a.preview_lines), 0);
}

#[gpui_kit::gpui::test]
fn the_follow_up_row_sets_the_timeout(cx: &mut TestAppContext) {
    let mut h = app(cx);
    let window = open_page(&mut h, INBOX);
    assert_eq!(h.read(|a| a.mailbox.follow_up_timeout()), 3 * DAY);

    click_in_item(&mut h, window, 2, "decrement");
    assert_eq!(h.read(|a| a.mailbox.follow_up_timeout()), 2 * DAY);

    click_in_item(&mut h, window, 2, "increment");
    click_in_item(&mut h, window, 2, "increment");
    assert_eq!(h.read(|a| a.mailbox.follow_up_timeout()), 4 * DAY);
}

#[gpui_kit::gpui::test]
fn the_group_switch_reaches_the_inbox(cx: &mut TestAppContext) {
    let mut h = app(cx);
    let window = open_page(&mut h, INBOX);
    click_in_item(&mut h, window, 0, "check");
    assert!(h.read(|a| a.group_threads));
}

#[gpui_kit::gpui::test]
fn lowering_the_follow_up_row_resurfaces_a_waiting_thread_on_tick(cx: &mut TestAppContext) {
    let mut h = app(cx);
    // Reply and expect an answer: the thread now waits on the default 3-day timeout.
    h.app.update(h.cx, |a, _| a.mailbox.send_reply_at(1, "on it".into(), true, harness::NOON));
    h.advance(DAY);
    assert!(!h.has_tag(1, Tag::FollowUp), "three days of patience: one day is not enough");

    let window = open_page(&mut h, INBOX);
    click_in_item(&mut h, window, 2, "decrement");
    click_in_item(&mut h, window, 2, "decrement");
    assert_eq!(h.read(|a| a.mailbox.follow_up_timeout()), DAY);
    click(&mut h, window, "settings-close");

    h.tick();
    assert_eq!(h.state_of(1), TriageState::Inbox);
    assert!(h.has_tag(1, Tag::FollowUp), "the overdue thread is flagged for follow-up");
    assert!(!h.has_tag(1, Tag::AwaitingReply), "resurfacing clears Awaiting Reply");
}
