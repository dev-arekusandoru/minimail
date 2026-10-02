//! Headless keystroke tests for the v2 features, driving the real `MailApp` with a `FakeClock`.

use std::rc::Rc;
use gpui_kit::{AnyWindowHandle, AppContext, Bounds, Entity, Focusable, Point, TestAppContext, WindowBounds, WindowOptions, px, size};
use gpui_kit::base::Root;
use gpui_kit::test::TestWindowExt;
use mail_classifier::app::actions::{bind_keys};
use mail_classifier::app::MailApp;
use mail_classifier::clock::{Clock, FakeClock, HOUR, Timestamp};
use mail_classifier::model::{Mailbox, MessageId, Tag, TriageState};
use mail_classifier::tz::Utc;

#[path = "../common/menu.rs"]
mod menu;

pub const NOON: Timestamp = 1_790_683_200;
pub const MIDNIGHT: Timestamp = NOON - 12 * HOUR;

pub struct Harness<'a> {
    pub cx: &'a mut TestAppContext,
    pub window: AnyWindowHandle,
    pub app: Entity<MailApp>,
    pub clock: Rc<FakeClock>,
}

pub fn harness_with(cx: &mut TestAppContext, mailbox: Mailbox) -> Harness<'_> {
    harness_inner(cx, mailbox, None)
}

/// A harness whose app reads and writes preferences through `store`, as the real app does.
/// The store is never a real database: tests pass an in-memory one they share with a second
/// app to stand in for a restart.
pub fn harness_with_prefs(
    cx: &mut TestAppContext,
    mailbox: Mailbox,
    store: Rc<mail_classifier::contacts::ContactStore>,
) -> Harness<'_> {
    harness_inner(cx, mailbox, Some(store))
}

fn harness_inner(
    cx: &mut TestAppContext,
    mailbox: Mailbox,
    prefs: Option<Rc<mail_classifier::contacts::ContactStore>>,
) -> Harness<'_> {
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
                let view = cx.new(|cx| {
                    let mut app = MailApp::new_with_clock(mailbox, c2, Rc::new(Utc), window, cx);
                    if let Some(store) = prefs {
                        app.load_preferences(store, window, cx);
                    }
                    app
                });
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
    /// Click the row of the open popup menu labelled `label`.
    pub fn click_row(&mut self, label: &str) {
        let was_open = self.read(|a| a.modal_open());
        menu::click_row(self.cx, self.window, label);
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
        self.read(|a| a.triage.cursor(&a.mailbox, &a.local_now()))
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
    /// Rows of the open filter picker, top to bottom.
    pub fn picker_rows(&mut self) -> Vec<String> {
        self.app.read_with(self.cx, |a, cx| a.filter_popover_rows(cx))
    }
    /// The pills the list header shows.
    pub fn pills(&mut self) -> Vec<String> {
        self.read(|a| a.pill_texts())
    }
    /// The list header's first line, e.g. `"Inbox · 12"`.
    pub fn header(&mut self) -> String {
        self.read(|a| a.header_title(a.visible_ids().len()))
    }
    /// The query the list is showing, as text.
    pub fn query(&mut self) -> String {
        self.read(|a| a.query().describe())
    }
    /// Whether an element is in the tree of the last completed frame.
    pub fn has(&mut self, id: impl Into<gpui_kit::ElementId>) -> bool {
        let id = id.into();
        self.cx
            .update_window(self.window, |_, window, _| window.try_find(id).is_some())
            .unwrap_or(false)
    }
}

// ---------------------------------------------------------------- Settings window

impl Harness<'_> {
    /// The handle of the open settings window, if any.
    pub fn settings_window(&mut self) -> Option<AnyWindowHandle> {
        self.read(|a| a.settings_window())
    }

    /// Open the settings window with ⌘, and return its handle. Settings lives in its own
    /// window, so its elements are looked up there and not in the mail window.
    pub fn open_settings(&mut self) -> AnyWindowHandle {
        self.keys("cmd-,");
        self.settings_handle()
    }

    /// Open Settings and show `page` of its sidebar, returning the window handle.
    pub fn settings_open_page(&mut self, page: usize) -> AnyWindowHandle {
        let window = self.open_settings();
        if page != 0 {
            self.cx
                .update_window(window, |_, window, cx| window.click(format!("0-{page}"), cx))
                .expect("settings window alive");
            self.cx.run_until_parked();
        }
        window
    }

    fn settings_handle(&mut self) -> AnyWindowHandle {
        self.settings_window().expect("the settings window is open")
    }

    /// Keystrokes into the settings window.
    pub fn settings_keys(&mut self, keys: &str) {
        let window = self.settings_handle();
        self.cx.simulate_keystrokes(window, keys);
        self.cx.run_until_parked();
    }

    /// Click an element of the settings window.
    pub fn settings_click(&mut self, id: impl Into<gpui_kit::ElementId>) {
        let window = self.settings_handle();
        self.cx
            .update_window(window, |_, window, cx| window.click(id, cx))
            .expect("settings window alive");
        self.cx.run_until_parked();
    }

    /// Type text into the field the settings window has focused.
    pub fn settings_type(&mut self, text: &str) {
        let window = self.settings_handle();
        self.cx
            .update_window(window, |_, window, cx| window.input(text, cx))
            .expect("settings window alive");
        self.cx.run_until_parked();
    }

    /// Whether an element is in the settings window's current frame. A page's groups live in
    /// a measured list, so an element can take a few frames to show up: ask for a handful
    /// before answering no.
    pub fn settings_has(&mut self, id: impl Into<gpui_kit::ElementId>) -> bool {
        let id = id.into();
        let window = self.settings_handle();
        for _ in 0..8 {
            let found = self
                .cx
                .update_window(window, |_, window, cx| {
                    window.render_frame(cx);
                    window.try_find(id.clone()).is_some()
                })
                .expect("settings window alive");
            self.cx.run_until_parked();
            if found {
                return true;
            }
        }
        false
    }

    /// Click `target` inside item `item` of group `group` of the page on screen.
    pub fn settings_click_in(
        &mut self,
        group: usize,
        item: usize,
        target: impl Into<gpui_kit::ElementId>,
    ) {
        let window = self.settings_handle();
        self.cx
            .update_window(window, |_, window, cx| {
                window
                    .within(format!("group-{group}"))
                    .within(format!("item-{item}"))
                    .click(target, cx)
            })
            .expect("settings window alive");
        self.cx.run_until_parked();
    }

    /// Open the dropdown of an item and choose its `option`-th entry.
    pub fn settings_pick_option(&mut self, group: usize, item: usize, option: usize) {
        self.settings_click_in(group, item, "btn");
        let window = self.settings_handle();
        self.cx
            .update_window(window, |_, window, cx| window.within("popup-menu").click(option, cx))
            .expect("settings window alive");
        self.cx.run_until_parked();
    }

    /// The settings page the sidebar is on.
    pub fn settings_page(&mut self) -> usize {
        let panel = self.read(|a| a.settings_panel()).expect("the settings window is open");
        panel.read_with(self.cx, |panel, _| panel.page())
    }
}
