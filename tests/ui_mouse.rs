//! Headless mouse tests driving the real `MailApp`: clicks go through GPUI's native hit testing.

use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, ElementId, Entity, Focusable, Point, TestAppContext,
    WindowBounds, WindowOptions, base::Root, px, size,
};
use mail_classifier::app::MailApp;
use mail_classifier::app::actions::bind_keys;
use mail_classifier::model::{Mailbox, MessageId, TriageState, TriageState::*};

#[path = "common/menu.rs"]
mod menu;

struct Harness<'a> {
    cx: &'a mut TestAppContext,
    window: AnyWindowHandle,
    app: Entity<MailApp>,
}

fn harness(cx: &mut TestAppContext) -> Harness<'_> {
    cx.update(|cx| {
        gpui_kit::init(cx);
        bind_keys(cx);
    });
    let (window, app) = cx.update(|cx| {
        let (window, content) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(1400.), px(900.)),
                })),
                ..Default::default()
            },
            cx,
            |window, cx| {
                let view = cx.new(|cx| MailApp::new(Mailbox::load_default(), window, cx));
                window.focus(&view.focus_handle(cx), cx);
                view
            },
        )
        .expect("open window");
        (window.downcast::<Root>().expect("root").into(), content)
    });
    let mut h = Harness { cx, window, app };
    h.keys("");
    h
}

impl Harness<'_> {
    fn keys(&mut self, keys: &str) {
        let was_open = self.read(|a| a.modal_open());
        if !keys.is_empty() {
            self.cx.simulate_keystrokes(self.window, keys);
        }
        self.settle(was_open);
    }
    fn click(&mut self, id: impl Into<ElementId>) {
        let id = id.into();
        let was_open = self.read(|a| a.modal_open());
        self.cx
            .update_window(self.window, |_, window, cx| window.click(id, cx))
            .expect("window alive");
        self.settle(was_open);
    }
    /// Click the row of the open popup menu labelled `label`.
    fn click_row(&mut self, label: &str) {
        let was_open = self.read(|a| a.modal_open());
        menu::click_row(self.cx, self.window, label);
        self.settle(was_open);
    }
    /// Hover the row of the open popup menu labelled `label` (opens a submenu).
    fn hover_row(&mut self, label: &str) {
        menu::hover_row(self.cx, self.window, label);
    }
    /// Whether the open popup menu has a row labelled `label`.
    fn has_row(&mut self, label: &str) -> bool {
        menu::has_row(self.cx, self.window, label)
    }
    /// A dialog slides in for 250ms and its controls move meanwhile; wait that out before the
    /// next click aims at them.
    fn settle(&mut self, was_open: bool) {
        self.cx.run_until_parked();
        if !was_open && self.read(|a| a.modal_open()) {
            std::thread::sleep(std::time::Duration::from_millis(300));
            self.cx.update_window(self.window, |_, window, cx| window.draw(cx).clear(cx)).unwrap();
        }
    }
    fn read<R>(&mut self, f: impl FnOnce(&MailApp) -> R) -> R {
        self.cx.read_entity(&self.app, |a, _| f(a))
    }
    /// Whether the current frame paints `id`. Contextual controls come and go with their
    /// target, so absence is the interesting half of the assertion.
    fn has(&mut self, id: impl Into<ElementId>) -> bool {
        let id = id.into();
        self.cx
            .update_window(self.window, |_, window, cx| {
                window.render_frame(cx);
                window.try_find(id).is_some()
            })
            .expect("window alive")
    }
    fn count(&mut self, s: TriageState) -> usize {
        self.read(|a| a.mailbox.count(s))
    }
    fn ids(&mut self) -> Vec<MessageId> {
        self.read(|a| a.visible_ids())
    }
    fn index(&mut self) -> usize {
        self.read(|a| a.triage.cursor_index())
    }
    fn cursor(&mut self) -> Option<MessageId> {
        self.read(|a| a.triage.cursor(&a.mailbox, a.now()))
    }
    fn state_of(&mut self, id: MessageId) -> TriageState {
        self.read(|a| a.mailbox.state_of(id).unwrap())
    }
}

