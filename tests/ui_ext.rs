//! Headless keystroke tests for the v2 features, driving the real `MailApp` with a `FakeClock`.

use std::rc::Rc;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, Entity, Focusable, Point, TestAppContext, WindowBounds,
    WindowOptions, base::Root, px, size,
};
use mail_classifier::app::actions::{bind_keys, commands};
use mail_classifier::app::MailApp;
use mail_classifier::clock::{Clock, DAY, FakeClock, HOUR, Timestamp};
use mail_classifier::judge::{Mode, QuestionKey};
use mail_classifier::model::{Mailbox, MessageId, Tag, TriageState, TriageState::*};

/// 2026-09-29 (a Tuesday) 12:00:00 UTC.
const NOON: Timestamp = 1_790_683_200;
const MIDNIGHT: Timestamp = NOON - 12 * HOUR;

struct Harness<'a> {
    cx: &'a mut TestAppContext,
    window: AnyWindowHandle,
    app: Entity<MailApp>,
    clock: Rc<FakeClock>,
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
    let mut h = Harness { cx, window, app, clock };
    h.keys("");
    h
}

fn harness(cx: &mut TestAppContext) -> Harness<'_> {
    harness_with(cx, Mailbox::load_default())
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
        if !keys.is_empty() {
            self.cx.simulate_keystrokes(self.window, keys);
        }
        self.cx.run_until_parked();
    }
    /// Type literal text into the focused input.
    fn type_text(&mut self, text: &str) {
        let window = self.window;
        self.cx
            .update_window(window, |_, window, cx| window.input(text, cx))
            .unwrap();
        self.cx.run_until_parked();
    }
    fn read<R>(&mut self, f: impl FnOnce(&MailApp) -> R) -> R {
        self.cx.read_entity(&self.app, |a, _| f(a))
    }
    /// Advance the fake clock and run the app's tick.
    fn advance(&mut self, secs: Timestamp) {
        self.clock.advance(secs);
        self.tick();
    }
    fn tick(&mut self) {
        self.app.update(self.cx, |a, cx| a.tick(cx));
        self.cx.run_until_parked();
    }
    fn count(&mut self, s: TriageState) -> usize {
        self.read(|a| a.mailbox.count(s))
    }
    fn cursor(&mut self) -> Option<MessageId> {
        self.read(|a| a.triage.cursor(&a.mailbox))
    }
    fn state_of(&mut self, id: MessageId) -> TriageState {
        self.read(|a| a.mailbox.state_of(id).unwrap())
    }
    fn visible(&mut self) -> Vec<MessageId> {
        self.read(|a| a.visible_ids())
    }
    fn total(&mut self) -> usize {
        self.read(|a| a.mailbox.messages().len())
    }
    fn has_tag(&mut self, id: MessageId, tag: Tag) -> bool {
        self.read(|a| a.mailbox.tags(id).contains(&tag))
    }
    fn toast(&mut self) -> String {
        self.read(|a| a.toast.as_ref().map(|t| t.to_string()).unwrap_or_default())
    }
    /// sum(counts) + screener + hidden == total
    fn assert_invariant(&mut self, ctx: &str) {
        let (sum, screener, hidden, total) = self.read(|a| {
            (
                TriageState::ALL.iter().map(|s| a.mailbox.count(*s)).sum::<usize>(),
                a.mailbox.screener_ids().len(),
                a.mailbox.hidden_count(),
                a.mailbox.messages().len(),
            )
        });
        assert_eq!(sum + screener + hidden, total, "invariant broken after {ctx}");
    }
    /// Move the cursor with `j` until it reaches `id` (current view).
    fn goto(&mut self, id: MessageId) {
        for _ in 0..200 {
            if self.cursor() == Some(id) {
                return;
            }
            self.keys("j");
        }
        panic!("could not reach message {id}");
    }
    fn send_reply(&mut self, body: &str) {
        self.keys("r");
        assert!(self.read(|a| a.compose_open()));
        self.type_text(body);
        self.keys("cmd-enter");
        assert!(!self.read(|a| a.compose_open()));
    }
}

