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
fn checkbox_click_toggles_selection_without_opening(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let ids = h.ids();
    h.click(("row-check", 0usize));
    h.click(("row-check", 1usize));
    assert_eq!(h.read(|a| a.triage.selected()), vec![ids[0], ids[1]]);
    assert_eq!(h.read(|a| a.opened()), None);
    h.click(("row-check", 0usize));
    assert_eq!(h.read(|a| a.triage.selected()), vec![ids[1]]);
}

#[gpui_kit::gpui::test]
fn done_button_marks_selection_and_undo_button_restores(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let start = (h.count(Inbox), h.count(Done));
    let ids = h.ids();
    h.click(("row-check", 0usize));
    h.click(("row-check", 1usize));
    h.click("btn-done");
    assert_eq!(h.state_of(ids[0]), Done);
    assert_eq!(h.state_of(ids[1]), Done);
    assert_eq!((h.count(Inbox), h.count(Done)), (start.0 - 2, start.1 + 2));
    h.click("btn-undo");
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
fn palette_opens_and_runs_a_clicked_command(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.click("btn-palette");
    assert!(h.read(|a| a.palette_open()));
    let ix = commands().iter().position(|c| c.name == "Show waiting").unwrap();
    h.click(("command", ix));
    assert!(!h.read(|a| a.palette_open()));
    assert_eq!(h.read(|a| a.triage.view), Waiting);
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

#[gpui_kit::gpui::test]
fn reply_button_opens_compose_and_cancel_closes_it(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.click("btn-reply");
    assert!(h.read(|a| a.compose_open()));
    h.click("compose-cancel");
    assert!(!h.read(|a| a.compose_open()));
}

#[gpui_kit::gpui::test]
fn settings_and_help_buttons_toggle_their_panels(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.click("btn-settings");
    assert!(h.read(|a| a.settings_open()));
    h.click("settings-close");
    assert!(!h.read(|a| a.settings_open()));
    h.click("btn-help");
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