#[gpui_kit::gpui::test]
fn row_click_selects_and_opens(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let id = h.ids()[2];
    assert_eq!(h.read(|a| a.opened()), None);
    h.click(("row", id as usize));
    assert_eq!(h.index(), 2);
    assert_eq!(h.cursor(), Some(id));
    assert_eq!(h.read(|a| a.opened()), Some(id));
}

#[gpui_kit::gpui::test]
fn keys_still_work_after_a_click(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let id = h.ids()[1];
    h.click(("row", id as usize));
    h.keys("e");
    assert_eq!(h.state_of(id), Archived);
}

#[gpui_kit::gpui::test]
fn left_edge_click_toggles_selection_without_opening(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let ids = h.ids();
    h.click(("row-select", 0usize));
    h.click(("row-select", 1usize));
    assert_eq!(h.read(|a| a.triage.selected()), vec![ids[0], ids[1]]);
    assert_eq!(h.read(|a| a.opened()), None);
    h.click(("row-select", 0usize));
    assert_eq!(h.read(|a| a.triage.selected()), vec![ids[1]]);
}

#[gpui_kit::gpui::test]
fn list_header_menu_shows_with_two_selected_and_acts_on_all_of_them(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let start = (h.count(Inbox), h.count(Archived));
    let ids = h.ids();
    assert!(!h.has("btn-selection-more"), "nothing selected");
    h.click(("row-select", 0usize));
    assert!(!h.has("btn-selection-more"), "one selected message has its own menu in the reader");
    h.click(("row-select", 1usize));
    assert!(h.has("btn-selection-more"), "two selected");
    h.click("btn-selection-more");
    assert!(h.read(|a| a.menu_open()));
    assert!(!h.has_row("Summarize thread") && !h.has_row("Mute thread"), "single-message rows are left out");
    h.click_row("Archive");
    assert_eq!(h.state_of(ids[0]), Archived);
    assert_eq!(h.state_of(ids[1]), Archived);
    assert_eq!((h.count(Inbox), h.count(Archived)), (start.0 - 2, start.1 + 2));
    assert!(!h.has("btn-selection-more"), "the selection is spent");
    h.click("btn-more");
    h.click_row("Undo");
    assert_eq!((h.count(Inbox), h.count(Archived)), (start.0, start.1), "one undo step for both");
}

/// The toast after an action carries an Undo button that is the same step as `u`.
#[gpui_kit::gpui::test]
fn the_toast_undo_button_undoes_the_last_action(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let id = h.cursor().unwrap();
    h.keys("d");
    assert_eq!(h.state_of(id), Deleted);
    assert!(h.read(|a| a.toast.is_some()), "an action raises a toast");
    // The notification fades in on the executor's timers and over real frame time; run both
    // out before aiming at it.
    h.cx.executor().advance_clock(std::time::Duration::from_millis(600));
    std::thread::sleep(std::time::Duration::from_millis(450));
    h.cx.run_until_parked();
    h.cx.update_window(h.window, |_, window, cx| window.draw(cx).clear(cx)).unwrap();
    h.cx.update_window(h.window, |_, window, cx| window.draw(cx).clear(cx)).unwrap();
    h.click("toast-undo");
    assert_eq!(h.state_of(id), Inbox);
}

#[gpui_kit::gpui::test]
fn delete_menu_row_moves_message_to_trash_and_undo_restores(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let id = h.cursor().unwrap();
    let start = h.count(Deleted);
    h.click(("row", id as usize));
    h.click(("btn-message-more", id as usize));
    h.click_row("Delete");
    assert_eq!(h.state_of(id), Deleted);
    assert_eq!(h.count(Deleted), start + 1);
    h.click("btn-more");
    h.click_row("Undo");
    assert_eq!(h.state_of(id), Inbox);
    assert_eq!(h.count(Deleted), start);
}

