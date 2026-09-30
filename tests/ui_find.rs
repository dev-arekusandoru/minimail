//! Headless UI tests for find in the reader tab (`cmd-f`) and global search (`cmd-shift-f`).

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, ElementId, TestAppContext};
use mail_classifier::model::{Mailbox, MessageId};

#[path = "ui_ext/harness.rs"]
#[allow(dead_code)]
mod harness;
use harness::{Harness, harness_with};

/// Thread 1: messages 1-3 (day 1-3), "budget" in the subject, two bodies and a quote; thread 2:
/// message 4. Inbox order, newest first: 4, 3, 2, 1. Opened on 3, the matches of "budget" are
/// 3's subject, 1's body, 2's body and 2's (folded) quote, in that order.
fn mailbox() -> Mailbox {
    let m = |id: u32, thread: u32, day: u32, subject: &str, body: &str| {
        serde_json::json!({
            "id": id, "thread_id": thread, "from_name": "Ann", "from_email": "ann@example.com",
            "to": "me@example.com", "subject": subject, "body": body,
            "received": format!("2026-09-{day:02}T09:00:00Z"), "state": "Inbox",
        })
    };
    Mailbox::from_json(
        &serde_json::json!([
            m(1, 1, 1, "Budget", "Draft budget attached."),
            m(2, 1, 2, "Re: Budget", "Agreed on the budget.\n\n> The budget is tight."),
            m(3, 1, 3, "Re: Budget", "Thanks"),
            m(4, 2, 4, "Lunch", "Noon?"),
        ])
        .to_string(),
    )
    .expect("valid mailbox")
}

impl Harness<'_> {
    fn row(&mut self, id: MessageId) {
        self.click(("row", id as usize));
    }
    fn exists(&mut self, id: impl Into<ElementId>) -> bool {
        let id = id.into();
        self.cx
            .update_window(self.window, |_, window, cx| {
                window.render_frame(cx);
                window.try_find(id).is_some()
            })
            .expect("window alive")
    }
    fn found(&mut self) -> Option<(usize, usize)> {
        self.read(|a| a.find_count())
    }
    fn opened(&mut self) -> Option<MessageId> {
        self.read(|a| a.opened())
    }
    /// Open the bar on thread 1 and search for `query`.
    fn find(&mut self, query: &str) {
        self.row(3);
        self.keys("cmd-f");
        self.type_text(query);
    }
}

#[gpui_kit::gpui::test]
fn cmd_f_opens_the_bar_and_typing_counts_matches(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox());
    h.keys("cmd-f");
    assert!(!h.read(|a| a.find_open()), "no reader tab, no find bar");
    h.row(3);
    assert!(!h.exists("find-bar"));
    h.keys("cmd-f");
    assert!(h.read(|a| a.find_open()));
    assert!(h.exists("find-bar"));
    assert_eq!(h.found(), Some((0, 0)), "nothing typed yet");

    h.type_text("budget");
    assert_eq!(h.found(), Some((1, 4)), "subject, two bodies and one folded quote; first is current");
    h.type_text("x");
    assert_eq!(h.found(), Some((0, 0)), "\"budgetx\" matches nothing");
}

#[gpui_kit::gpui::test]
fn enter_steps_through_matches_expanding_collapsed_messages_and_quotes(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox());
    h.find("budget");
    assert!(!h.read(|a| a.tabs.tabs()[0].pinned));
    assert!(!h.read(|a| a.reader_expanded(1)));

    h.keys("enter");
    assert_eq!(h.found(), Some((2, 4)), "now in message 1's body");
    assert!(h.read(|a| a.reader_expanded(1)), "landing expands the collapsed message");
    assert!(h.read(|a| a.tabs.tabs()[0].pinned), "which pins the tab");

    h.keys("enter");
    assert_eq!(h.found(), Some((3, 4)));
    assert!(h.read(|a| a.reader_expanded(2)));
    assert!(!h.read(|a| a.reader_quoted_open(2)));

    h.keys("enter");
    assert_eq!(h.found(), Some((4, 4)), "the folded quote of message 2");
    assert!(h.read(|a| a.reader_quoted_open(2)), "landing in hidden quoted text reveals it");

    h.keys("enter");
    assert_eq!(h.found(), Some((1, 4)), "wraps forward");
    h.keys("shift-enter");
    assert_eq!(h.found(), Some((4, 4)), "and backward");
    h.keys("cmd-shift-g");
    assert_eq!(h.found(), Some((3, 4)));
    h.keys("cmd-g");
    assert_eq!(h.found(), Some((4, 4)));
    assert_eq!(h.opened(), Some(3), "finding never changes the opened message");
}