// ---------------------------------------------------------------- Waiting resurfacing

#[gpui_kit::gpui::test]
fn waiting_resurfaces_after_three_days_with_no_reply_tag(cx: &mut TestAppContext) {
    let mut h = harness_with(
        cx,
        mailbox(&[
            msg(1, 1, "carol@a.io", "alpha", 20, "Inbox"),
            msg(2, 2, "dana@a.io", "beta", 19, "Inbox"),
            // A reply from someone else in thread 2 that arrives after msg 2 started waiting.
            msg(3, 2, "erin@a.io", "Re: beta", 30, "Done"),
        ]),
    );
    h.goto(1);
    h.keys("w");
    h.goto(2);
    h.keys("w");
    assert_eq!(h.state_of(1), Waiting);
    assert_eq!(h.state_of(2), Waiting);

    h.advance(3 * DAY - 1);
    assert_eq!(h.state_of(1), Waiting, "must not resurface before 3 days");

    h.advance(1);
    assert_eq!(h.state_of(1), Inbox, "resurfaces at exactly 3 days");
    assert!(h.has_tag(1, Tag::NoReply));
    assert_eq!(
        h.state_of(2),
        Waiting,
        "a newer message from someone else keeps the thread waiting"
    );
    assert!(!h.has_tag(2, Tag::NoReply));

    // Idempotent.
    h.advance(DAY);
    assert_eq!(h.state_of(1), Inbox);
    assert_eq!(h.state_of(2), Waiting);
    h.assert_invariant("waiting resurfacing");
}

// ---------------------------------------------------------------- Snooze

fn snooze_box() -> Mailbox {
    mailbox(&[
        msg(1, 1, "a@a.io", "one", 25, "Inbox"),
        msg(2, 2, "b@a.io", "two", 24, "Inbox"),
        msg(3, 3, "c@a.io", "three", 23, "Inbox"),
    ])
}

#[gpui_kit::gpui::test]
fn snooze_presets_hide_until_time_then_return(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, snooze_box());
    let cases: [(&str, Timestamp); 3] = [
        ("1", MIDNIGHT + 18 * HOUR),
        ("2", MIDNIGHT + DAY + 8 * HOUR),
        ("3", MIDNIGHT + 6 * DAY + 8 * HOUR), // next Monday (Tue + 6)
    ];
    for (i, (key, until)) in cases.iter().enumerate() {
        let id = (i + 1) as u32;
        h.clock.set(NOON);
        h.goto(id);
        h.keys("l");
        assert!(h.read(|a| a.snooze_open()), "l opens the picker");
        h.keys(key);
        assert!(!h.read(|a| a.snooze_open()));
        assert_eq!(h.state_of(id), Later, "preset {key}");
        assert_eq!(h.read(|a| a.mailbox.snoozed_until(id)), Some(*until), "preset {key}");
        assert!(!h.visible().contains(&id));

        h.clock.set(*until - 1);
        h.tick();
        assert_eq!(h.state_of(id), Later, "still asleep 1s early (preset {key})");
        h.clock.set(*until);
        h.tick();
        assert_eq!(h.state_of(id), Inbox, "wakes at the return time (preset {key})");
    }
    h.assert_invariant("snooze presets");
}

#[gpui_kit::gpui::test]
fn snooze_custom_duration_and_escape(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, snooze_box());
    let id = h.cursor().unwrap();

    h.keys("l escape");
    assert!(!h.read(|a| a.snooze_open()));
    assert_eq!(h.state_of(id), Inbox, "escape cancels the picker");

    h.keys("l 4");
    h.type_text("3h");
    h.keys("enter");
    assert_eq!(h.state_of(id), Later);
    assert_eq!(h.read(|a| a.mailbox.snoozed_until(id)), Some(NOON + 3 * HOUR));

    h.advance(3 * HOUR - 1);
    assert_eq!(h.state_of(id), Later);
    h.advance(1);
    assert_eq!(h.state_of(id), Inbox);

    // Undo of a snooze restores the message immediately.
    h.keys("l 4");
    h.type_text("30m");
    h.keys("enter");
    assert_eq!(h.state_of(id), Later);
    h.keys("u");
    assert_eq!(h.state_of(id), Inbox);
    assert_eq!(h.read(|a| a.mailbox.snoozed_until(id)), None);
}

