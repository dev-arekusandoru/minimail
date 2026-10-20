//! Headless UI tests for the reader's tabs: preview replacement, pinning, switching, closing,
//! and how triage and sessions treat them. Mailboxes are built inline.

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, ElementId, TestAppContext};
use mail_classifier::model::{Mailbox, MessageId};

#[path = "ui_ext/harness.rs"]
#[allow(dead_code)]
mod harness;
use harness::{Harness, harness_with, json, msg};

/// Thread 1: messages 1, 2 ("Plans"); thread 2: message 3 ("Lunch"); thread 3: message 4
/// ("Budget"). Inbox order, newest first: 4, 3, 2, 1.
fn mailbox() -> Mailbox {
    Mailbox::from_json(&json(&[
        msg(1, 1, "alice@example.com", "Plans", 1, "Inbox"),
        msg(2, 1, "bob@example.com", "Re: Plans", 2, "Inbox"),
        msg(3, 2, "cy@example.com", "Lunch", 3, "Inbox"),
        msg(4, 3, "di@example.com", "Budget", 4, "Inbox"),
    ]))
    .expect("valid mailbox json")
}

impl Harness<'_> {
    fn row(&mut self, id: MessageId) {
        self.click(("row", id as usize));
    }
    fn double_click(&mut self, id: impl Into<ElementId>) {
        let id = id.into();
        self.cx
            .update_window(self.window, |_, window, cx| window.double_click(id, cx))
            .unwrap();
        self.cx.run_until_parked();
    }
    fn exists(&mut self, id: impl Into<ElementId>) -> bool {
        let id = id.into();
        self.cx
            .update_window(self.window, |_, window, cx| {
                window.render_frame(cx);
                window.try_find(id).is_some()
            })
            .expect("window alive")
    }
    /// `(thread, pinned)` of every tab, left to right.
    fn tabs(&mut self) -> Vec<(u32, bool)> {
        self.read(|a| a.tabs.tabs().iter().map(|t| (t.thread, t.pinned)).collect())
    }
    fn opened(&mut self) -> Option<MessageId> {
        self.read(|a| a.opened())
    }
}

#[gpui_kit::gpui::test]
fn opening_another_message_replaces_the_preview_tab(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox());
    h.row(3);
    assert_eq!(h.tabs(), vec![(2, false)]);
    h.row(4);
    assert_eq!(h.tabs(), vec![(3, false)], "the preview was replaced, not added to");
    assert_eq!(h.opened(), Some(4));
}

#[gpui_kit::gpui::test]
fn double_clicking_a_tab_pins_it_and_the_next_open_adds_a_tab(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox());
    h.row(3);
    h.double_click(("reader-tab", 2usize));
    assert_eq!(h.tabs(), vec![(2, true)]);
    h.row(4);
    assert_eq!(h.tabs(), vec![(2, true), (3, false)]);
    assert_eq!(h.opened(), Some(4));
}

#[gpui_kit::gpui::test]
fn a_click_inside_the_message_pins_the_tab(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox());
    h.row(3);
    assert_eq!(h.tabs(), vec![(2, false)]);
    h.click("reader-body");
    assert_eq!(h.tabs(), vec![(2, true)]);
}

#[gpui_kit::gpui::test]
fn enter_on_the_previewed_message_pins_it(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox());
    h.row(3);
    h.keys("enter");
    assert_eq!(h.tabs(), vec![(2, true)]);
    // Enter on a message that is not open yet only previews it.
    h.keys("k");
    h.keys("enter");
    assert_eq!(h.tabs(), vec![(2, true), (3, true)], "k already previewed it; enter pins it");
}

#[gpui_kit::gpui::test]
fn expanding_a_collapsed_thread_message_pins_the_tab(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox());
    h.row(2);
    assert_eq!(h.tabs(), vec![(1, false)]);
    h.click(("reader-thread-msg", 1usize));
    assert_eq!(h.tabs(), vec![(1, true)]);
    assert!(h.read(|a| a.reader_expanded(1)));
}

