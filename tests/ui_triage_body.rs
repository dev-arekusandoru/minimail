//! Triage advances: the next queued bodies are fetched before they are current, and the email
//! card keeps its skeleton until the document behind it is ready.

use std::rc::Rc;
use std::sync::{Arc, Mutex};

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext};
use mail_classifier::clock::Timestamp;
use mail_classifier::model::Mailbox;
use mail_classifier::provider::{
    Body, Changes, MailProvider, Page, ProviderError, RemoteFlags, RemoteFolder, RemoteId,
    RemoteMessage, RemoteState, Scope, Window,
};
use mail_classifier::sync::cache::Cache;
use mail_classifier::triage_session::SessionOrder;

#[path = "ui_ext/harness.rs"]
#[allow(dead_code)]
mod harness;
use harness::{harness_with, mailbox};

/// `n` header-only messages, one thread each, oldest first.
fn partial_queue(n: u32) -> Mailbox {
    let messages: Vec<String> = (1..=n)
        .map(|i| {
            serde_json::json!({
                "id": i,
                "thread_id": i,
                "from_name": "news",
                "from_email": "news@example.com",
                "to": "you@example.com",
                "subject": "Plans",
                "body": "Checking in.",
                "received": format!("2026-09-{:02}T09:00:00Z", i),
                "state": "Inbox",
                "snooze": null,
                "partial": true
            })
            .to_string()
        })
        .collect();
    mailbox(&messages)
}

/// `n` messages with an HTML body, one thread each, oldest first.
fn html_queue(n: u32) -> Mailbox {
    let messages: Vec<String> = (1..=n)
        .map(|i| {
            serde_json::json!({
                "id": i,
                "thread_id": i,
                "from_name": "news",
                "from_email": "news@example.com",
                "to": "you@example.com",
                "subject": "Plans",
                "body": "Checking in.",
                "html": "<p>Hello</p>",
                "received": format!("2026-09-{:02}T09:00:00Z", i),
                "state": "Inbox",
                "snooze": null
            })
            .to_string()
        })
        .collect();
    mailbox(&messages)
}

/// A provider that serves every body immediately and records which ids were asked for.
struct FakeProvider {
    fetched: Arc<Mutex<Vec<RemoteId>>>,
}

impl MailProvider for FakeProvider {
    fn folders(&mut self) -> Result<Vec<RemoteFolder>, ProviderError> {
        unreachable!()
    }
    fn create_folder(&mut self, _: &str) -> Result<RemoteFolder, ProviderError> {
        unreachable!()
    }
    fn rename_folder(&mut self, _: &str, _: &str) -> Result<RemoteFolder, ProviderError> {
        unreachable!()
    }
    fn delete_folder(&mut self, _: &str) -> Result<(), ProviderError> {
        unreachable!()
    }
    fn cursor(&mut self) -> Result<String, ProviderError> {
        unreachable!()
    }
    fn list(
        &mut self,
        _: &Scope,
        _: Window,
        _: Option<&str>,
        _: usize,
    ) -> Result<Page, ProviderError> {
        unreachable!()
    }
    fn snapshot(&mut self, _: Timestamp) -> Result<Vec<(RemoteId, RemoteFlags)>, ProviderError> {
        unreachable!()
    }
    fn changes(&mut self, _: &str) -> Result<Changes, ProviderError> {
        unreachable!()
    }
    fn fetch_headers(&mut self, _: &[RemoteId]) -> Result<Vec<RemoteMessage>, ProviderError> {
        unreachable!()
    }
    fn fetch_body(&mut self, id: &RemoteId) -> Result<Body, ProviderError> {
        self.fetched.lock().unwrap().push(id.clone());
        Ok(Body { body: format!("full body of {id}"), html: None, attachments: Vec::new() })
    }
    fn move_message(&mut self, _: &RemoteId, _: &RemoteState, _: &RemoteState) -> Result<(), ProviderError> {
        unreachable!()
    }
    fn set_read(&mut self, _: &RemoteId, _: bool) -> Result<(), ProviderError> {
        unreachable!()
    }
}

/// A cache in memory holding `mb`'s messages as remote, header-only mail of the first account.
fn cache_for(mb: &Mailbox) -> (Rc<Cache>, String) {
    let cache = Rc::new(Cache::open_in_memory().expect("in-memory cache"));
    let account = mb.accounts().first().expect("an account").id.clone();
    for m in mb.messages() {
        cache
            .upsert_message(m, &format!("r{}", m.id), &RemoteState::Inbox, false)
            .expect("cache the message");
    }
    (cache, account)
}

#[gpui_kit::gpui::test]
fn the_next_queued_bodies_land_before_their_message_is_current(cx: &mut TestAppContext) {
    let mb = partial_queue(3);
    let (cache, account) = cache_for(&mb);
    let fetched: Arc<Mutex<Vec<RemoteId>>> = Arc::default();
    let mut h = harness_with(cx, mb);
    h.app.update(h.cx, |app, _| {
        app.attach_test_sync(cache, &account, Box::new(FakeProvider { fetched: fetched.clone() }));
    });
    h.keys("t");
    h.app.update(h.cx, |a, cx| a.setup_set_order(SessionOrder::OldestFirst, cx));
    h.keys("enter");
    h.cx.run_until_parked();
    assert_eq!(h.read(|a| a.opened()), Some(1), "the session starts on the oldest message");

    let asked = fetched.lock().unwrap().clone();
    assert!(
        asked.contains(&"r2".to_owned()),
        "the next message's body is fetched while it is still ahead of the cursor: {asked:?}"
    );
    assert!(
        asked.contains(&"r3".to_owned()),
        "the one after it is prefetched too: {asked:?}"
    );

    h.keys("space");
    assert_eq!(h.read(|a| a.opened()), Some(2), "the session advanced");
    assert!(
        h.read(|a| a.mailbox.get(2).is_some_and(|m| !m.partial)),
        "the body was already there when the message became current"
    );
}

#[gpui_kit::gpui::test]
fn a_body_keeps_its_skeleton_until_the_document_is_ready(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, html_queue(2));
    h.keys("t");
    h.app.update(h.cx, |a, cx| a.setup_set_order(SessionOrder::OldestFirst, cx));
    h.keys("enter");
    assert_eq!(h.read(|a| a.opened()), Some(1));
    assert!(h.has(("reader-html", 1usize)), "the current message's body is up");

    // Advance without rendering: the next frame builds it for the first time.
    let app = h.app.clone();
    h.cx.update_window(h.window, |_, window, cx| {
        app.update(cx, |app, cx| app.skip_current(window, cx));
    })
    .expect("window alive");

    let frame = |h: &mut harness::Harness<'_>| {
        h.cx.update_window(h.window, |_, window, cx| {
            window.render_frame(cx);
            (
                window.try_find("reader-body-skeleton").is_some(),
                window.try_find(("reader-html", 2usize)).is_some(),
            )
        })
        .expect("window alive")
    };
    let (skeleton, body) = frame(&mut h);
    assert!(skeleton, "the new body has a skeleton the moment it is current");
    assert!(!body, "the page behind it is not up yet");
    assert!(skeleton != body, "the card is never blank between the two");

    h.cx.run_until_parked();
    let (skeleton, body) = frame(&mut h);
    assert!(!skeleton, "the skeleton goes once the document is ready");
    assert!(body, "the body takes its place");
}