#[gpui_kit::gpui::test]
fn later_view_lists_snoozed_message_with_return_time(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, snooze_box());
    let id = h.cursor().unwrap();
    h.keys("l 2 3");
    assert_eq!(h.view_state(), Later);
    assert_eq!(h.visible(), vec![id]);
    assert!(h.read(|a| a.mailbox.snoozed_until(id)).is_some());
}

impl Harness<'_> {
    fn view_state(&mut self) -> TriageState {
        self.read(|a| a.triage.view)
    }
}

// ---------------------------------------------------------------- Rule suggestions

fn rules_box() -> Mailbox {
    mailbox(&[
        msg(1, 1, "sam@news.io", "n1", 25, "Inbox"),
        msg(2, 2, "sam@news.io", "n2", 24, "Inbox"),
        msg(3, 3, "sam@news.io", "n3", 23, "Inbox"),
        msg(4, 4, "tom@news.io", "t1", 22, "Inbox"),
        msg(5, 5, "tom@news.io", "t2", 21, "Inbox"),
        msg(6, 6, "zed@news.io", "z1", 20, "Inbox"),
    ])
}

/// Sender-wide Done from the Inbox cursor, then bring one message back so it can be repeated.
fn repeat_sender_done(h: &mut Harness<'_>) {
    h.keys("shift-e 4 i 1");
}

#[gpui_kit::gpui::test]
fn second_sender_wide_action_suggests_rule_and_accept_applies(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, rules_box());
    h.keys("shift-e");
    assert_eq!(h.read(|a| a.pending_rule()), None, "no suggestion after the first action");
    assert_eq!(h.count(Done), 3);

    repeat_sender_done(&mut h);
    assert_eq!(h.read(|a| a.pending_rule()), None);
    h.keys("shift-e");
    let rule = h.read(|a| a.pending_rule()).expect("suggested on the 2nd identical action");
    assert_eq!(rule.sender, "sam@news.io");
    assert_eq!(rule.state, Done);

    // Bring another message from the sender back, then accept: rule applies to it.
    h.keys("4");
    let id = h.cursor().unwrap();
    h.keys("i 1");
    assert_eq!(h.state_of(id), Inbox);
    h.keys("shift-y");
    assert_eq!(h.read(|a| a.pending_rule()), None);
    assert_eq!(h.read(|a| a.rules.rule_for("sam@news.io")), Some(Done));
    assert_eq!(h.state_of(id), Done, "accepted rule is applied to the sender's inbox mail");
    h.assert_invariant("accept rule");
}

#[gpui_kit::gpui::test]
fn dismissed_rule_is_never_suggested_again(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, rules_box());
    h.keys("shift-e");
    repeat_sender_done(&mut h);
    h.keys("shift-e");
    assert!(h.read(|a| a.pending_rule()).is_some());
    h.keys("shift-n");
    assert_eq!(h.read(|a| a.pending_rule()), None);
    assert_eq!(h.read(|a| a.rules.rule_for("sam@news.io")), None);

    for _ in 0..3 {
        repeat_sender_done(&mut h);
        h.keys("shift-e");
        assert_eq!(h.read(|a| a.pending_rule()), None, "dismissed rules stay dismissed");
    }
    assert!(h.read(|a| a.rules.rules().is_empty()));
}