/// A menu hangs under its trigger, right-aligned to it, and never leaves the window.
#[gpui_kit::gpui::test]
fn menus_anchor_to_their_trigger_and_stay_inside_the_window(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let id = h.ids()[0];
    h.click(("row", id as usize));
    let cases: [ElementId; 2] = [("btn-message-more", id as usize).into(), "btn-more".into()];
    for trigger in cases {
        h.click(trigger.clone());
        let (button, row, viewport) = h
            .cx
            .update_window(h.window, |_, window, cx| {
                window.render_frame(cx);
                (
                    window.find(trigger.clone()).bounds(),
                    window.within("popup-menu").find(0usize).bounds(),
                    window.viewport_size(),
                )
            })
            .expect("window alive");
        assert!(row.origin.y >= button.bottom(), "{trigger:?}: menu starts under the trigger");
        assert!(row.right() <= button.right() + px(1.), "{trigger:?}: right-aligned to the trigger");
        assert!(row.origin.x >= px(0.) && row.right() <= viewport.width, "{trigger:?}: inside the window");
        h.keys("escape");
    }
}


#[gpui_kit::gpui::test]
fn titlebar_commands_button_opens_palette_and_runs_the_highlighted_command(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    for id in ["btn-session", "btn-layout", "tb-settings", "btn-more"] {
        assert!(h.has(id), "{id}");
    }
    assert!(!h.read(|a| a.menu_open()), "the More menu's rows exist only while it is open");
    h.click("search-box");
    assert!(h.read(|a| a.palette_open()), "search opens the command/search palette");
    // The first escape clears the prefilled `/`, the second closes.
    h.keys("escape escape");
    assert!(!h.read(|a| a.palette_open()));
    h.click("btn-more");
    h.click_row("Commands…");
    assert!(h.read(|a| a.palette_open()));
    h.keys("s n o o z e d");
    h.keys("enter");
    assert!(!h.read(|a| a.palette_open()));
    assert_eq!(
        h.read(|a| a.location()),
        Some(mail_classifier::model::Location::Snoozed("personal".into()))
    );
}

/// An open menu is driven by the keyboard: `j`/`k` and the arrows move, `enter` runs the row,
/// and the list's bare keys do nothing meanwhile.
#[gpui_kit::gpui::test]
fn an_open_menu_is_navigated_with_j_and_enter(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let id = h.cursor().unwrap();
    h.keys("e");
    assert_eq!(h.state_of(id), Archived);
    h.click("btn-more");
    h.keys("e");
    assert_eq!(h.state_of(id), Archived, "bare list keys are inert while a menu is open");
    // Rows: Commands…, Undo, … — `j` twice lands on Undo.
    h.keys("j");
    h.keys("j");
    h.keys("enter");
    assert!(!h.read(|a| a.menu_open()), "running a row closes the menu");
    assert_eq!(h.state_of(id), Inbox, "enter ran Undo");
}

/// Escape must dismiss without running anything — the menu takes focus, so this is the
/// keyboard's way out, and the list keeps its keys.
#[gpui_kit::gpui::test]
fn escape_dismisses_a_menu_without_running_a_row(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let ids = h.ids();
    h.click(("row-select", 0usize));
    h.click("btn-more");
    assert!(h.read(|a| a.menu_open()));
    h.keys("escape");
    assert!(!h.read(|a| a.menu_open()));
    assert_eq!(h.state_of(ids[0]), Inbox, "dismissing ran no menu row");
    assert_eq!(
        h.read(|a| a.triage.selected()),
        vec![ids[0]],
        "escape went to the menu, not on to clearing the selection"
    );
    h.keys("e");
    assert_eq!(h.state_of(ids[0]), Archived, "the list kept its keys after a dismissal");
}