#[gpui_kit::gpui::test]
fn opening_a_message_of_a_tabbed_thread_reuses_its_tab(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox());
    h.row(2);
    h.row(1);
    assert_eq!(h.tabs(), vec![(1, false)]);
    assert_eq!(h.opened(), Some(1), "the tab now shows the message that was opened");
}

#[gpui_kit::gpui::test]
fn cmd_w_closes_the_active_tab_and_falls_back_to_a_neighbour(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox());
    for id in [2, 3, 4] {
        h.row(id);
        h.keys("enter");
    }
    assert_eq!(h.tabs(), vec![(1, true), (2, true), (3, true)]);
    h.keys("ctrl-shift-tab");
    assert_eq!(h.opened(), Some(3));
    h.keys("cmd-w");
    assert_eq!(h.tabs(), vec![(1, true), (3, true)]);
    assert_eq!(h.opened(), Some(4), "the right neighbour takes over");
    h.keys("cmd-w cmd-w");
    assert_eq!(h.tabs(), vec![]);
    assert_eq!(h.opened(), None);
    assert!(!h.exists(("reader-tab", 1usize)));
}

#[gpui_kit::gpui::test]
fn the_close_button_closes_only_its_tab(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox());
    h.row(3);
    h.keys("enter");
    h.row(4);
    h.click(("reader-tab-close", 2usize));
    assert_eq!(h.tabs(), vec![(3, false)]);
    assert_eq!(h.opened(), Some(4), "closing another tab does not switch to it");
}

#[gpui_kit::gpui::test]
fn switching_tabs_moves_the_list_cursor_but_not_the_selection(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox());
    h.row(3);
    h.keys("enter");
    h.row(4);
    h.keys("x");
    let selected = h.read(|a| a.triage().selected());
    assert_eq!(selected, vec![4]);

    h.click(("reader-tab", 2usize));
    assert_eq!(h.opened(), Some(3));
    assert_eq!(h.cursor(), Some(3));
    assert_eq!(h.read(|a| a.triage().selected()), selected);

    h.keys("ctrl-tab");
    assert_eq!((h.opened(), h.cursor()), (Some(4), Some(4)));
    h.keys("cmd-shift-[");
    assert_eq!((h.opened(), h.cursor()), (Some(3), Some(3)));
    h.keys("cmd-shift-]");
    assert_eq!(h.opened(), Some(4));
}

#[gpui_kit::gpui::test]
fn a_pinned_tab_stays_when_its_thread_leaves_the_list(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox());
    h.row(4);
    h.keys("enter e");
    // The cursor moved on to the next message, so the reader previews it beside the pinned tab.
    assert_eq!(h.tabs(), vec![(3, true), (2, false)], "a pinned tab stays open");
    assert_eq!(h.read(|a| a.cursor_message()), h.opened());
}

#[gpui_kit::gpui::test]
fn tabs_keep_their_thread_state_while_another_tab_is_active(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox());
    h.row(2);
    h.keys("shift-o");
    assert!(h.read(|a| a.reader_expanded(1)));
    h.row(4);
    assert!(h.read(|a| a.reader_expanded(1)), "thread 1 keeps its expansion in the background");
    h.click(("reader-tab", 1usize));
    assert!(h.read(|a| a.reader_expanded(1)));
    assert_eq!(h.opened(), Some(2));
}

#[gpui_kit::gpui::test]
fn a_session_hides_the_tabs_and_they_return_afterwards(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox());
    h.row(3);
    h.keys("enter");
    assert!(h.exists(("reader-tab", 2usize)));
    h.keys("t");
    assert!(!h.exists(("reader-tab", 2usize)), "the Start screen takes the window too");
    h.keys("enter");
    assert_eq!(h.opened(), Some(4), "the session shows its own message");
    assert!(!h.exists(("reader-tab", 2usize)), "no tab bar during a session");
    assert_eq!(h.tabs(), vec![(2, true)], "the tabs are untouched");
    h.keys("escape");
    assert!(h.exists(("reader-tab", 2usize)));
    assert_eq!(h.opened(), Some(3));
}