#[gpui_kit::gpui::test]
fn rules_panel_revokes_accepted_rule(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, rules_box());
    h.keys("shift-e");
    repeat_sender_done(&mut h);
    h.keys("shift-e shift-y");
    assert_eq!(h.read(|a| a.rules.rules().len()), 1);

    h.keys("shift-r");
    assert!(h.read(|a| a.rules_open()));
    h.keys("backspace");
    assert!(h.read(|a| a.rules.rules().is_empty()), "backspace revokes the selected rule");
    assert_eq!(h.read(|a| a.rules.rule_for("sam@news.io")), None);
    h.keys("escape");
    assert!(!h.read(|a| a.rules_open()));
}

// ---------------------------------------------------------------- Screener

fn screener_box() -> Mailbox {
    let msgs = json(&[
        msg(1, 1, "known@a.io", "k1", 25, "Inbox"),
        msg(2, 2, "new1@a.io", "n1a", 24, "Inbox"),
        msg(3, 3, "new1@a.io", "n1b", 23, "Inbox"),
        msg(4, 4, "new2@a.io", "n2", 22, "Inbox"),
    ]);
    Mailbox::from_json_with_contacts(&msgs, r#"["known@a.io"]"#).unwrap()
}

#[gpui_kit::gpui::test]
fn screener_allow_and_block(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, screener_box());
    assert_eq!(h.count(Inbox), 1, "unscreened mail is not in the inbox");
    h.assert_invariant("start");

    h.keys("5");
    assert!(h.read(|a| a.screener_open()));
    assert_eq!(h.visible(), vec![2, 3, 4]);

    // Allow: the whole sender moves to the inbox.
    h.keys("a");
    assert_eq!(h.visible(), vec![4]);
    assert_eq!(h.count(Inbox), 3);
    h.assert_invariant("allow");

    // Block: hidden, not deleted.
    h.keys("b");
    assert!(h.visible().is_empty());
    assert_eq!(h.total(), 4);
    assert_eq!(h.read(|a| a.mailbox.hidden_count()), 1);
    assert_eq!(h.count(Inbox), 3);
    h.assert_invariant("block");

    // Undo restores the blocked sender to the screener.
    h.keys("u");
    assert_eq!(h.visible(), vec![4]);
    h.assert_invariant("undo block");
}

// ---------------------------------------------------------------- Triage session

#[gpui_kit::gpui::test]
fn triage_session_auto_advances_and_ends_with_summary(cx: &mut TestAppContext) {
    let mut h = harness_with(
        cx,
        mailbox(&[
            msg(1, 1, "a@a.io", "one", 25, "Inbox"),
            msg(2, 2, "b@a.io", "two", 24, "Inbox"),
            msg(3, 3, "c@a.io", "three", 23, "Inbox"),
        ]),
    );
    h.keys("t");
    assert_eq!(h.read(|a| a.session_progress()), Some((1, 3)));
    assert_eq!(h.read(|a| a.opened()), Some(1), "session shows the current message");

    h.keys("e");
    assert_eq!(h.state_of(1), Done);
    assert_eq!(h.read(|a| a.session_progress()), Some((2, 3)));
    h.clock.advance(65);
    h.keys("w");
    assert_eq!(h.state_of(2), Waiting);
    assert_eq!(h.read(|a| a.session_progress()), Some((3, 3)));
    h.keys("i");
    assert_eq!(h.state_of(3), Inbox);

    assert_eq!(h.read(|a| a.session_progress()), None, "session is over");
    assert_eq!(h.read(|a| a.session_end()), Some((3, 65)), "3 handled · 1m 5s");

    h.keys("escape");
    assert_eq!(h.read(|a| a.session_end()), None);
    h.assert_invariant("session");
}

#[gpui_kit::gpui::test]
fn triage_session_escape_ends_early(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, snooze_box());
    h.keys("t e");
    assert_eq!(h.read(|a| a.session_progress()), Some((2, 3)));
    h.keys("escape");
    assert_eq!(h.read(|a| a.session_progress()), None);
    assert_eq!(h.state_of(1), Done);
    assert_eq!(h.state_of(2), Inbox);
    assert_eq!(h.read(|a| a.session_end()), Some((1, 0)), "escape after handling shows the end card");
    h.keys("escape");
    assert_eq!(h.read(|a| a.session_end()), None, "a second escape dismisses the card");
    h.keys("t escape");
    assert_eq!(h.read(|a| a.session_end()), None, "nothing handled: no card");
}

