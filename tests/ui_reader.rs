//! Headless keystroke tests for the reader: per-message menus, thread expansion,
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
    fn exists(&mut self, id: impl Into<gpui_kit::ElementId>) -> bool {
        let id = id.into();
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

    /// The accessibility label registered at `id` on the last frame, if the element is there.
    fn label(&mut self, id: impl Into<gpui_kit::ElementId>) -> Option<String> {
        let id = id.into();
        let window = self.window;
        self.cx
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.try_find(id).and_then(|snap| snap.label().map(str::to_owned))
            })
            .expect("window alive")
    }
}

/// The `⋯` on an expanded thread message that is not the opened one acts on that message only.
#[gpui_kit::gpui::test]
fn the_menu_of_a_non_opened_thread_message_archives_that_message(cx: &mut TestAppContext) {
    use mail_classifier::model::TriageState::{Archived, Inbox};
    let mut h = harness_with(cx, thread_of_two());
    h.keys("enter");
    let opened = h.opened();
    let other = if opened == 1 { 2 } else { 1 };
    h.keys("shift-o");
    assert!(h.read(|a| a.reader_expanded(other)));

    h.click(("btn-message-more", other as usize));
    assert!(h.read(|a| a.menu_open()));
    h.click_row("Archive");
    assert!(!h.read(|a| a.menu_open()));
    assert_eq!(h.state_of(other), Archived, "the message whose menu it was");
    assert_eq!(h.state_of(opened), Inbox, "the opened message is untouched");

    h.keys("u");
    assert_eq!(h.state_of(other), Inbox, "one undo step");

    // The keyboard path still targets the opened message.
    h.keys("e");
    assert_eq!(h.state_of(opened), Archived);
    assert_eq!(h.state_of(other), Inbox);
}

/// The reader's `⋯` menu keeps working for flows that outlive the dispatch (the folder picker).
#[gpui_kit::gpui::test]
fn filing_from_a_non_opened_message_menu_files_that_message(cx: &mut TestAppContext) {
    use mail_classifier::model::TriageState::{Filed, Inbox};
    let mut h = harness_with(cx, thread_of_two());
    h.keys("enter");
    let opened = h.opened();
    let other = if opened == 1 { 2 } else { 1 };
    h.keys("shift-o");

    h.click(("btn-message-more", other as usize));
    h.click_row("File…");
    assert!(h.read(|a| a.folder_picker_open()));
    h.keys("enter");
    assert!(matches!(h.state_of(other), Filed(_)), "the menu's message was filed");
    assert_eq!(h.state_of(opened), Inbox);
}

/// Reply on an expanded thread message replies to that message.
#[gpui_kit::gpui::test]
fn reply_on_a_thread_message_opens_compose_for_it(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, thread_of_two());
    h.keys("enter");
    let opened = h.opened();
    let other = if opened == 1 { 2 } else { 1 };
    h.keys("shift-o");
    assert!(h.exists(("btn-reply", opened as usize)), "the opened message has its own Reply");
    h.click(("btn-reply", other as usize));
    assert!(h.read(|a| a.compose_open()));
    assert_eq!(h.opened(), other, "the reply is to the message whose button was pressed");
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

/// An HTML message renders into one web view per tab: it loads once, switching tabs or Reader mode
/// never reloads it, and closing the tab frees it so the next open loads afresh.
#[gpui_kit::gpui::test]
fn an_html_message_loads_once_per_tab_and_again_after_the_tab_closes(cx: &mut TestAppContext) {
    let page = |id: u32, subject: &str, body: &str| {
        with_fields(
            msg(id, id, "news@example.com", subject, id, "Inbox"),
            serde_json::json!({ "html": body }),
        )
    };
    let mut h = harness_with(
        cx,
        mailbox_of(&[page(1, "Digest", "<h1>Digest</h1>"), page(2, "Offer", "<p>Offer</p>")]),
    );
    let loads = |h: &mut Harness<'_>| h.read(|a| a.webview_loads());

    // Newest first: message 2 is on top. `enter` twice opens and pins its tab.
    h.goto(2);
    h.keys("enter enter");
    assert!(h.exists(("reader-html", 2usize)), "the HTML body is a web view element");
    let first = loads(&mut h);
    assert_eq!(first.len(), 1);
    assert!(first[0].document.contains("<p>Offer</p>"), "the message is loaded as received");

    h.goto(1);
    h.keys("enter enter");
    assert_eq!(loads(&mut h).len(), 2, "each tab has its own document");

    h.keys("ctrl-tab");
    assert_eq!(h.opened(), 2);
    h.keys("ctrl-tab");
    assert_eq!(h.opened(), 1);
    assert_eq!(loads(&mut h).len(), 2, "switching tabs reloads nothing");

    h.keys("v");
    assert!(!h.exists(("reader-html", 1usize)), "Reader mode shows text instead of the web view");
    h.keys("v");
    assert!(h.exists(("reader-html", 1usize)));
    assert_eq!(loads(&mut h).len(), 2, "leaving Reader mode reuses the loaded view");

    h.keys("cmd-w");
    h.goto(1);
    h.keys("enter");
    assert_eq!(loads(&mut h).len(), 3, "a closed tab's view is gone, so reopening loads again");
}