#[gpui_kit::gpui::test]
fn the_avatar_setting_controls_the_tab_icon(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox());
    h.row(3);
    assert!(h.exists(("reader-tab-avatar", 2usize)), "on by default");
    h.app.update(h.cx, |a, cx| {
        a.tab_avatars = false;
        cx.notify();
    });
    h.cx.run_until_parked();
    assert!(!h.exists(("reader-tab-avatar", 2usize)));
    assert!(h.exists(("reader-tab", 2usize)), "the tab itself stays");
}

#[gpui_kit::gpui::test]
fn tab_avatar_title_and_close_are_evenly_spaced(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox());
    h.row(3);
    let (avatar, title, close) = h
        .cx
        .update_window(h.window, |_, window, cx| {
            window.render_frame(cx);
            (
                window.find(("reader-tab-avatar", 2usize)).bounds(),
                window.find(("reader-tab", 2usize)).bounds(),
                window.find(("reader-tab-close", 2usize)).bounds(),
            )
        })
        .unwrap();
    let before = f32::from(title.left() - avatar.right());
    let after = f32::from(close.left() - title.right());
    assert!((before - after).abs() <= 1., "gaps {before} vs {after}");
    assert!(before <= 8., "gap {before} too wide");
    assert!(f32::from(avatar.size.height) >= 16., "avatar clipped: {avatar:?}");
    let (a, c) = (avatar.center().y, close.center().y);
    assert!((f32::from(a - c)).abs() <= 1., "not vertically aligned");
}

#[gpui_kit::gpui::test]
fn moving_the_cursor_with_a_tab_open_updates_the_preview_in_place(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox());
    h.row(4);
    h.keys("j");
    assert_eq!(h.read(|a| a.cursor_message()), Some(3));
    assert_eq!(h.opened(), Some(3));
    assert_eq!(h.tabs(), vec![(2, false)]);
}

#[gpui_kit::gpui::test]
fn moving_the_cursor_beside_a_pinned_tab_opens_a_preview(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox());
    h.row(4);
    h.double_click(("reader-tab", 3usize));
    h.keys("j");
    assert_eq!(h.tabs(), vec![(3, true), (2, false)]);
    assert_eq!(h.opened(), Some(3));
}

#[gpui_kit::gpui::test]
fn moving_the_cursor_with_no_tab_open_opens_nothing(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox());
    h.keys("j");
    assert_eq!(h.tabs(), vec![]);
    assert_eq!(h.opened(), None);
}

#[gpui_kit::gpui::test]
fn clicking_a_tab_moves_the_cursor_to_its_message(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox());
    h.row(4);
    h.double_click(("reader-tab", 3usize));
    h.row(3);
    assert_eq!(h.read(|a| a.cursor_message()), Some(3));
    h.click(("reader-tab", 3usize));
    assert_eq!(h.opened(), Some(4));
    assert_eq!(h.read(|a| a.cursor_message()), Some(4));
}

/// With no tabs open the reader group must not draw the dock's empty tab strip, and its
/// empty state must actually have an icon rather than a blank frame.
#[gpui_kit::gpui::test]
fn no_tabs_means_no_tab_strip_and_the_empty_state_icon_is_drawn(cx: &mut TestAppContext) {
    use gpui_kit::AssetSource as _;
    use gpui_kit::assets::IconName;

    let mut h = harness_with(cx, mailbox());
    assert_eq!(h.opened(), None, "nothing is open at startup");
    assert!(
        mail_classifier::app::icons::AppAssets
            .load(&IconName::MailOpen.path())
            .unwrap()
            .is_some(),
        "the reader's empty-state icon must be embedded in the app's assets"
    );
    h.cx.update_window(h.window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("reader-tab-strip").is_none(), "no tab strip without tabs");
        let icon = window.try_find("empty-icon").expect("the reader's empty-state icon");
        let size = icon.bounds().size;
        assert!(
            size.width > gpui_kit::px(0.) && size.height > gpui_kit::px(0.),
            "empty-state icon has no size: {size:?}"
        );
    }).unwrap();
}
