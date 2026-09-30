//! Headless keyboard-only tests driving the real `MailApp` view.

use gpui_kit::{
    AppContext, Bounds, Entity, Focusable, Point, TestAppContext, WindowBounds, WindowOptions,
    base::Root, px, size,
};
use mail_classifier::app::{MailApp, actions::bind_keys};
use mail_classifier::model::{MessageId, TriageState, TriageState::*};

struct Harness<'a> {
    cx: &'a mut TestAppContext,
    window: gpui_kit::AnyWindowHandle,
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
                    size: size(px(1200.), px(800.)),
                })),
                ..Default::default()
            },
            cx,
            |window, cx| {
                let view = cx.new(|cx| {
                    MailApp::new(mail_classifier::model::Mailbox::load_default(), window, cx)
                });
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
    fn read<R>(&mut self, f: impl FnOnce(&MailApp) -> R) -> R {
        self.cx.read_entity(&self.app, |a, _| f(a))
    }
    fn count(&mut self, s: TriageState) -> usize {
        self.read(|a| a.mailbox.count(s))
    }
    fn counts(&mut self) -> [usize; 4] {
        TriageState::ALL.map(|s| self.count(s))
    }
    fn cursor(&mut self) -> Option<MessageId> {
        self.read(|a| a.triage.cursor(&a.mailbox))
    }
    fn index(&mut self) -> usize {
        self.read(|a| a.triage.cursor_index())
    }
    fn view(&mut self) -> TriageState {
        self.read(|a| a.triage.view)
    }
    fn state_of(&mut self, id: MessageId) -> TriageState {
        self.read(|a| a.mailbox.state_of(id).unwrap())
    }
    fn total(&mut self) -> usize {
        self.read(|a| a.mailbox.messages().len())
    }
    /// Visible counts + screener + hidden: every message is in exactly one bucket.
    fn accounted(&mut self) -> usize {
        self.read(|a| {
            TriageState::ALL.iter().map(|s| a.mailbox.count(*s)).sum::<usize>()
                + a.mailbox.screener_ids().len()
                + a.mailbox.hidden_count()
        })
    }
}

#[gpui_kit::gpui::test]
fn jk_moves_cursor(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    assert_eq!(h.index(), 0);
    h.keys("j j");
    assert_eq!(h.index(), 2);
    h.keys("k");
    assert_eq!(h.index(), 1);
    h.keys("down");
    assert_eq!(h.index(), 2);
    h.keys("up up up");
    assert_eq!(h.index(), 0);
}

#[gpui_kit::gpui::test]
fn enter_opens_message(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    assert_eq!(h.read(|a| a.opened()), None);
    h.keys("j");
    let id = h.cursor().unwrap();
    h.keys("enter");
    assert_eq!(h.read(|a| a.opened()), Some(id));
}

#[gpui_kit::gpui::test]
fn state_keys_move_messages_and_undo(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let start = h.counts();
    let total = h.total();

    let a = h.cursor().unwrap();
    h.keys("e");
    assert_eq!(h.state_of(a), Done);
    assert_eq!(h.count(Inbox), start[0] - 1);
    assert_eq!(h.count(Done), start[3] + 1);

    let b = h.cursor().unwrap();
    assert_ne!(a, b);
    h.keys("w");
    assert_eq!(h.state_of(b), Waiting);
    let c = h.cursor().unwrap();
    h.keys("l 1"); // `l` opens the snooze picker; 1 = Tonight
    assert_eq!(h.state_of(c), Later);
    assert_eq!(h.count(Inbox), start[0] - 3);
    assert_eq!(h.count(Waiting), start[1] + 1);
    assert_eq!(h.count(Later), start[2] + 1);

    // Move back to inbox from the Done view.
    h.keys("4");
    assert_eq!(h.cursor(), h.read(|a| a.mailbox.ids_in(Done).first().copied()));
    let d = h.cursor().unwrap();
    h.keys("i");
    assert_eq!(h.state_of(d), Inbox);

    // Undo everything, one step at a time.
    h.keys("u");
    assert_eq!(h.state_of(d), Done);
    h.keys("u u u");
    assert_eq!(h.counts(), start);
    assert_eq!(h.accounted(), total);
    // Extra undo is harmless.
    h.keys("u");
    assert_eq!(h.counts(), start);
}

#[gpui_kit::gpui::test]
fn cmd_z_undoes(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let start = h.counts();
    h.keys("e");
    assert_ne!(h.counts(), start);
    h.keys("cmd-z");
    assert_eq!(h.counts(), start);
}