// ---------------------------------------------------------------- Search

fn search_box() -> Mailbox {
    mailbox(&[
        msg(1, 1, "dana@x.io", "Invoice for lunch", 25, "Inbox"),
        msg(2, 2, "dana@x.io", "Roadmap", 10, "Inbox"),
        msg(3, 3, "omar@x.io", "Invoice overdue", 12, "Waiting"),
        msg(4, 4, "omar@x.io", "Team offsite", 5, "Done"),
    ])
}

fn search(h: &mut Harness<'_>, q: &str) -> Vec<MessageId> {
    h.keys("/");
    assert!(h.read(|a| a.palette_open()));
    h.type_text(q.trim_start_matches('/'));
    h.keys("enter");
    let ids = h.visible();
    assert!(h.read(|a| a.search_header()).is_some_and(|s| s.starts_with("search:")));
    h.keys("escape");
    assert_eq!(h.read(|a| a.search_header()), None, "escape clears the search");
    ids
}

#[gpui_kit::gpui::test]
fn search_operators(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, search_box());
    let sorted = |h: &mut Harness<'_>, q: &str| {
        let mut v = search(h, q);
        v.sort();
        v
    };
    assert_eq!(sorted(&mut h, "from:dana"), vec![1, 2]);
    assert_eq!(sorted(&mut h, "from:OMAR@x.io"), vec![3, 4], "case-insensitive");
    assert_eq!(sorted(&mut h, "subject:invoice"), vec![1, 3]);
    assert_eq!(sorted(&mut h, "is:waiting"), vec![3]);
    assert_eq!(sorted(&mut h, "is:done"), vec![4]);
    assert_eq!(sorted(&mut h, "before:2026-09-11"), vec![2, 4]);
    assert_eq!(sorted(&mut h, "after:2026-09-11"), vec![1, 3]);
    assert_eq!(sorted(&mut h, "lunch"), vec![1]);
    assert_eq!(sorted(&mut h, "from:omar subject:invoice"), vec![3], "terms are ANDed");
    assert_eq!(sorted(&mut h, "zzzznomatch"), Vec::<u32>::new());
    // The mailbox is untouched by searching.
    assert_eq!(h.total(), 4);
    h.assert_invariant("search");
}

// ---------------------------------------------------------------- Mute / unsubscribe

#[gpui_kit::gpui::test]
fn mute_hides_thread_and_unsubscribe_hides_sender(cx: &mut TestAppContext) {
    let mut h = harness_with(
        cx,
        mailbox(&[
            msg(1, 1, "a@a.io", "t1 first", 25, "Inbox"),
            msg(2, 1, "b@a.io", "t1 reply", 24, "Inbox"),
            msg(3, 2, "c@a.io", "t2", 23, "Inbox"),
            msg(4, 3, "c@a.io", "t3", 22, "Inbox"),
            msg(5, 4, "d@a.io", "t4", 21, "Inbox"),
        ]),
    );
    h.keys("m");
    assert_eq!(h.read(|a| a.mailbox.hidden_count()), 2, "whole thread is muted");
    assert!(h.read(|a| a.mailbox.is_muted(1)));
    assert_eq!(h.visible(), vec![3, 4, 5]);
    h.assert_invariant("mute");
    h.keys("u");
    assert_eq!(h.read(|a| a.mailbox.hidden_count()), 0);
    assert_eq!(h.visible(), vec![1, 2, 3, 4, 5]);

    h.goto(3);
    h.keys("shift-u");
    assert_eq!(h.read(|a| a.mailbox.unsubscribed().to_vec()), vec!["c@a.io".to_string()]);
    assert_eq!(h.read(|a| a.mailbox.hidden_count()), 2, "all of the sender's mail is hidden");
    assert_eq!(h.visible(), vec![1, 2, 5]);
    assert_eq!(h.total(), 5, "nothing is deleted");
    h.assert_invariant("unsubscribe");
    h.keys("u");
    assert!(h.read(|a| a.mailbox.unsubscribed().is_empty()));
    assert_eq!(h.visible().len(), 5);
}

