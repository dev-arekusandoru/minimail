//! Headless keystroke tests for group-by-thread and thread navigation.

use std::rc::Rc;

use gpui_kit::{
    Action, AnyWindowHandle, AppContext, Bounds, ElementId, Entity, Focusable, Pixels, Point,
    ScrollDelta, TestAppContext, WindowBounds, WindowOptions, base::Root, point, px, size,
};
use gpui_kit::test::TestWindowExt;
use mail_classifier::app::MailApp;
use mail_classifier::app::actions::{CyclePreviewLines, bind_keys};
use mail_classifier::app::row::{message_row_height, thread_row_height};
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
    fn dispatch(&mut self, action: impl Action + Clone) {
        self.cx.dispatch_action(self.window, action);
        self.cx.run_until_parked();
    }
    /// Laid-out bounds of a row in the real window.
    fn bounds(&mut self, id: impl Into<ElementId>) -> Bounds<Pixels> {
        self.cx
            .update_window(self.window, |_, window, _| window.find(id).bounds())
            .expect("window open")
    }
    /// Laid-out height of a row in the real window, in pixels.
    fn row_height(&mut self, id: impl Into<ElementId>) -> Pixels {
        self.bounds(id).size.height
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
        self.read(|a| a.opened())
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
    h.keys("ctrl-g");
    let rows = h.rows();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0], Row::Single(4));
    assert!(matches!(&rows[1], Row::Header { ids, expanded: false, .. } if ids == &[3, 2, 1]));
    let edge = h.bounds(("row-select", 1usize));
    assert_eq!(edge.size.width, px(10.), "thread header exposes the same left-edge hit area");
    assert_eq!(h.read(|a| a.visible_ids()), vec![4, 3, 2, 1]);
    h.keys("j");
    assert_eq!(h.cursor(), Some(3), "header stands for its newest message");
    h.keys("ctrl-g j");
    assert_eq!(h.cursor(), Some(2), "flat again: one row per message");
}

#[gpui_kit::gpui::test]
fn expand_and_collapse_with_arrows_and_enter(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, threaded());
    h.keys("ctrl-g j right");
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
    h.keys("ctrl-g j e");
    for id in [1, 2, 3] {
        assert_eq!(h.state_of(id), Archived);
    }
    assert_eq!(h.state_of(4), Inbox);
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
            msg(2, 1, "bob@x.com", "Re: Plan", 2, "Archived"),
            msg(3, 1, "ann@x.com", "Re: Plan", 3, "Inbox"),
        ]),
    );
    h.keys("ctrl-g");
    assert!(matches!(&h.rows()[0], Row::Header { ids, .. } if ids == &[3, 1]));
    h.keys("g a");
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
    h.keys("ctrl-g");
    h.read(|a| assert!(a.group_threads));
    h.keys("[");
    assert_eq!(h.opened(), Some(2));
    assert_eq!(h.cursor(), Some(2), "thread expands so the message row is focused");
    h.keys("]");
    assert_eq!(h.opened(), Some(3));
}

/// Top edge of each message of thread `ids` (as laid out in the reader), oldest id first in
/// `ids`; the focused message is found by its expanded body, the rest by their collapsed line.
fn reader_tops(h: &mut Harness<'_>, ids: &[MessageId], focused: MessageId) -> Vec<Pixels> {
    ids.iter()
        .map(|&id| {
            if id == focused {
                h.bounds("reader-body").top()
            } else {
                let expanded = h.read(move |a| a.reader_expanded(id));
                h.bounds((if expanded { "reader-thread-body" } else { "reader-thread-msg" }, id as usize)).top()
            }
        })
        .collect()
}

