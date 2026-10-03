//! Headless keystroke tests for the v2 features, driving the real `MailApp` with a `FakeClock`.

use gpui_kit::base::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, Entity, Focusable, Point, TestAppContext, WindowBounds,
    WindowOptions, px, size,
};
use mail_classifier::app::MailApp;
use mail_classifier::app::actions::bind_keys;
use mail_classifier::clock::{Clock, FakeClock, HOUR, Timestamp};
use mail_classifier::model::{Location, Mailbox, MessageId, Tag, TriageState};
use mail_classifier::outgoing::{DraftId, DraftKind, HOLD_SECONDS};
use mail_classifier::tz::Utc;
use mail_classifier::app::composer_state::LOCAL_DEBOUNCE_SECS;
use std::path::PathBuf;
use std::rc::Rc;

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
                    // The real worker, over stores that keep nothing: the composer paths a
                    // test drives are the ones the app runs, with no draft left on disk.
                    app.attach_composer_in_memory(window, cx);
                    app
                });
                window.focus(&view.focus_handle(cx), cx);
                view
            },
        )
        .expect("open window");
        (window.downcast::<Root>().expect("root").into(), content)
    });
    let mut h = Harness {
        cx,
        window,
        app,
        clock,
    };
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

/// Give the composer worker — a real OS thread — a moment of real time to finish.
///
/// The app reads the worker's event queue on its own beat, and the test clock is not that
/// thread's clock, so waiting has to be in wall-clock time. It is bounded so a worker that
/// never answers fails the test instead of hanging the suite.
fn settle_worker() {
    std::thread::sleep(std::time::Duration::from_millis(20));
}

