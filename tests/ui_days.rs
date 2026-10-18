//! Day headers: collapsing a day takes its rows out of navigation and selection.

use std::rc::Rc;

use gpui_kit::{
    Action, AnyWindowHandle, AppContext, Bounds, ElementId, Entity, Focusable, Point,
    TestAppContext, WindowBounds, WindowOptions, base::Root, px, size,
};
use gpui_kit::test::TestWindowExt;
use mail_classifier::app::MailApp;
use mail_classifier::app::actions::{ToggleDay, bind_keys};
use mail_classifier::clock::{Clock, FakeClock, Timestamp};
use mail_classifier::model::{Mailbox, MessageId};
use mail_classifier::tz::Utc;

/// 2026-09-29 12:00 UTC.
const NOON: Timestamp = 1_790_683_200;

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
    let clock = Rc::new(FakeClock::new(NOON));
    let c2: Rc<dyn Clock> = clock.clone();
    let mailbox = mailbox();
    let (window, app) = cx.update(|cx| {
        let (window, content) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(1200.), px(800.)),
                })),
                ..Default::default()
            },
            cx,
            |window, cx| {
                let view = cx.new(|cx| MailApp::new_with_clock(mailbox, c2, Rc::new(Utc), window, cx));
                window.focus(&view.focus_handle(cx), cx);
                view
            },
        )
        .expect("open window");
        (window.downcast::<Root>().expect("root").into(), content)
    });
    let h = Harness { cx, window, app };
    h.cx.run_until_parked();
    h
}

fn msg(id: u32, day: u32, hour: u32) -> String {
    format!(
        r#"{{"id":{id},"thread_id":{id},"from_name":"s{id}","from_email":"s{id}@x.com","to":"you@example.com","subject":"Subject {id}","body":"Hello there.","received":"2026-09-{day:02}T{hour:02}:00:00Z","state":"Inbox"}}"#
    )
}

/// Newest first: 5, 4 (Today), 3 (Yesterday), 2, 1 (Sep 27).
fn mailbox() -> Mailbox {
    let msgs = [msg(1, 27, 9), msg(2, 27, 11), msg(3, 28, 9), msg(4, 29, 9), msg(5, 29, 10)];
    Mailbox::from_json(&format!("[{}]", msgs.join(","))).expect("valid mailbox json")
}

impl Harness<'_> {
    fn keys(&mut self, keys: &str) {
        self.cx.simulate_keystrokes(self.window, keys);
        self.cx.run_until_parked();
    }
    fn dispatch(&mut self, action: impl Action + Clone) {
        self.cx.dispatch_action(self.window, action);
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
    fn ids(&mut self) -> Vec<MessageId> {
        self.read(|a| a.visible_ids())
    }
    fn cursor(&mut self) -> Option<MessageId> {
        self.read(|a| a.cursor_message())
    }
}

#[gpui_kit::gpui::test]
fn collapsing_a_day_by_click_skips_its_rows_in_navigation(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    assert_eq!(h.ids(), vec![5, 4, 3, 2, 1]);
    h.keys("j");
    assert_eq!(h.cursor(), Some(4));
    h.click(("day", 1usize)); // Yesterday
    assert_eq!(h.ids(), vec![5, 4, 2, 1]);
    assert_eq!(h.cursor(), Some(4), "the cursor row did not move");
    h.keys("j");
    assert_eq!(h.cursor(), Some(2), "j steps over the collapsed day");
    h.keys("k");
    assert_eq!(h.cursor(), Some(4));
    h.click(("day", 1usize));
    assert_eq!(h.ids(), vec![5, 4, 3, 2, 1], "a second click expands it again");
}

#[gpui_kit::gpui::test]
fn the_toggle_day_action_collapses_the_cursor_rows_day(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.dispatch(ToggleDay);
    assert_eq!(h.ids(), vec![3, 2, 1], "Today is collapsed");
    assert_eq!(h.cursor(), Some(3), "the cursor lands on the next visible row");
    h.click(("day", 0usize));
    assert_eq!(h.ids(), vec![5, 4, 3, 2, 1]);
}

#[gpui_kit::gpui::test]
fn collapsing_deselects_hidden_rows_and_select_all_ignores_them(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.keys("x");
    assert_eq!(h.read(|a| a.triage().selected()), vec![5]);
    h.click(("day", 0usize));
    assert!(h.read(|a| a.triage().selected().is_empty()), "hidden rows are not selected");
    h.keys("x j x");
    let mut selected = h.read(|a| a.triage().selected());
    selected.sort();
    assert_eq!(selected, vec![2, 3]);
}

#[gpui_kit::gpui::test]
fn collapsing_everything_leaves_the_headers(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    for day in 0..3usize {
        h.click(("day", day));
    }
    assert!(h.ids().is_empty());
    assert_eq!(h.read(|a| a.mailbox.collapsed_days().len()), 3);
    h.click(("day", 2usize));
    assert_eq!(h.ids(), vec![2, 1]);
}