#[gpui_kit::gpui::test]
fn number_keys_switch_views(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    assert_eq!(h.view(), Inbox);
    h.keys("2");
    assert_eq!(h.view(), Waiting);
    h.keys("3");
    assert_eq!(h.view(), Later);
    h.keys("4");
    assert_eq!(h.view(), Done);
    h.keys("1");
    assert_eq!(h.view(), Inbox);
    h.keys("j j 3");
    assert_eq!(h.index(), 0, "switching view resets the cursor");
}

#[gpui_kit::gpui::test]
fn palette_filters_and_runs_command(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    assert!(!h.read(|a| a.palette_open()));
    let start = h.counts();
    let target = h.cursor().unwrap();

    h.keys("cmd-k");
    assert!(h.read(|a| a.palette_open()));
    // Letters here are also single-key bindings (e, r, o...): they must go to the input.
    h.keys("m a r k space d o n e");
    assert_eq!(h.counts(), start, "typing must not trigger bindings");
    assert!(h.read(|a| a.palette_open()));
    h.keys("enter");
    assert!(!h.read(|a| a.palette_open()), "palette closes after running");
    assert_eq!(h.state_of(target), Done);
    assert_eq!(h.count(Done), start[3] + 1);

    // Escape dismisses without effect.
    h.keys("cmd-k");
    assert!(h.read(|a| a.palette_open()));
    h.keys("escape");
    assert!(!h.read(|a| a.palette_open()));
    // cmd-k toggles.
    h.keys("cmd-k cmd-k");
    assert!(!h.read(|a| a.palette_open()));
}

#[gpui_kit::gpui::test]
fn palette_runs_view_switch(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.keys("cmd-k");
    h.keys("s h o w space l a t e r");
    h.keys("enter");
    assert_eq!(h.view(), Later);
}

#[gpui_kit::gpui::test]
fn reply_send_moves_to_waiting(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let id = h.cursor().unwrap();
    let start = h.counts();
    assert_eq!(h.read(|a| a.mailbox.sent().len()), 0);

    h.keys("r");
    assert!(h.read(|a| a.compose_open()));
    // 'e', 'j', 'x', 'u' etc. must be typed into the body, not trigger actions.
    h.keys("t h a n k s space j e x");
    assert_eq!(h.counts(), start);
    h.keys("cmd-enter");
    assert!(!h.read(|a| a.compose_open()));
    let (n, sent, reply) = h.read(|a| {
        (
            a.mailbox.outbox().len(),
            a.mailbox.sent().len(),
            a.mailbox.outbox().last().map(|o| (o.reply.in_reply_to, o.reply.body.clone())),
        )
    });
    assert_eq!((n, sent), (1, 0), "sends wait in the outbox first");
    let (to, body) = reply.unwrap();
    assert_eq!(to, id);
    assert!(body.contains("thanks je"), "body was {body:?}");
    assert_eq!(h.state_of(id), Waiting);
    assert_eq!(h.count(Waiting), start[1] + 1);
    assert_eq!(h.count(Inbox), start[0] - 1);

    // Undo recalls the pending reply and reopens compose.
    h.keys("u");
    assert!(h.read(|a| a.compose_open()));
    assert_eq!(h.read(|a| a.mailbox.outbox().len()), 0);
    assert_eq!(h.state_of(id), Inbox);
    h.keys("escape");
}

#[gpui_kit::gpui::test]
fn reply_escape_cancels(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let id = h.cursor().unwrap();
    let start = h.counts();
    h.keys("r");
    assert!(h.read(|a| a.compose_open()));
    h.keys("h i");
    h.keys("escape");
    assert!(!h.read(|a| a.compose_open()));
    assert_eq!(h.read(|a| a.mailbox.sent().len()), 0);
    assert_eq!(h.state_of(id), Inbox);
    assert_eq!(h.counts(), start);
    // Keyboard control returns to the list.
    h.keys("j");
    assert_eq!(h.index(), 1);
}

#[gpui_kit::gpui::test]
fn question_mark_toggles_help(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    assert!(!h.read(|a| a.help_open()));
    h.keys("?");
    assert!(h.read(|a| a.help_open()));
    h.keys("?");
    assert!(!h.read(|a| a.help_open()));
}