// ---------------------------------------------------------------- Undo send / outbox

#[gpui_kit::gpui::test]
fn send_goes_to_outbox_and_undo_recalls_within_window(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, snooze_box());
    let id = h.cursor().unwrap();
    h.send_reply("hello there");
    assert_eq!(h.read(|a| a.mailbox.outbox().len()), 1);
    assert_eq!(h.read(|a| a.mailbox.sent().len()), 0);
    assert_eq!(h.state_of(id), Waiting, "original moves to Waiting immediately");

    h.advance(9);
    assert_eq!(h.read(|a| a.mailbox.outbox().len()), 1, "still recallable at 9s");
    h.keys("u");
    assert!(h.read(|a| a.compose_open()), "recall reopens compose");
    assert_eq!(h.read(|a| a.mailbox.outbox().len()), 0);
    assert_eq!(h.read(|a| a.mailbox.sent().len()), 0);
    assert_eq!(h.state_of(id), Inbox);
    h.keys("escape");
    h.assert_invariant("recall");
}

#[gpui_kit::gpui::test]
fn outbox_flushes_to_sent_after_ten_seconds(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, snooze_box());
    let id = h.cursor().unwrap();
    h.send_reply("bye");
    h.advance(9);
    assert_eq!(h.read(|a| a.mailbox.sent().len()), 0);
    h.advance(1);
    assert_eq!(h.read(|a| a.mailbox.outbox().len()), 0);
    let sent = h.read(|a| a.mailbox.sent().to_vec());
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].in_reply_to, id);
    assert!(sent[0].body.contains("bye"));

    // Too late to recall: undo only reverts state.
    h.keys("u");
    assert!(!h.read(|a| a.compose_open()));
    assert_eq!(h.state_of(id), Inbox);
    assert_eq!(h.read(|a| a.mailbox.sent().len()), 1);
}

// ---------------------------------------------------------------- Classifier

fn tag_total(h: &mut Harness<'_>) -> usize {
    h.read(|a| a.mailbox.messages().iter().map(|m| a.mailbox.tags(m.id).len()).sum())
}

fn pending_total(h: &mut Harness<'_>) -> usize {
    h.read(|a| a.mailbox.messages().iter().map(|m| a.mailbox.pending(m.id).len()).sum())
}

/// Move the cursor down the current view until a message with pending suggestions is found.
fn goto_pending(h: &mut Harness<'_>) -> MessageId {
    h.keys("1");
    for _ in 0..80 {
        let id = h.cursor().unwrap();
        if h.read(|a| !a.mailbox.pending(id).is_empty()) {
            return id;
        }
        h.keys("j");
    }
    panic!("no message with pending suggestions in the inbox");
}

#[gpui_kit::gpui::test]
fn classifier_runs_on_startup_and_c_reruns(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    assert!(
        tag_total(&mut h) + pending_total(&mut h) > 0,
        "startup classification produces badges"
    );
    h.keys("c");
    assert!(h.toast().contains("auto-applied"), "toast was {:?}", h.toast());
    assert!(h.toast().contains("to review"));
    h.assert_invariant("classify");
}