#[gpui_kit::gpui::test]
fn the_option_toggles_change_the_results(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox());
    h.find("budget");
    assert_eq!(h.found(), Some((1, 4)));

    h.keys("alt-c");
    assert_eq!(h.found(), Some((1, 3)), "case-sensitive: \"Budget\" in the subject no longer matches");
    h.keys("alt-c");
    assert_eq!(h.found(), Some((1, 4)));

    h.keys("alt-w");
    assert!(h.read(|a| a.find_options().unwrap().whole_word));
    assert_eq!(h.found(), Some((1, 4)), "every hit here is a whole word");
    h.keys("alt-w");

    // Regex: "bud.et" is literal text until the toggle is on.
    h.keys("backspace backspace backspace backspace backspace backspace");
    h.type_text("bud.et");
    assert_eq!(h.found(), Some((0, 0)));
    h.keys("alt-r");
    assert_eq!(h.found(), Some((1, 4)));

    h.type_text("(");
    assert!(h.read(|a| a.find_invalid()), "\"bud.et(\" does not compile");
    assert_eq!(h.found(), Some((0, 0)), "an invalid regex matches nothing and does not panic");
    h.keys("backspace");
    assert!(!h.read(|a| a.find_invalid()));
    assert_eq!(h.found(), Some((1, 4)));

    // The buttons do the same as the keys.
    h.click("find-regex");
    assert!(!h.read(|a| a.find_options().unwrap().regex));
    assert_eq!(h.found(), Some((0, 0)));
}

#[gpui_kit::gpui::test]
fn escape_closes_the_bar_and_returns_focus_to_the_app(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox());
    h.find("budget");
    h.keys("escape");
    assert!(!h.read(|a| a.find_open()));
    assert!(!h.exists("find-bar"));
    assert_eq!(h.found(), None, "the query is dropped with its highlights");
    let before = h.read(|a| a.triage.cursor_index());
    h.keys("k");
    assert_ne!(h.read(|a| a.triage.cursor_index()), before, "list keys work again");
}

#[gpui_kit::gpui::test]
fn each_tab_keeps_its_own_find_and_closing_the_tab_drops_it(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox());
    h.find("budget");
    h.keys("enter");
    assert_eq!(h.found(), Some((2, 4)));
    assert!(h.read(|a| a.tabs.tabs()[0].pinned), "landing pinned the tab");

    h.row(4);
    assert!(!h.read(|a| a.find_open()), "the new tab has no find bar");
    assert!(!h.exists("find-bar"));
    h.click(("reader-tab", 1usize));
    assert_eq!(h.found(), Some((2, 4)), "back on the first tab: same query, same position");
    assert!(h.exists("find-bar"));

    h.keys("escape cmd-w");
    h.row(3);
    h.keys("cmd-f");
    assert_eq!(h.found(), Some((0, 0)), "a reopened tab starts with an empty query");
}

#[gpui_kit::gpui::test]
fn cmd_shift_f_opens_global_search_like_slash(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox());
    h.row(3);
    h.keys("cmd-shift-f");
    assert!(h.read(|a| a.palette_open()));
    assert_eq!(h.app.read_with(h.cx, |a, cx| a.palette_query(cx)), Some("/".to_owned()));
    h.keys("escape");
    assert!(!h.read(|a| a.palette_open()));
    h.keys("/");
    assert!(h.read(|a| a.palette_open()), "`/` still works");
}