impl Harness<'_> {
    pub fn keys(&mut self, keys: &str) {
        let was_open = self.read(|a| a.modal_open());
        if !keys.is_empty() {
            self.cx.simulate_keystrokes(self.window, keys);
        }
        self.settle_modal(was_open);
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
        self.cx
            .update_window(self.window, |_, window, cx| window.click(id, cx))
            .unwrap();
        self.settle_modal(was_open);
    }
    /// Click the row of the open popup menu labelled `label`.
    pub fn click_row(&mut self, label: &str) {
        let was_open = self.read(|a| a.modal_open());
        menu::click_row(self.cx, self.window, label);
        self.settle_modal(was_open);
    }
    /// A dialog slides in for 250ms and its controls move meanwhile; wait that out before the
    /// next click aims at them.
    fn settle_modal(&mut self, was_open: bool) {
        self.cx.run_until_parked();
        if !was_open && self.read(|a| a.modal_open()) {
            std::thread::sleep(std::time::Duration::from_millis(300));
            self.cx
                .update_window(self.window, |_, window, cx| window.draw(cx).clear(cx))
                .unwrap();
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
        self.cx
            .executor()
            .advance_clock(std::time::Duration::from_millis(50));
        self.app.update(self.cx, |a, cx| a.tick(cx));
        self.cx.run_until_parked();
    }
    pub fn count(&mut self, s: TriageState) -> usize {
        self.read(|a| a.mailbox.count(s))
    }
    /// Every state's message count, so a test can assert that nothing moved.
    pub fn counts(&mut self) -> [usize; 5] {
        TriageState::ALL.map(|s| self.count(s))
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

    /// Open a durable inline reply before typing its body.
    pub fn compose_reply(&mut self, body: &str) {
        self.keys("r");
        self.await_composer("the reply opens inline", |a| a.inline_compose_open());
        self.type_text(body);
    }

    /// Keep ticking until `done` says the composer has caught up.
    ///
    /// The worker answers on its own thread, so a beat can find nothing new to do until it
    /// has finished: each round advances the test clock and then waits in real time. What a
    /// test means is "the draft exists now", so that is what is waited on.
    pub fn await_composer(&mut self, what: &str, done: impl Fn(&MailApp) -> bool) {
        for _ in 0..80 {
            if self.read(&done) {
                return;
            }
            self.tick();
            if self.read(&done) {
                return;
            }
            settle_worker();
        }
        panic!("{what}");
    }

    /// How many drafts are counting a hold down at once. Each keeps its own toast, bar and
    /// Undo, so two sends at once are two independent recalls.
    pub fn hold_count(&mut self) -> usize {
        self.read(|a| a.composer.holds.len())
    }

    /// The draft `u` would recall: the hold that has been counting down longest.
    pub fn held_draft(&mut self) -> Option<DraftId> {
        self.read(|a| a.hold_in_flight())
    }

    /// The plain text of every message a confirmed send put in the mailbox, in order.
    pub fn sent_bodies(&mut self) -> Vec<String> {
        self.read(|a| {
            a.mailbox
                .messages()
                .iter()
                .filter(|m| m.outgoing)
                .map(|m| m.body.clone())
                .collect()
        })
    }

    /// Bring an existing draft to the front of the tab strip, the way a tab click does.
    pub fn show_draft(&mut self, id: DraftId) {
        self.app.update(self.cx, |a, _| {
            a.tabs.open_draft_pinned(id);
        });
        self.tick();
    }

    /// Tick until `n` drafts are counting down, so a test asserts on the hold rather than on
    /// the worker's round trip.
    pub fn await_holds(&mut self, n: usize) {
        self.await_composer(&format!("{n} holds never started"), |a| {
            a.composer.holds.len() == n
        });
    }

    /// Press the composer's Send control for the draft it is showing. That control is the
    /// user-facing path into the hold, and it works whichever surface currently has the
    /// draft, focused or not.
    pub fn send_draft(&mut self) {
        self.click("compose-send");
        self.tick();
    }

    /// Hand the worker real files, exactly what the picker does once the user has chosen
    /// them. The draft is busy from this call until the worker answers.
    pub fn attach(&mut self, files: Vec<PathBuf>) {
        let id = self.read(|a| {
            a.tabs
                .active_draft()
                .or(a.inline_composer)
                .expect("a draft is open")
        });
        self.app
            .update(self.cx, |a, cx| a.attach_files_at(id, files, cx));
        self.tick();
    }

    /// Close a draft's tab, the way the tab strip's close control does. The draft itself
    /// is kept: only the surface goes.
    pub fn close_draft_tab(&mut self, id: DraftId) {
        self.app.update(self.cx, |a, _| {
            if let Some(ix) = a.tabs.index_of_draft(id) {
                a.tabs.close(ix);
            }
        });
        self.tick();
    }

    /// Run the countdown out and let the provider confirm, ticking until the delivery is in
    /// the mailbox. Nothing sleeps: the fake clock and the worker's own answer drive it.
    pub fn deliver_held(&mut self) {
        self.advance(HOLD_SECONDS + 1);
        for _ in 0..20 {
            self.tick();
            if !self.sent_bodies().is_empty() {
                return;
            }
            self.advance(1);
        }
        panic!("the confirmed send never reached the mailbox");
    }

    /// Every draft the composer holds open, inline ones included.
    pub fn open_drafts(&mut self) -> usize {
        self.read(|a| a.composer.drafts.len())
    }

    /// The state line of the draft the composer is showing.
    pub fn draft_status(&mut self) -> String {
        self.read(|a| {
            let id = a.tabs.active_draft().or(a.inline_composer)?;
            let open = a.composer.get(id)?;
            Some(open.status_line())
        })
        .unwrap_or_default()
    }

    /// Whether a send hold is counting down for any draft.
    pub fn hold_pending(&mut self) -> bool {
        self.read(|a| a.hold_in_flight().is_some())
    }

    /// The durable record of the draft the composer is showing, as
    /// `(kind, to, subject, body)`. The composer's own view state is never consulted: a test
    /// reads what the draft store actually holds.
    pub fn draft_fields(&mut self) -> (DraftKind, String, String, String) {
        self.read(|a| {
            let id = a
                .tabs
                .active_draft()
                .or(a.inline_composer)
                .expect("a draft is open");
            let r = a.composer.record(id).expect("the draft record");
            (
                r.kind,
                r.to.clone(),
                r.subject.clone(),
                r.body.source.clone(),
            )
        })
    }

    /// The attachments the draft's record holds, by file name.
    pub fn draft_attachments(&mut self) -> Vec<String> {
        self.read(|a| {
            let Some(id) = a.tabs.active_draft().or(a.inline_composer) else {
                return Vec::new();
            };
            a.composer
                .record(id)
                .map(|r| r.attachments.iter().map(|a| a.file_name.clone()).collect())
                .unwrap_or_default()
        })
    }

    /// Open a durable draft tab and focus its body for tests that edit the body.
    pub fn new_draft(&mut self) {
        // A draft this call opened, not whichever draft was already on screen: the tab of
        // the previous one is still there and its composer still exists.
        let before = self.read(|a| a.composer.drafts.len());
        self.keys("cmd-n");
        self.await_composer("cmd-n opens a draft tab", |a| {
            a.composer.drafts.len() > before
                && a.tabs
                    .active_draft()
                    .is_some_and(|id| a.composers.contains_key(&id))
        });
        self.cx
            .update_window(self.window, |_, window, cx| {
                self.app.update(cx, |a, cx| {
                    let id = a.tabs.active_draft().expect("draft tab");
                    a.composers
                        .get(&id)
                        .expect("composer")
                        .update(cx, |composer, cx| composer.focus_body(window, cx));
                });
            })
            .expect("window alive");
        self.settle();
    }

    pub fn settle(&mut self) {
        // The local autosave is debounced, so a pending write only becomes due once typing
        // has paused: let the fake clock pass that pause before waiting on the worker.
        self.advance(LOCAL_DEBOUNCE_SECS);
        self.await_composer("the composer never settled", |a| a.composer_idle());
    }

    /// Whether the Drafts pane is the list the app is showing.
    pub fn drafts_pane_open(&mut self) -> bool {
        self.read(|a| a.drafts_pane_open())
    }

    /// The Drafts pane's rows, top to bottom: `(account, subject, recipient, snippet)`.
    pub fn draft_rows(&mut self) -> Vec<(String, String, String, String)> {
        self.read(|a| {
            a.draft_rows()
                .into_iter()
                .map(|r| {
                    (
                        r.account.clone(),
                        r.subject.clone(),
                        r.recipients.clone(),
                        r.snippet.clone(),
                    )
                })
                .collect()
        })
    }

    /// The draft the pane cursor is on, if the Drafts pane is showing.
    pub fn cursor_draft(&mut self) -> Option<DraftId> {
        self.read(|a| a.cursor_draft().map(|r| r.id))
    }

    /// Move the draft the composer is showing into the tab strip, through the real control.
    pub fn expand_active_draft(&mut self) {
        self.click("compose-expand");
        self.tick();
    }

    /// Move the draft the composer is showing back under its thread.
    pub fn collapse_active_draft(&mut self) {
        self.click("compose-collapse");
        self.tick();
    }

    /// Discard the draft the composer is showing, answering the confirmation.
    pub fn discard_active_draft(&mut self) {
        self.click("compose-discard");
        self.tick();
        self.keys("1");
        self.settle();
    }
}

impl Harness<'_> {
    pub fn palette_rows(&mut self) -> Vec<String> {
        self.app.read_with(self.cx, |a, cx| a.palette_rows(cx))
    }
    /// Rows of the open filter picker, top to bottom.
    pub fn picker_rows(&mut self) -> Vec<String> {
        self.app
            .read_with(self.cx, |a, cx| a.filter_popover_rows(cx))
    }
    /// The pills the list header shows.
    pub fn pills(&mut self) -> Vec<String> {
        self.read(|a| a.pill_texts())
    }
    /// The list header's first line, e.g. `"Inbox · 12"` (or the Drafts pane's count).
    pub fn header(&mut self) -> String {
        self.read(|a| {
            let count = if a.drafts_pane_open() {
                a.draft_rows().len()
            } else {
                a.visible_ids().len()
            };
            a.header_title(count)
        })
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

    /// How many drafts have a local write queued but not yet started: the autosave is
    /// debounced, so this stays non-zero until typing pauses.
    pub fn pending_saves(&mut self) -> usize {
        self.read(|a| {
            a.composer
                .drafts
                .iter()
                .filter(|d| d.has_pending_write())
                .count()
        })
    }

    /// The location the list is showing, or `None` for a global search.
    pub fn location(&mut self) -> Option<Location> {
        self.read(|a| a.location())
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
                .update_window(window, |_, window, cx| {
                    window.click(format!("0-{page}"), cx)
                })
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
            .update_window(window, |_, window, cx| {
                window.within("popup-menu").click(option, cx)
            })
            .expect("settings window alive");
        self.cx.run_until_parked();
    }

    /// The settings page the sidebar is on.
    pub fn settings_page(&mut self) -> usize {
        let panel = self
            .read(|a| a.settings_panel())
            .expect("the settings window is open");
        panel.read_with(self.cx, |panel, _| panel.page())
    }
}