#[gpui_kit::gpui::test]
fn accept_applies_and_reject_discards_pending_suggestions(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let id = goto_pending(&mut h);
    let before_state = h.state_of(id);
    let tags_before = h.read(|a| a.mailbox.tags(id).len());
    let suggestions: Vec<_> = h.read(|a| {
        a.mailbox.pending(id).iter().map(|s| (s.key, format!("{:?}", s.answer.value))).collect()
    });
    assert!(!suggestions.is_empty());

    h.keys("y");
    assert!(h.read(|a| a.mailbox.pending(id).is_empty()), "accept clears the pending badges");
    let after_state = h.state_of(id);
    let tags_after = h.read(|a| a.mailbox.tags(id).len());
    assert!(
        after_state != before_state || tags_after > tags_before,
        "accepting must apply something ({suggestions:?})"
    );
    h.assert_invariant("accept");

    // One undo step brings the pending badges back.
    h.keys("u");
    assert!(!h.read(|a| a.mailbox.pending(id).is_empty()));
    assert_eq!(h.state_of(id), before_state);
    assert_eq!(h.read(|a| a.mailbox.tags(id).len()), tags_before);

    // Reject discards without applying.
    h.keys("n");
    assert!(h.read(|a| a.mailbox.pending(id).is_empty()));
    assert_eq!(h.state_of(id), before_state);
    assert_eq!(h.read(|a| a.mailbox.tags(id).len()), tags_before);
}

#[gpui_kit::gpui::test]
fn auto_applied_labels_can_be_undone(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let tags = tag_total(&mut h);
    assert!(tags > 0, "startup auto-applies at least one label");
    // Undo repeatedly until the auto-applied labels are gone; each undo removes a step, none adds.
    let mut last = tags;
    let mut dropped = false;
    for _ in 0..200 {
        h.keys("u");
        let now = tag_total(&mut h);
        assert!(now <= last, "undo never adds labels here");
        if now < last {
            dropped = true;
            break;
        }
        last = now;
    }
    assert!(dropped, "undo eventually removes an auto-applied label");
    h.assert_invariant("undo auto");
}

// ---------------------------------------------------------------- Settings / summaries

#[gpui_kit::gpui::test]
fn settings_switch_question_to_review(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let auto = |h: &mut Harness<'_>| {
        h.read(|a| matches!(a.policy.mode(QuestionKey::Spam), Mode::Auto { .. }))
    };
    assert!(auto(&mut h), "Spam defaults to Auto");
    h.keys("cmd-,");
    assert!(h.read(|a| a.settings_open()));
    h.keys("space");
    assert!(!auto(&mut h), "space toggles the selected question to Review");
    h.keys("space");
    assert!(auto(&mut h), "and back to Auto");
    h.keys("escape");
    assert!(!h.read(|a| a.settings_open()));
}

/// A mailbox whose only inbox mail is an obvious spam message (id 1), classified on startup.
fn spam_box() -> Mailbox {
    mailbox(&[
        msg(1, 1, "recruiter@talentloop.com", "quick question", 25, "Inbox"),
        msg(2, 2, "friend@a.io", "lunch", 24, "Inbox"),
    ])
}

/// Undo startup auto-labels until message 1 is a plain Inbox message again.
fn reset_spam(h: &mut Harness<'_>) {
    for _ in 0..20 {
        if h.state_of(1) == Inbox && !h.has_tag(1, Tag::Spam) {
            return;
        }
        h.keys("u");
    }
    panic!("could not restore message 1");
}

#[gpui_kit::gpui::test]
fn spam_in_auto_mode_is_applied(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, spam_box());
    assert!(h.has_tag(1, Tag::Spam), "startup auto-applies the spam label");
    assert_eq!(h.state_of(1), Done);
    reset_spam(&mut h);
    assert!(h.read(|a| a.mailbox.pending(1).iter().all(|s| s.key != QuestionKey::Spam)));
    h.keys("c");
    assert!(h.has_tag(1, Tag::Spam), "Auto: c applies Tag::Spam");
    assert_eq!(h.state_of(1), Done);
}