/// The `Block remote images` setting changes what the reader renders: turning it on blocks the
/// remote image (the document reloads under the block policy) and reports the count, turning it
/// off reloads without it.
#[gpui_kit::gpui::test]
fn the_remote_image_setting_switches_the_reader_policy(cx: &mut TestAppContext) {
    // A loopback pixel: with blocking off the engine's SSRF policy refuses it without a network
    // fetch, so the test stays offline.
    let html = with_fields(
        msg(1, 1, "news@example.com", "Weekly digest", 1, "Inbox"),
        serde_json::json!({
            "html": "<p>Hello</p><img src=\"http://127.0.0.1:9/pixel.png\" alt=\"Pixel\">",
        }),
    );
    let mut h = harness_with(cx, mailbox_of(&[html]));

    // Block remote images is the Reader page's only row (sidebar index 3).
    let set_blocking = |h: &mut Harness<'_>, on: bool| {
        h.settings_open_page(3);
        h.settings_click_in(0, 0, "check");
        assert_eq!(h.read(|a| a.block_remote_images), on, "the switch sets the policy");
        h.settings_keys("escape");
        assert!(!h.read(|a| a.settings_open()));
    };

    h.keys("enter");
    assert!(!h.exists("reader-images-blocked"), "images load by default, nothing is blocked");

    set_blocking(&mut h, true);
    assert!(h.exists("reader-images-blocked"), "blocking reports the blocked image count");
    let loaded = h.read(|a| a.webview_loads());
    assert_eq!(loaded.len(), 2, "the policy change reloads the document once");
    assert!(loaded[1].style.block && !loaded[0].style.block);

    set_blocking(&mut h, false);
    assert!(!h.exists("reader-images-blocked"), "unblocking stops reporting blocked images");
    assert_eq!(h.read(|a| a.webview_loads()).len(), 3);
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

/// The labels row shows classifier chips as words: an urgency level a row hides (2) and kind
/// Other both render, and the raw `n/5` never does.
#[gpui_kit::gpui::test]
fn reader_labels_show_classifier_chips(cx: &mut TestAppContext) {
    use mail_classifier::judge::Kind;
    use mail_classifier::model::Tag;
    let mut mb = mailbox_of(&[msg(1, 1, "alice@example.com", "Lunch?", 1, "Inbox")]);
    mb.set_label(1, QuestionKey::Urgency, Some(Tag::Urgent(2)));
    mb.set_label(1, QuestionKey::Kind, Some(Tag::Kind(Kind::Other)));
    let mut h = harness_with(cx, mb);
    h.keys("enter");
    assert_eq!(h.label(("reader-chip-urgency", 1u64)).as_deref(), Some("Low"));
    assert_eq!(h.label(("reader-chip-kind", 1u64)).as_deref(), Some("Other"));
    assert!(!h.label(("reader-chip-urgency", 1u64)).unwrap().contains('/'), "no N/5 form");
}

/// `mb` with a pending Folder suggestion on message 1 pointing at its first folder; returns the
/// target folder.
fn with_folder_suggestion(mb: &mut Mailbox) -> mail_classifier::model::FolderId {
    let options = mb.folder_options("personal");
    assert!(!options.is_empty(), "the fixture account has folders");
    mb.add_suggestions(vec![Suggestion {
        message: 1,
        key: QuestionKey::Folder,
        answer: Answer {
            probabilities: vec![1.0; options.len() + 1],
            value: AnswerValue::Choice(0),
            confidence: 0.9,
        },
    }]);
    options[0].0
}

/// A pending non-folder suggestion is not a folder suggestion: the strip stays away.
#[gpui_kit::gpui::test]
fn reader_folder_strip_needs_a_folder_suggestion(cx: &mut TestAppContext) {
    let mut mb = mailbox_of(&[msg(1, 1, "alice@example.com", "Lunch?", 1, "Inbox")]);
    mb.add_suggestions(vec![Suggestion {
        message: 1,
        key: QuestionKey::NeedsReply,
        answer: Answer { probabilities: vec![0.1, 0.9], value: AnswerValue::Bool(true), confidence: 0.9 },
    }]);
    let mut h = harness_with(cx, mb);
    h.keys("enter");
    assert!(!h.exists("reader-folder-suggestion"));
}

/// `y` on the folder strip (the Move button) files the message into the suggested folder.
#[gpui_kit::gpui::test]
fn reader_folder_strip_move_files_the_message(cx: &mut TestAppContext) {
    use mail_classifier::model::TriageState;
    let mut mb = mailbox_of(&[msg(1, 1, "alice@example.com", "Lunch?", 1, "Inbox")]);
    let target = with_folder_suggestion(&mut mb);
    let mut h = harness_with(cx, mb);
    h.keys("enter");
    assert!(h.exists("reader-folder-suggestion"));
    h.keys("y");
    assert_eq!(h.state_of(1), TriageState::Filed(target));
    assert!(!h.exists("reader-folder-suggestion"), "accepting clears the strip");
}

/// `n` on the folder strip (the Dismiss button) drops the suggestion without filing.
#[gpui_kit::gpui::test]
fn reader_folder_strip_dismiss_keeps_the_message(cx: &mut TestAppContext) {
    use mail_classifier::model::TriageState;
    let mut mb = mailbox_of(&[msg(1, 1, "alice@example.com", "Lunch?", 1, "Inbox")]);
    with_folder_suggestion(&mut mb);
    let mut h = harness_with(cx, mb);
    h.keys("enter");
    assert!(h.exists("reader-folder-suggestion"));
    h.keys("n");
    assert!(h.read(|a| a.mailbox.pending(1).is_empty()), "Dismiss drops the suggestion");
    assert_eq!(h.state_of(1), TriageState::Inbox);
    assert!(!h.exists("reader-folder-suggestion"));
}