/// A click anywhere outside the open menu dismisses it without running a row.
#[gpui_kit::gpui::test]
fn a_menu_dismisses_when_the_backdrop_is_clicked(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let id = h.ids()[0];
    h.click("btn-more");
    assert!(h.read(|a| a.menu_open()));
    h.click(("row", id as usize));
    assert!(!h.read(|a| a.menu_open()), "clicking away dismisses");
    assert_eq!(h.state_of(id), Inbox, "the click ran no menu row");

    h.click(("row", id as usize));
    h.click(("btn-message-more", id as usize));
    assert!(h.read(|a| a.menu_open()), "the other menu opens too");
    h.click(("row", id as usize));
    assert!(!h.read(|a| a.menu_open()));
}

#[gpui_kit::gpui::test]
fn sender_actions_submenu_bulk_marks_one_sender(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let id = h.ids()[3];
    let sender = h.read(|a| a.mailbox.get(id).unwrap().from_email.clone());

    let inbox: Vec<MessageId> = h.ids().into_iter().filter(|id| h.state_of(*id) == Inbox).collect();
    let other_senders: Vec<MessageId> = inbox
        .iter()
        .copied()
        .filter(|other| {
            let other = *other;
            h.read(|a| a.mailbox.get(other).unwrap().from_email.clone()) != sender
        })
        .collect();
    assert!(!other_senders.is_empty(), "the fixture needs more than one sender");

    h.click(("row", id as usize));
    h.click(("btn-message-more", id as usize));
    assert!(h.read(|a| a.menu_open()));
    assert!(h.has_row("Sender actions"));
    assert!(!h.has_row("Archive from sender"), "the submenu's rows are a level down");
    h.hover_row("Sender actions");
    assert!(h.has_row("Archive from sender"), "hovering the submenu row opens its level");
    h.click_row("Archive from sender");
    assert!(!h.read(|a| a.menu_open()), "the whole menu stack closes after a choice");
    assert!(h.read(|a| a.dialog_open()), "sender-wide actions confirm first");
    h.keys("1");

    assert_eq!(h.state_of(id), Archived);
    for other in inbox.iter().filter(|other| **other != id) {
        let expected = if other_senders.contains(other) { Inbox } else { Archived };
        assert_eq!(h.state_of(*other), expected, "only the cursor's sender was marked");
    }
}

/// Accept/reject only exist in a message's menu while a suggestion is pending on it.
#[gpui_kit::gpui::test]
fn ai_rows_appear_only_while_a_suggestion_is_pending(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let ids = h.ids();
    let pending = ids
        .iter()
        .copied()
        .find(|id| h.read(|a| !a.mailbox.pending(*id).is_empty()))
        .expect("the fixture classifies at startup, so some message is pending");
    h.click(("row", pending as usize));
    h.click(("btn-message-more", pending as usize));
    assert!(h.has_row("Accept AI suggestions"), "a pending suggestion offers Accept");
    assert!(h.has_row("Reject AI suggestions"), "and Reject");
    h.click_row("Reject AI suggestions");
    assert!(
        h.read(|a| a.mailbox.pending(pending).is_empty()),
        "reject drops the badges"
    );
    h.click(("btn-message-more", pending as usize));
    assert!(h.has_row("Archive"), "the menu still opens");
    assert!(!h.has_row("Accept AI suggestions"), "nothing left pending: no Accept");
    assert!(!h.has_row("Reject AI suggestions"), "nothing left pending: no Reject");
}

#[gpui_kit::gpui::test]
fn snooze_menu_row_opens_picker_and_preset_click_snoozes(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let id = h.ids()[0];
    h.click(("row", id as usize));
    h.click(("btn-message-more", id as usize));
    h.click_row("Snooze…");
    assert!(h.read(|a| a.snooze_open()));
    h.click(("snooze-preset", 0usize));
    assert!(!h.read(|a| a.snooze_open()));
    assert_eq!(h.state_of(id), Snoozed);
}

