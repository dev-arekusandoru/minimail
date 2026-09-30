//! Headless keystroke tests for the reader: the action-toolbar setting, thread expansion,
//! Reader mode and the footer hint mode. Mailboxes are built inline so nothing depends on the
//! shipped fixtures.

use gpui_kit::{AppContext, TestAppContext};
use gpui_kit::test::TestWindowExt;
use mail_classifier::hints::{HintContext, HintMode};
use mail_classifier::judge::{Answer, AnswerValue, QuestionKey, Suggestion};
use mail_classifier::model::{Mailbox, MessageId};

#[path = "ui_ext/harness.rs"]
#[allow(dead_code)]
mod harness;
use harness::{Harness, harness_with, json, msg};

/// `message` (a harness `msg()` JSON object) with extra top-level fields merged in.
fn with_fields(message: String, fields: serde_json::Value) -> String {
    let mut value: serde_json::Value = serde_json::from_str(&message).expect("valid message json");
    let object = value.as_object_mut().expect("message is an object");
    for (key, field) in fields.as_object().expect("fields are an object") {
        object.insert(key.clone(), field.clone());
    }
    value.to_string()
}

fn mailbox_of(messages: &[String]) -> Mailbox {
    Mailbox::from_json(&json(messages)).expect("valid mailbox json")
}

/// Two messages in one thread.
fn thread_of_two() -> Mailbox {
    mailbox_of(&[
        msg(1, 7, "alice@example.com", "Plans", 1, "Inbox"),
        msg(2, 7, "bob@example.com", "Re: Plans", 2, "Inbox"),
    ])
}

impl Harness<'_> {
    fn exists(&mut self, id: &'static str) -> bool {
        let window = self.window;
        self.cx
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.try_find(id).is_some()
            })
            .expect("window alive")
    }

    fn opened(&mut self) -> MessageId {
        self.read(|a| a.opened()).expect("a message is open")
    }

    fn hint(&mut self) -> HintContext {
        self.read(|a| a.hint_context())
    }

    /// Settings ▸ Appearance ▸ Reader action toolbar, then close the panel.
    fn toggle_toolbar_setting(&mut self) {
        self.keys("cmd-,");
        self.click(("settings-section", 1usize));
        self.click("reader-toolbar-row");
        self.keys("escape");
        assert!(!self.read(|a| a.settings_open()));
    }
}

#[gpui_kit::gpui::test]
fn reader_toolbar_is_off_by_default_and_the_setting_toggles_it(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, thread_of_two());
    assert!(!h.read(|a| a.reader_toolbar), "keyboard hints only by default");

    h.keys("enter");
    h.opened();
    assert!(!h.exists("btn-reader-archive"), "no action buttons while the toolbar is off");

    h.toggle_toolbar_setting();
    assert!(h.read(|a| a.reader_toolbar));
    h.opened();
    assert!(h.exists("btn-reader-archive"), "the toolbar appears once the setting is on");

    h.toggle_toolbar_setting();
    assert!(!h.read(|a| a.reader_toolbar), "the same row turns it off again");
    h.opened();
    assert!(!h.exists("btn-reader-archive"));
}

#[gpui_kit::gpui::test]
fn shift_o_expands_then_collapses_the_other_thread_messages(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, thread_of_two());
    h.keys("enter");
    let opened = h.opened();
    let other = if opened == 1 { 2 } else { 1 };
    assert!(!h.read(|a| a.reader_expanded(other)), "earlier messages start collapsed");

    h.keys("shift-o");
    assert!(h.read(|a| a.reader_expanded(other)), "shift-o expands the rest of the thread");

    h.keys("shift-o");
    assert!(!h.read(|a| a.reader_expanded(other)), "a second shift-o collapses them again");
    assert_eq!(h.opened(), opened, "expansion never changes which message is open");
}

#[gpui_kit::gpui::test]
fn shift_o_on_a_single_message_thread_does_nothing(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox_of(&[msg(1, 1, "alice@example.com", "Solo", 1, "Inbox")]));
    h.keys("enter");
    h.keys("shift-o");
    assert!(!h.read(|a| a.reader_expanded(1)));
    assert_eq!(h.opened(), 1);
}

#[gpui_kit::gpui::test]
fn clicking_a_thread_message_row_toggles_just_that_message(cx: &mut TestAppContext) {
    let mut h = harness_with(
        cx,
        mailbox_of(&[
            msg(1, 7, "alice@example.com", "Plans", 1, "Inbox"),
            msg(2, 7, "bob@example.com", "Re: Plans", 2, "Inbox"),
            msg(3, 7, "carol@example.com", "Re: Re: Plans", 3, "Inbox"),
        ]),
    );
    h.keys("enter");
    let opened = h.opened();
    let others: Vec<MessageId> = [1, 2, 3].into_iter().filter(|id| *id != opened).collect();

    h.click(("reader-thread-msg", others[0] as usize));
    assert!(h.read(|a| a.reader_expanded(others[0])));
    assert!(!h.read(|a| a.reader_expanded(others[1])), "siblings stay collapsed");

    h.click(("reader-thread-msg", others[0] as usize));
    assert!(!h.read(|a| a.reader_expanded(others[0])));
}

#[gpui_kit::gpui::test]
fn v_toggles_reader_mode_only_for_an_html_message(cx: &mut TestAppContext) {
    let html = with_fields(
        msg(1, 1, "news@example.com", "Weekly digest", 1, "Inbox"),
        serde_json::json!({ "html": "<h1>Digest</h1><p>Hello <b>there</b></p>" }),
    );
    let text = msg(2, 2, "alice@example.com", "Plain note", 2, "Inbox");
    let mut h = harness_with(cx, mailbox_of(&[html, text]));

    // Newest first: the text message is on top, so visit it before moving down.
    h.goto(2);
    h.keys("enter");
    assert_eq!(h.opened(), 2);
    h.keys("v");
    assert!(!h.read(|a| a.reader_plain(2)), "a text-only message has no Reader mode");

    h.goto(1);
    h.keys("enter");
    assert_eq!(h.opened(), 1);
    assert!(!h.read(|a| a.reader_plain(1)), "HTML is shown as designed until asked otherwise");
    h.keys("v");
    assert!(h.read(|a| a.reader_plain(1)), "v switches the HTML message to Reader mode");
    h.keys("v");
    assert!(!h.read(|a| a.reader_plain(1)), "v switches back");
}

#[gpui_kit::gpui::test]
fn reader_hints_follow_the_open_message_and_pending_suggestions(cx: &mut TestAppContext) {
    let mut mb = mailbox_of(&[msg(1, 1, "alice@example.com", "Lunch?", 1, "Inbox")]);
    mb.add_suggestions(vec![Suggestion {
        message: 1,
        key: QuestionKey::NeedsReply,
        answer: Answer { probabilities: vec![0.1, 0.9], value: AnswerValue::Bool(true), confidence: 0.9 },
    }]);
    let mut h = harness_with(cx, mb);
    let c = h.hint();
    assert_eq!((c.mode, c.suggestions), (HintMode::List, true), "the list sees the cursor's suggestions");

    h.keys("enter");
    let c = h.hint();
    assert_eq!((c.mode, c.suggestions), (HintMode::Reader, true), "pending suggestions add accept/reject");

    h.keys("n");
    let c = h.hint();
    assert_eq!((c.mode, c.suggestions), (HintMode::Reader, false), "rejecting clears the suggestion hints");

    h.keys("x");
    assert_eq!(h.hint().mode, HintMode::Selection(1), "a selection takes the footer over from the reader");
}