#[gpui_kit::gpui::test]
fn spam_in_review_mode_is_queued_not_applied(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, spam_box());
    reset_spam(&mut h);
    h.keys("cmd-, space escape");
    assert!(h.read(|a| matches!(a.policy.mode(QuestionKey::Spam), Mode::Review)));
    h.keys("c");
    assert!(!h.has_tag(1, Tag::Spam), "Review: no auto-applied label");
    assert_eq!(h.state_of(1), Inbox);
    assert!(
        h.read(|a| a.mailbox.pending(1).iter().any(|s| s.key == QuestionKey::Spam)),
        "Review: a pending Spam suggestion (badge) is queued"
    );
    // Accepting it applies the label.
    h.keys("y");
    assert!(h.has_tag(1, Tag::Spam));
    assert_eq!(h.state_of(1), Done);
}

#[gpui_kit::gpui::test]
fn search_excludes_muted_threads(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, search_box());
    assert_eq!(search(&mut h, "roadmap"), vec![2]);
    h.goto(2);
    h.keys("m");
    assert_eq!(h.read(|a| a.mailbox.hidden_count()), 1);
    assert!(search(&mut h, "roadmap").is_empty(), "muted mail is not searchable");
    assert_eq!(search(&mut h, "from:dana"), vec![1]);
}

#[gpui_kit::gpui::test]
fn summary_is_opt_in(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    assert!(!h.read(|a| a.summaries_enabled));
    h.keys("s");
    assert!(h.toast().contains("Enable summaries"), "toast was {:?}", h.toast());
    assert!(h.read(|a| a.summary_shown()).is_none());

    h.keys("cmd-,");
    // Five questions, then the summaries row.
    h.keys("j j j j j space");
    assert!(h.read(|a| a.summaries_enabled));
    h.keys("escape");
    h.keys("s");
    let summary = h.read(|a| a.summary_shown()).expect("summary shown after enabling");
    assert!(!summary.summary.is_empty());
}

// ---------------------------------------------------------------- Palette / help

/// Keys introduced by the v2 features.
const NEW_KEYS: [&str; 14] = [
    "l", "y", "n", "shift-y", "shift-n", "shift-r", "5", "m", "shift-u", "s", "cmd-,", "t", "/",
    "c",
];

#[gpui_kit::gpui::test]
fn every_new_command_is_in_palette_search_and_help(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let specs = commands();
    let mut names = Vec::new();
    for key in NEW_KEYS {
        let spec = specs
            .iter()
            .find(|c| c.key == key)
            .unwrap_or_else(|| panic!("no command with key {key:?}"));
        names.push(spec.name);
    }

    h.keys("?");
    assert!(h.read(|a| a.help_open()));
    let help = h.read(|a| a.help_lines());
    for name in &names {
        assert!(help.iter().any(|l| l.contains(name)), "help lacks {name:?}");
    }
    h.keys("?");

    for name in names {
        h.keys("cmd-k");
        assert!(h.read(|a| a.palette_open()));
        h.type_text(name);
        let rows = h.palette_rows();
        assert!(rows.iter().any(|r| r.contains(name)), "palette search for {name:?}: {rows:?}");
        h.keys("escape");
        assert!(!h.read(|a| a.palette_open()));
    }
}

// ---------------------------------------------------------------- Invariant

#[gpui_kit::gpui::test]
fn counts_plus_screener_plus_hidden_equal_total_over_mixed_sequence(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.assert_invariant("start");
    let steps = [
        "e", "w", "j j x j x e", "shift-e", "u", "l 1", "shift-w", "m", "5", "a", "b", "1",
        "j shift-u", "u u", "r", "t", "e w i e", "escape", "c", "y", "n", "2", "i", "3", "4",
        "shift-i", "shift-l", "1", "u u u u", "cmd-z",
    ];
    for step in steps {
        if step == "r" {
            h.send_reply("ok");
        } else {
            h.keys(step);
        }
        // Panels/pickers left open by a step must not leak into the next.
        h.keys("escape");
        h.assert_invariant(step);
    }
    h.advance(DAY);
    h.assert_invariant("after a day");
    h.advance(3 * DAY);
    h.assert_invariant("after 4 days");
}

impl Harness<'_> {
    fn palette_rows(&mut self) -> Vec<String> {
        self.app.read_with(self.cx, |a, cx| a.palette_rows(cx))
    }
}