/// Each expanded message of the reader carries its own Reply; it replies to that message.
#[gpui_kit::gpui::test]
fn reply_button_shows_with_a_reader_and_opens_compose(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let id = h.ids()[1];
    assert!(!h.has(("btn-reply", id as usize)), "no message open: nothing to reply to");
    h.click(("row", id as usize));
    assert!(h.has(("btn-reply", id as usize)));
    h.click(("btn-reply", id as usize));
    assert!(h.read(|a| a.compose_open()));
    h.click("compose-cancel");
    assert!(!h.read(|a| a.compose_open()));
}

#[gpui_kit::gpui::test]
fn titlebar_settings_and_help_buttons_toggle_their_panels(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.click("tb-settings");
    assert!(h.read(|a| a.settings_open()));
    let settings = h.read(|a| a.settings_window()).expect("the settings window is open");
    h.cx.simulate_keystrokes(settings, "escape");
    h.cx.run_until_parked();
    assert!(!h.read(|a| a.settings_open()));
    h.click("btn-more");
    h.click_row("Shortcuts");
    assert!(h.read(|a| a.help_open()));
    h.keys("escape");
    assert!(!h.read(|a| a.help_open()));
}

#[gpui_kit::gpui::test]
fn session_button_starts_a_session(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.click("btn-session");
    assert!(h.read(|a| a.session_progress()).is_some());
    h.click("session-end");
    assert!(h.read(|a| a.session_progress()).is_none());
}

#[gpui_kit::gpui::test]
fn titlebar_search_box_stays_centered_while_searching_and_resizing(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    for width in [1400., 900.] {
        h.cx.simulate_window_resize(h.window, size(px(width), px(900.)));
        h.cx.run_until_parked();
        for searching in [false, true] {
            if searching {
                h.keys("/");
                h.keys("i n b o x");
                h.keys("enter");
                assert!(h.has("search-clear"), "clear button lives inside the search box");
            }
            let b = h
                .cx
                .update_window(h.window, |_, window, cx| {
                    window.render_frame(cx);
                    window.find("search-box").bounds()
                })
                .expect("window alive");
            let center = f32::from(b.origin.x + b.size.width / 2.);
            assert!((center - width / 2.).abs() < 0.5, "width {width}, searching {searching}: center {center}");
        }
        h.keys("escape");
    }
}

/// The kit adds its own inset to the bar in fullscreen; the search box still centres on the window.
#[gpui_kit::gpui::test]
fn titlebar_search_box_stays_centered_in_fullscreen(cx: &mut TestAppContext) {
    let h = harness(cx);
    h.cx.update_window(h.window, |_, window, _| window.toggle_fullscreen()).unwrap();
    h.cx.run_until_parked();
    let fullscreen = h.cx.update_window(h.window, |_, window, _| window.is_fullscreen()).unwrap();
    assert!(fullscreen, "the test window can be made fullscreen");
    let (b, width) = h
        .cx
        .update_window(h.window, |_, window, cx| {
            window.render_frame(cx);
            (window.find("search-box").bounds(), f32::from(window.viewport_size().width))
        })
        .expect("window alive");
    let center = f32::from(b.origin.x + b.size.width / 2.);
    assert!((center - width / 2.).abs() < 0.5, "fullscreen: center {center} of {width}");
}

/// Row keycaps must stay painted for as long as a menu is open, not just on its first frame
/// (the app's key context used to be dropped once the menu opened, losing every binding).
#[gpui_kit::gpui::test]
fn menu_keycaps_stay_visible_while_the_menu_is_open(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.click("btn-more");
    for frame in 0..4 {
        h.cx.run_until_parked();
        let mut vcx = gpui_kit::VisualTestContext::from_window(h.window, h.cx);
        vcx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(vcx.debug_bounds("kbd:cmd-z").is_some(), "Undo keycap missing on frame {frame}");
        assert!(vcx.debug_bounds("kbd:cmd-,").is_some(), "Settings keycap missing on frame {frame}");
    }
}
