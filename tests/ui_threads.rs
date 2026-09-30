//! Headless keystroke tests for group-by-thread and thread navigation.

use std::rc::Rc;

use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, Entity, Focusable, Point, TestAppContext, WindowBounds,
    WindowOptions, base::Root, px, size,
};
use mail_classifier::app::MailApp;
use mail_classifier::app::actions::bind_keys;
use mail_classifier::clock::{Clock, FakeClock, Timestamp};
use mail_classifier::model::{Mailbox, MessageId, TriageState, TriageState::*};
use mail_classifier::threads::Row;

const NOON: Timestamp = 1_790_683_200;

struct Harness<'a> {
    cx: &'a mut TestAppContext,
    window: AnyWindowHandle,
    app: Entity<MailApp>,
}

fn harness_with(cx: &mut TestAppContext, mailbox: Mailbox) -> Harness<'_> {
    cx.update(|cx| {
        gpui_kit::init(cx);
        bind_keys(cx);
    });
    let clock = Rc::new(FakeClock::new(NOON));
    let c2: Rc<dyn Clock> = clock.clone();
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
                let view = cx.new(|cx| MailApp::new_with_clock(mailbox, c2, window, cx));
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

/// One JSON message. `day` is the day of September 2026 it was received.
fn msg(id: u32, thread: u32, email: &str, subject: &str, day: u32, state: &str) -> String {
    format!(
        r#"{{"id":{id},"thread_id":{thread},"from_name":"{name}","from_email":"{email}","to":"you@example.com","subject":"{subject}","body":"Just checking in about this.","received":"2026-09-{day:02}T09:00:00Z","state":"{state}"}}"#,
        name = email.split('@').next().unwrap()
    )
}

fn json(msgs: &[String]) -> String {
    format!("[{}]", msgs.join(","))
}

fn mailbox(msgs: &[String]) -> Mailbox {
    Mailbox::from_json(&json(msgs)).expect("valid mailbox json")
}

impl Harness<'_> {
    fn keys(&mut self, keys: &str) {
        self.cx.simulate_keystrokes(self.window, keys);
        self.cx.run_until_parked();
    }
    fn read<R>(&mut self, f: impl FnOnce(&MailApp) -> R) -> R {
        self.cx.read_entity(&self.app, |a, _| f(a))
    }
    fn state_of(&mut self, id: MessageId) -> TriageState {
        self.read(|a| a.mailbox.state_of(id).unwrap())
    }
    fn rows(&mut self) -> Vec<Row> {
        self.read(|a| a.rows())
    }
    fn cursor(&mut self) -> Option<MessageId> {
        self.read(|a| a.cursor_message())
    }
    fn opened(&mut self) -> Option<MessageId> {
        self.read(|a| a.opened)
    }
    fn toast(&mut self) -> String {
        self.read(|a| a.toast.as_ref().map(|t| t.to_string()).unwrap_or_default())
    }
}

/// Thread 1: ids 1,2,3 (days 1-3); id 4 alone (day 4). Inbox order: 4,3,2,1.
fn threaded() -> Mailbox {
    mailbox(&[
        msg(1, 1, "ann@x.com", "Plan", 1, "Inbox"),
        msg(2, 1, "bob@x.com", "Re: Plan", 2, "Inbox"),
        msg(3, 1, "ann@x.com", "Re: Plan", 3, "Inbox"),
        msg(4, 2, "cy@x.com", "Lunch", 4, "Inbox"),
    ])
}

#[gpui_kit::gpui::test]
fn grouping_shows_one_row_per_thread_and_off_restores_messages(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, threaded());
    assert!(h.rows().is_empty(), "off by default");
    h.keys("g");
    let rows = h.rows();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0], Row::Single(4));
    assert!(matches!(&rows[1], Row::Header { ids, expanded: false, .. } if ids == &[3, 2, 1]));
    assert_eq!(h.read(|a| a.visible_ids()), vec![4, 3, 2, 1]);
    h.keys("j");
    assert_eq!(h.cursor(), Some(3), "header stands for its newest message");
    h.keys("g j");
    assert_eq!(h.cursor(), Some(2), "flat again: one row per message");
}

#[gpui_kit::gpui::test]
fn expand_and_collapse_with_arrows_and_enter(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, threaded());
    h.keys("g j right");
    assert_eq!(h.rows().len(), 5);
    h.keys("j j");
    assert_eq!(h.cursor(), Some(2));
    h.keys("left");
    assert_eq!(h.rows().len(), 2);
    assert_eq!(h.cursor(), Some(3), "cursor returns to the header");
    h.keys("enter");
    assert_eq!(h.rows().len(), 5, "enter expands a collapsed thread");
    assert_eq!(h.opened(), Some(3));
}

#[gpui_kit::gpui::test]
fn actions_apply_to_the_whole_thread_and_undo_restores(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, threaded());
    h.keys("g j e");
    for id in [1, 2, 3] {
        assert_eq!(h.state_of(id), Done);
    }
    assert_eq!(h.state_of(4), Inbox);
    h.keys("u");
    for id in 1..=4 {
        assert_eq!(h.state_of(id), Inbox);
    }
    // Selection over thread rows covers every message too.
    h.keys("x k x w");
    for id in 1..=4 {
        assert_eq!(h.state_of(id), Waiting);
    }
    h.keys("u");
    for id in 1..=4 {
        assert_eq!(h.state_of(id), Inbox);
    }
}

#[gpui_kit::gpui::test]
fn split_thread_shows_only_the_panels_messages(cx: &mut TestAppContext) {
    let mut h = harness_with(
        cx,
        mailbox(&[
            msg(1, 1, "ann@x.com", "Plan", 1, "Inbox"),
            msg(2, 1, "bob@x.com", "Re: Plan", 2, "Done"),
            msg(3, 1, "ann@x.com", "Re: Plan", 3, "Inbox"),
        ]),
    );
    h.keys("g");
    assert!(matches!(&h.rows()[0], Row::Header { ids, .. } if ids == &[3, 1]));
    h.keys("4");
    assert_eq!(h.rows(), vec![Row::Single(2)]);
}

#[gpui_kit::gpui::test]
fn next_and_previous_in_thread_work_flat_and_grouped(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, threaded());
    // Flat: open message 3 (second row), walk back and forth.
    h.keys("j enter");
    assert_eq!(h.opened(), Some(3));
    h.keys("[");
    assert_eq!((h.opened(), h.cursor()), (Some(2), Some(2)));
    h.keys("[");
    assert_eq!(h.opened(), Some(1));
    h.keys("[");
    assert_eq!(h.opened(), Some(1));
    assert!(h.toast().contains("First"));
    h.keys("] ] ]");
    assert_eq!(h.opened(), Some(3));
    assert!(h.toast().contains("Last"));

    // Grouped: works from the list selection without opening first.
    h.keys("g");
    h.read(|a| assert!(a.group_threads));
    h.keys("[");
    assert_eq!(h.opened(), Some(2));
    assert_eq!(h.cursor(), Some(2), "thread expands so the message row is focused");
    h.keys("]");
    assert_eq!(h.opened(), Some(3));
}
