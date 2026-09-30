//! Headless mouse tests driving the real `MailApp`: clicks go through GPUI's native hit testing.

use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, ElementId, Entity, Focusable, Point, TestAppContext,
    WindowBounds, WindowOptions, base::Root, px, size,
};
use mail_classifier::app::MailApp;
use mail_classifier::app::actions::{bind_keys, commands};
use mail_classifier::model::{Mailbox, MessageId, TriageState, TriageState::*};

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
        if !keys.is_empty() {
            self.cx.simulate_keystrokes(self.window, keys);
        }
        self.cx.run_until_parked();
    }
    fn click(&mut self, id: impl Into<ElementId>) {
        let id = id.into();
        self.cx
            .update_window(self.window, |_, window, cx| window.click(id, cx))
            .expect("window alive");
        self.cx.run_until_parked();
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
    /// Open a menu by its trigger, then click one of its rows.
    fn menu(&mut self, trigger: &'static str, row: &'static str) {
        self.click(trigger);
        assert!(self.read(|a| a.menu_open()), "{trigger} must open a menu");
        self.click(row);
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
        self.read(|a| a.triage.cursor(&a.mailbox))
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
    assert_eq!(h.state_of(id), Done);
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
fn done_button_marks_selection_and_undo_menu_row_restores(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let start = (h.count(Inbox), h.count(Done));
    let ids = h.ids();
    h.click(("row-select", 0usize));
    h.click(("row-select", 1usize));
    h.click("btn-done");
    assert_eq!(h.state_of(ids[0]), Done);
    assert_eq!(h.state_of(ids[1]), Done);
    assert_eq!((h.count(Inbox), h.count(Done)), (start.0 - 2, start.1 + 2));
    h.menu("btn-more", "btn-undo");
    assert_eq!((h.count(Inbox), h.count(Done)), (start.0, start.1));
}

#[gpui_kit::gpui::test]
fn state_buttons_act_on_the_cursor_row(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let id = h.ids()[3];
    h.click(("row", id as usize));
    h.click("btn-waiting");
    assert_eq!(h.state_of(id), Waiting);
}

/// The Inbox button is only an offer outside the Inbox; inside it there is nothing to move back to.
#[gpui_kit::gpui::test]
fn inbox_button_appears_only_outside_the_inbox_view(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    assert_eq!(h.read(|a| a.triage.view), Inbox);
    assert!(!h.has("btn-inbox"), "already in the Inbox: nothing to move back to");
    h.click(("view-tab", 2usize));
    assert_eq!(h.read(|a| a.triage.view), Later);
    assert!(h.has("btn-inbox"));
    let id = h.cursor().unwrap();
    assert_eq!(h.state_of(id), Later);
    h.click("btn-inbox");
    assert_eq!(h.state_of(id), Inbox);
}

/// Nothing to act on means no contextual bar at all: no dead rows to click.
#[gpui_kit::gpui::test]
fn contextual_actions_vanish_in_the_screener(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    assert!(h.has("btn-done"), "the Inbox has a cursor row to act on");
    h.click(("view-tab", 4usize));
    assert!(h.read(|a| a.screener_open()));
    assert!(!h.has("btn-done"), "the screener brings its own controls");
    assert!(!h.has("btn-message-more"));
}

#[gpui_kit::gpui::test]
fn sidebar_tabs_switch_views(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.click(("view-tab", 3usize));
    assert_eq!(h.read(|a| a.triage.view), Done);
    h.click(("view-tab", 1usize));
    assert_eq!(h.read(|a| a.triage.view), Waiting);
    h.click(("view-tab", 4usize));
    assert!(h.read(|a| a.screener_open()));
    h.click(("view-tab", 0usize));
    assert!(!h.read(|a| a.screener_open()));
    assert_eq!(h.read(|a| a.triage.view), Inbox);
}

#[gpui_kit::gpui::test]
fn palette_opens_from_the_more_menu_and_runs_a_clicked_command(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    assert!(!h.has("btn-palette"), "Commands lives in the overflow, not on the header");
    h.menu("btn-more", "btn-palette");
    assert!(!h.read(|a| a.menu_open()), "choosing a row closes the menu");
    assert!(h.read(|a| a.palette_open()));
    let ix = commands().iter().position(|c| c.name == "Show waiting").unwrap();
    h.click(("command", ix));
    assert!(!h.read(|a| a.palette_open()));
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
    assert_eq!(h.state_of(ids[0]), Done, "the list kept its keys after a dismissal");
}

/// A click anywhere outside the open menu dismisses it without running a row.
#[gpui_kit::gpui::test]
fn a_menu_dismisses_when_the_backdrop_is_clicked(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let id = h.ids()[0];
    h.click("btn-more");
    assert!(h.read(|a| a.menu_open()));
    h.click("menu-backdrop");
    assert!(!h.read(|a| a.menu_open()), "clicking away dismisses");
    assert_eq!(h.state_of(id), Inbox, "the click ran no menu row");

    h.click("btn-message-more");
    assert!(h.read(|a| a.menu_open()), "the other menu opens too");
    h.click("menu-backdrop");
    assert!(!h.read(|a| a.menu_open()));
}

/// The sender submenu is one level deeper: open it, then choose a bulk action.
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
    h.click("btn-message-more");
    assert!(h.read(|a| a.menu_open()));
    assert!(h.has("btn-sender-actions"));
    assert!(!h.has("btn-sender-done"), "the submenu's rows are a level down");
    h.click("btn-sender-actions");
    assert!(h.has("btn-sender-done"), "clicking the submenu row opens its level");
    h.click("btn-sender-done");
    assert!(!h.read(|a| a.menu_open()), "the whole menu stack closes after a choice");

    assert_eq!(h.state_of(id), Done);
    for other in inbox.iter().filter(|other| **other != id) {
        let expected = if other_senders.contains(other) { Inbox } else { Done };
        assert_eq!(h.state_of(*other), expected, "only the cursor's sender was marked");
    }
}
/// Accept/reject only exist while a suggestion is pending on the message under the cursor.
#[gpui_kit::gpui::test]
fn ai_buttons_appear_only_while_a_suggestion_is_pending(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let ids = h.ids();
    let pending = ids
        .iter()
        .copied()
        .find(|id| h.read(|a| !a.mailbox.pending(*id).is_empty()))
        .expect("the fixture classifies at startup, so some message is pending");
    h.click(("row", pending as usize));
    assert!(h.has("btn-accept"), "a pending suggestion offers Accept");
    assert!(h.has("btn-reject"), "and Reject");
    h.click("btn-reject");
    assert!(
        h.read(|a| a.mailbox.pending(pending).is_empty()),
        "reject drops the badges"
    );
    assert!(!h.has("btn-accept"), "nothing left pending: no Accept");
    assert!(!h.has("btn-reject"), "nothing left pending: no Reject");
}

#[gpui_kit::gpui::test]
fn later_button_opens_snooze_and_preset_click_snoozes(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let id = h.cursor().unwrap();
    h.click("btn-later");
    assert!(h.read(|a| a.snooze_open()));
    h.click(("snooze-preset", 0usize));
    assert!(!h.read(|a| a.snooze_open()));
    assert_eq!(h.state_of(id), Later);
}

/// Reply belongs to an open message, so it only shows in the reader.
#[gpui_kit::gpui::test]
fn reply_button_shows_with_a_reader_and_opens_compose(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    assert!(!h.has("btn-reply"), "no message open: nothing to reply to");
    let id = h.ids()[1];
    h.click(("row", id as usize));
    assert!(h.has("btn-reply"));
    h.click("btn-reply");
    assert!(h.read(|a| a.compose_open()));
    h.click("compose-cancel");
    assert!(!h.read(|a| a.compose_open()));
}

#[gpui_kit::gpui::test]
fn settings_and_help_menu_rows_toggle_their_panels(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.menu("btn-more", "btn-settings");
    assert!(h.read(|a| a.settings_open()));
    h.click("settings-close");
    assert!(!h.read(|a| a.settings_open()));
    h.menu("btn-more", "btn-help");
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