#[gpui_kit::gpui::test]
fn multi_select_then_mark_done(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let start = h.counts();
    let ids = h.read(|a| a.mailbox.ids_in(Inbox));

    // Range select with shift-j.
    h.keys("shift-j shift-j");
    let sel = h.read(|a| a.triage.selected());
    assert_eq!(sel.len(), 3);
    assert!(sel.iter().all(|s| ids[..3].contains(s)));
    h.keys("e");
    for id in &ids[..3] {
        assert_eq!(h.state_of(*id), Done);
    }
    assert_eq!(h.count(Inbox), start[0] - 3);
    assert!(h.read(|a| a.triage.selected().is_empty()));

    // Undo restores all three in one step.
    h.keys("u");
    assert_eq!(h.counts(), start);

    // shift-k extends upward.
    h.keys("j j j shift-k");
    assert_eq!(h.read(|a| a.triage.selected().len()), 2);
    h.keys("escape");
    assert!(h.read(|a| a.triage.selected().is_empty()));
}

#[gpui_kit::gpui::test]
fn x_toggles_selection_then_mark_done(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let start = h.counts();
    let ids = h.read(|a| a.mailbox.ids_in(Inbox));
    h.keys("x j j x");
    let mut sel = h.read(|a| a.triage.selected());
    sel.sort();
    let mut want = vec![ids[0], ids[2]];
    want.sort();
    assert_eq!(sel, want);
    h.keys("e");
    assert_eq!(h.state_of(ids[0]), Done);
    assert_eq!(h.state_of(ids[2]), Done);
    assert_eq!(h.state_of(ids[1]), Inbox);
    assert_eq!(h.count(Done), start[3] + 2);
}

#[gpui_kit::gpui::test]
fn shift_e_marks_all_from_sender_done(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    // Move the cursor (via keys) to a message whose sender has several messages.
    let multi = |h: &mut Harness| {
        h.read(|a| {
            let id = a.triage.cursor(&a.mailbox).unwrap();
            let email = a.mailbox.get(id).unwrap().from_email.clone();
            let n = a.mailbox.messages().iter().filter(|m| m.from_email == email).count();
            (email, n)
        })
    };
    let mut guard = 0;
    while multi(&mut h).1 < 2 && guard < 60 {
        h.keys("j");
        guard += 1;
    }
    let (email, expected) = multi(&mut h);
    assert!(expected > 1, "fixture should have multiple msgs per sender");
    let start = h.counts();
    h.keys("shift-e");
    let remaining = h.read(|a| {
        a.mailbox
            .messages()
            .iter()
            .filter(|m| m.from_email == email && m.state != Done)
            .count()
    });
    assert_eq!(remaining, 0);
    assert!(h.count(Done) > start[3]);
    assert_eq!(h.accounted(), h.total());
    h.keys("u");
    assert_eq!(h.counts(), start);
}

#[gpui_kit::gpui::test]
fn counts_always_sum_to_total(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let total = h.total();
    let seq = [
        "e", "j", "w", "j j", "l escape", "shift-j", "i", "2", "j", "e", "3", "w", "u", "4", "k", "i",
        "1", "x", "j", "x", "l escape", "shift-w", "u u", "r", "a b", "cmd-enter", "shift-l", "j", "enter",
        "shift-i", "u", "cmd-k", "m a r k space d o n e", "enter", "?", "?", "e", "shift-e",
        "u u u", "2", "e", "x", "shift-k", "w", "escape",
    ];
    for step in seq {
        h.keys(step);
        assert_eq!(
            h.accounted(),
            total,
            "invariant broken after {step:?}"
        );
    }
}

#[gpui_kit::gpui::test]
fn help_overlay_fits_window_and_scrolls_at_any_size(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.keys("?");
    for (w, ht) in [(1200., 800.), (420., 300.), (700., 500.), (1600., 1000.)] {
        h.cx.simulate_window_resize(h.window, size(px(w), px(ht)));
        h.cx
            .update_window(h.window, |_, window, cx| window.draw(cx).clear(cx))
            .unwrap();
        h.keys("");
        let (b, max) = h.read(|a| a.help_metrics());
        let (x0, y0) = (f32::from(b.origin.x), f32::from(b.origin.y));
        let (x1, y1) = (x0 + f32::from(b.size.width), y0 + f32::from(b.size.height));
        assert!(b.size.width > px(0.) && b.size.height > px(0.), "help body not laid out at {w}x{ht}");
        assert!(x0 >= 0. && y0 >= 0. && x1 <= w && y1 <= ht, "help body {b:?} outside {w}x{ht}");
        if ht <= 300. {
            assert!(max > 0., "help must scroll in a {w}x{ht} window");
        }
    }
    h.keys("pagedown j k pageup");
    assert!(h.read(|a| a.help_open()));
    h.keys("escape");
    assert!(!h.read(|a| a.help_open()));
}
