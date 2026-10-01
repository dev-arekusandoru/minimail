//! Headless keystroke tests for the v2 features, driving the real `MailApp` with a `FakeClock`.

use std::rc::Rc;
use gpui_kit::{AnyWindowHandle, AppContext, Bounds, Entity, Focusable, Point, TestAppContext, WindowBounds, WindowOptions, px, size};
use gpui_kit::base::Root;
use gpui_kit::test::TestWindowExt;
use mail_classifier::app::actions::{bind_keys};
use mail_classifier::app::MailApp;
use mail_classifier::clock::{Clock, FakeClock, HOUR, Timestamp};
use mail_classifier::model::{Mailbox, MessageId, Tag, TriageState};

pub const NOON: Timestamp = 1_790_683_200;
pub const MIDNIGHT: Timestamp = NOON - 12 * HOUR;

pub struct Harness<'a> {
    pub cx: &'a mut TestAppContext,
    pub window: AnyWindowHandle,
    pub app: Entity<MailApp>,
    pub clock: Rc<FakeClock>,
}

pub fn harness_with(cx: &mut TestAppContext, mailbox: Mailbox) -> Harness<'_> {
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
    let mut h = Harness { cx, window, app, clock };
    h.keys("");
    h
}

pub fn harness(cx: &mut TestAppContext) -> Harness<'_> {
    harness_with(cx, Mailbox::load_default())
}

/// One JSON message. `day` is the day of September 2026 it was received.
pub fn msg(id: u32, thread: u32, email: &str, subject: &str, day: u32, state: &str) -> String {
    let snooze = if state == "Snoozed" {
        r#""2099-12-31T00:00:00Z""#
    } else {
        "null"
    };
    format!(
        r#"{{"id":{id},"thread_id":{thread},"from_name":"{name}","from_email":"{email}","to":"you@example.com","subject":"{subject}","body":"Just checking in about this.","received":"2026-09-{day:02}T09:00:00Z","state":"{state}","snooze":{snooze}}}"#,
        name = email.split('@').next().unwrap()
    )
}

pub fn json(msgs: &[String]) -> String {
    format!("[{}]", msgs.join(","))
}

pub fn mailbox(msgs: &[String]) -> Mailbox {
    Mailbox::from_json(&json(msgs)).expect("valid mailbox json")
}

impl Harness<'_> {
    pub fn keys(&mut self, keys: &str) {
        let was_open = self.read(|a| a.modal_open());
        if !keys.is_empty() {
            self.cx.simulate_keystrokes(self.window, keys);
        }
        self.settle(was_open);
    }
    /// Type literal text into the focused input.
    pub fn type_text(&mut self, text: &str) {
        let window = self.window;
        self.cx
            .update_window(window, |_, window, cx| window.input(text, cx))
            .unwrap();
        self.cx.run_until_parked();
    }
    pub fn click(&mut self, id: impl Into<gpui_kit::ElementId>) {
        let id = id.into();
        let was_open = self.read(|a| a.modal_open());
        self.cx.update_window(self.window, |_, window, cx| window.click(id, cx)).unwrap();
        self.settle(was_open);
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
    pub fn read<R>(&mut self, f: impl FnOnce(&MailApp) -> R) -> R {
        self.cx.read_entity(&self.app, |a, _| f(a))
    }
    /// Advance the fake clock and run the app's tick.
    pub fn advance(&mut self, secs: Timestamp) {
        self.clock.advance(secs);
        self.tick();
    }
    pub fn tick(&mut self) {
        self.app.update(self.cx, |a, cx| a.tick(cx));
        self.cx.run_until_parked();
    }
    pub fn count(&mut self, s: TriageState) -> usize {
        self.read(|a| a.mailbox.count(s))
    }
    pub fn cursor(&mut self) -> Option<MessageId> {
        self.read(|a| a.triage.cursor(&a.mailbox))
    }
    pub fn state_of(&mut self, id: MessageId) -> TriageState {
        self.read(|a| a.mailbox.state_of(id).unwrap())
    }
    pub fn visible(&mut self) -> Vec<MessageId> {
        self.read(|a| a.visible_ids())
    }
    pub fn total(&mut self) -> usize {
        self.read(|a| a.mailbox.messages().len())
    }
    pub fn has_tag(&mut self, id: MessageId, tag: Tag) -> bool {
        self.read(|a| a.mailbox.tags(id).contains(&tag))
    }
    pub fn toast(&mut self) -> String {
        self.read(|a| a.toast.as_ref().map(|t| t.to_string()).unwrap_or_default())
    }
    /// Visible plus hidden messages account for the full mailbox, including all filed folders.
    pub fn assert_invariant(&mut self, ctx: &str) {
        let (visible, hidden, total) = self.read(|a| {
            (
                a.mailbox
                    .messages()
                    .iter()
                    .filter(|message| !a.mailbox.is_hidden(message.id))
                    .count(),
                a.mailbox.hidden_count(),
                a.mailbox.messages().len(),
            )
        });
        assert_eq!(visible + hidden, total, "invariant broken after {ctx}");
    }
    /// Move the cursor with `j` until it reaches `id` (current view).
    pub fn goto(&mut self, id: MessageId) {
        for _ in 0..200 {
            if self.cursor() == Some(id) {
                return;
            }
            self.keys("j");
        }
        panic!("could not reach message {id}");
    }
    pub fn send_reply(&mut self, body: &str) {
        self.keys("r");
        assert!(self.read(|a| a.compose_open()));
        self.type_text(body);
        self.keys("cmd-enter");
        assert!(!self.read(|a| a.compose_open()));
    }
}

impl Harness<'_> {
    pub fn palette_rows(&mut self) -> Vec<String> {
        self.app.read_with(self.cx, |a, cx| a.palette_rows(cx))
    }
}