#[gpui_kit::gpui::test]
fn reader_renders_the_thread_chronologically_and_stepping_does_not_reorder(
    cx: &mut TestAppContext,
) {
    let msgs: Vec<String> = (1..=6u32)
        .map(|i| msg(i, 1, "ann@x.com", "Plan", i, "Inbox"))
        .collect();
    let mut h = harness_with(cx, mailbox(&msgs));
    let ids: Vec<MessageId> = (1..=6).collect();

    // Inbox lists newest first: 6,5,...; open the 5th message.
    h.keys("j enter");
    assert_eq!(h.opened(), Some(5));
    let tops = reader_tops(&mut h, &ids, 5);
    assert!(tops.windows(2).all(|w| w[0] < w[1]), "oldest first, focused one in place: {tops:?}");
    for id in ids.iter().copied() {
        assert!(!h.read(move |a| a.reader_expanded(id)), "only the opened body is expanded");
    }
    let body = h.bounds("reader-body");
    assert!(body.top() >= px(0.) && body.bottom() <= px(800.), "focused message is visible: {body:?}");

    // `]` expands the next message in place and collapses nothing; order is fixed.
    h.keys("]");
    assert_eq!(h.opened(), Some(6));
    let tops = reader_tops(&mut h, &ids, 6);
    assert!(tops.windows(2).all(|w| w[0] < w[1]), "order unchanged after ]: {tops:?}");
    assert!(h.read(|a| a.reader_expanded(5)), "the message left behind stays expanded");
    h.bounds(("reader-thread-body", 5usize));
    assert!(h.read(|a| a.reader_expanded(6)), "the target is expanded too");

    h.keys("[ [");
    assert_eq!(h.opened(), Some(4));
    let tops = reader_tops(&mut h, &ids, 4);
    assert!(tops.windows(2).all(|w| w[0] < w[1]), "order unchanged after [: {tops:?}");
    for id in [5u32, 6] {
        assert!(h.read(move |a| a.reader_expanded(id)), "{id} stays expanded after stepping away");
        h.bounds(("reader-thread-body", id as usize));
    }
    for id in [1u32, 2, 3] {
        assert!(!h.read(move |a| a.reader_expanded(id)));
        h.bounds(("reader-thread-msg", id as usize));
    }
}

#[gpui_kit::gpui::test]
fn thread_rows_size_to_their_content_while_message_rows_keep_their_height(
    cx: &mut TestAppContext,
) {
    let mut h = harness_with(cx, threaded());
    h.keys("ctrl-g");
    let message = h.row_height(("row", 4usize));
    let header = h.row_height(("thread-row", 1usize));
    assert!(header < message, "collapsed header ({header}) must be shorter than a message row ({message})");

    // Expanding leaves the header alone and gives the children full message-row height.
    h.keys("j right");
    assert_eq!(h.row_height(("thread-row", 1usize)), header, "expanding does not resize the header");
    assert_eq!(h.row_height(("row", 2usize)), message, "a child message row keeps message height");
    h.keys("left");

    // The header follows the preview setting, and stays shorter than a message row.
    for _ in 0..4 {
        h.dispatch(CyclePreviewLines);
    }
    assert_eq!(h.read(|a| a.preview_lines), 0, "cycled to Off");
    let bare_message = h.row_height(("row", 4usize));
    let bare_header = h.row_height(("thread-row", 1usize));
    assert!(bare_message < message, "message rows shrink with the preview setting");
    assert!(bare_header < header, "headers shrink with the preview setting too");
    assert!(bare_header < bare_message, "and are still not padded to message height");
    assert!(bare_header < px(40.), "a header with no preview is one compact line: {bare_header}");

    // Back to the default: measured heights match what the row functions promise.
    h.dispatch(CyclePreviewLines);
    assert_eq!(h.read(|a| a.preview_lines), 1);
    h.dispatch(CyclePreviewLines);
    assert_eq!(h.read(|a| a.preview_lines), 2);
    let message_again = h.row_height(("row", 4usize));
    let header_again = h.row_height(("thread-row", 1usize));
    assert_eq!(message_again, message, "no stale heights after cycling previews");
    assert_eq!(header_again, header);
    assert_eq!(message_again, px(message_row_height(2)));
    assert_eq!(header_again, px(thread_row_height(2)));
}

/// Forty single-message threads, so the list is far taller than the window.
fn long_list() -> Mailbox {
    let msgs: Vec<String> = (1..=40u32)
        .map(|i| msg(i, i, &format!("s{i}@x.com"), &format!("Subject {i}"), 1 + i % 28, "Inbox"))
        .collect();
    mailbox(&msgs)
}

#[gpui_kit::gpui::test]
fn keyboard_and_wheel_scroll_the_variable_height_list(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, long_list());
    let newest = h.read(|a| a.visible_ids()[0]) as usize;
    let top_row = h.bounds(("row", newest));
    assert!(top_row.bottom() < px(800.), "the first row starts inside the window: {top_row:?}");

    for _ in 0..30 {
        h.keys("j");
    }
    let cursor = h.cursor().expect("a cursor") as usize;
    let cursor_bounds = h.bounds(("row", cursor));
    assert!(
        cursor_bounds.top() > px(0.) && cursor_bounds.bottom() < px(800.),
        "j scrolled row {cursor} into view: {cursor_bounds:?}"
    );

    let before = h.bounds(("row", cursor));
    h.cx
        .update_window(h.window, |_, window, cx| {
            window.scroll(("row", cursor), ScrollDelta::Pixels(point(px(0.), px(-800.))), cx);
        })
        .expect("window open");
    h.cx.run_until_parked();
    let after = h.bounds(("row", cursor));
    assert!(after.top() < before.top(), "the wheel scrolled the list: {before:?} -> {after:?}");
}
