//! The Triage reader uses the normal reader's message surface: its body is laid out inside the
//! reader viewport (and so its web view is visible), for the first message and after advancing.

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext};
use mail_classifier::model::Mailbox;

#[path = "ui_ext/harness.rs"]
#[allow(dead_code)]
mod harness;
use harness::{harness_with, mailbox};

/// One thread of `n` HTML messages, oldest first.
fn thread(n: u32) -> Mailbox {
    let messages: Vec<String> = (1..=n)
        .map(|i| {
            serde_json::json!({
                "id": i,
                "thread_id": 7,
                "from_name": "news",
                "from_email": "news@example.com",
                "to": "you@example.com",
                "subject": "Plans",
                "body": "Checking in.",
                "html": "<p>Hello</p>",
                "received": format!("2026-09-{:02}T09:00:00Z", 9 + i),
                "state": "Inbox",
                "snooze": null
            })
            .to_string()
        })
        .collect();
    mailbox(&messages)
}

impl harness::Harness<'_> {
    /// The opened message's body: present, laid out, and inside the reader viewport.
    fn body_bounds(&mut self) -> gpui_kit::Bounds<gpui_kit::Pixels> {
        let window = self.window;
        self.cx
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                let body = window.try_find("reader-body").expect("the body is rendered");
                assert!(body.bounds().size.width > gpui_kit::px(0.), "the body has width");
                assert!(body.bounds().size.height > gpui_kit::px(0.), "the body has height");
                assert!(body.visible(), "the body is inside the reader viewport, not clipped away");
                body.bounds()
            })
            .expect("window alive")
    }
}

#[gpui_kit::gpui::test]
fn triage_reader_body_is_narrower_than_normal_reader(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, thread(1));
    h.cx.simulate_window_resize(h.window, gpui_kit::size(gpui_kit::px(1800.), gpui_kit::px(900.)));
    h.keys("t");
    h.keys("enter");
    let triage_width = h.body_bounds().size.width;
    assert!(triage_width <= gpui_kit::px(720.), "triage body width: {triage_width:?}");

    h.keys("escape");
    h.keys("escape");
    h.keys("enter");
    let reader_width = h.body_bounds().size.width;
    assert!(reader_width > gpui_kit::px(720.), "reader body width: {reader_width:?}");
}

#[gpui_kit::gpui::test]
fn the_triage_reader_renders_each_current_message_body(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, thread(12));
    h.keys("t");
    h.app.update(h.cx, |a, cx| {
        a.setup_set_order(mail_classifier::triage_session::SessionOrder::OldestFirst, cx);
    });
    h.keys("enter");
    assert_eq!(h.read(|a| a.opened()), Some(1));

    let first = h.body_bounds();
    h.keys("e");
    assert_eq!(h.read(|a| a.opened()), Some(2), "the session advanced");
    let second = h.body_bounds();
    assert!(second.size.height > gpui_kit::px(0.));
    assert!(first.size.height > gpui_kit::px(0.));
}

#[gpui_kit::gpui::test]
fn possible_spam_banner_is_hidden_only_in_triage(cx: &mut TestAppContext) {
    let mut messages = thread(1);
    messages.add_suggestions(vec![mail_classifier::judge::Suggestion {
        message: 1,
        key: mail_classifier::judge::QuestionKey::Spam,
        answer: mail_classifier::judge::Answer {
            probabilities: vec![],
            value: mail_classifier::judge::AnswerValue::Bool(true),
            confidence: 1.0,
        },
    }]);
    messages.accept_suggestions(1, 0);
    let mut h = harness_with(cx, messages);
    h.keys("enter");

    let normal_banner = h.cx.update_window(h.window, |_, window, cx| {
        window.render_frame(cx);
        window.try_find("banner-possible-spam").is_some()
    }).expect("window alive");
    assert!(normal_banner, "normal reader shows the Possible-Spam banner");

    h.keys("t enter");
    let triage_banner = h.cx.update_window(h.window, |_, window, cx| {
        window.render_frame(cx);
        window.try_find("banner-possible-spam").is_some()
    }).expect("window alive");
    assert!(!triage_banner, "triage reader hides the Possible-Spam banner");
}


