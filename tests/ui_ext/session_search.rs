use gpui_kit::{TestAppContext};
use mail_classifier::model::{Mailbox, MessageId};
use mail_classifier::model::TriageState::*;
use crate::harness::{Harness, harness_with, mailbox, msg};
use crate::snooze::snooze_box;

// ---------------------------------------------------------------- Triage session

#[gpui_kit::gpui::test]
pub fn triage_session_auto_advances_and_ends_with_summary(cx: &mut TestAppContext) {
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
    assert_eq!(h.state_of(1), Archived);
    assert_eq!(h.read(|a| a.session_progress()), Some((2, 3)));
    h.clock.advance(65);
    h.keys("e");
    assert_eq!(h.state_of(2), Archived);
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
pub fn triage_session_escape_ends_early(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, snooze_box());
    h.keys("t e");
    assert_eq!(h.read(|a| a.session_progress()), Some((2, 3)));
    h.keys("escape");
    assert_eq!(h.read(|a| a.session_progress()), None);
    assert_eq!(h.state_of(1), Archived);
    assert_eq!(h.state_of(2), Inbox);
    assert_eq!(h.read(|a| a.session_end()), Some((1, 0)), "escape after handling shows the end card");
    h.keys("escape");
    assert_eq!(h.read(|a| a.session_end()), None, "a second escape dismisses the card");
    h.keys("t escape");
    assert_eq!(h.read(|a| a.session_end()), None, "nothing handled: no card");
}

// ---------------------------------------------------------------- Search

pub fn search_box() -> Mailbox {
    mailbox(&[
        msg(1, 1, "dana@x.io", "Invoice for lunch", 25, "Inbox"),
        msg(2, 2, "dana@x.io", "Roadmap", 10, "Inbox"),
        msg(3, 3, "omar@x.io", "Invoice overdue", 12, "Snoozed"),
        msg(4, 4, "omar@x.io", "Team offsite", 5, "Archived"),
    ])
}

/// Search for `q` from wherever the list is, then drop the pre-seeded folder pill, so the
/// results are the query over all mail. Returns the ids; the search is cleared afterwards.
pub fn search(h: &mut Harness<'_>, q: &str) -> Vec<MessageId> {
    h.keys("/");
    assert!(h.read(|a| a.palette_open()));
    h.type_text(q.trim_start_matches('/'));
    h.keys("enter");
    assert!(
        h.read(|a| a.search_header()).is_some_and(|s| s.starts_with("search:")),
        "the query is applied on top of the folder being browsed"
    );
    if h.read(|a| a.location()).is_some() {
        assert!(h.has("pill-remove-in-inbox"), "the folder is a removable pill now: q={} pills={:?} header={:?}", h.query(), h.pills(), h.header());
        h.click("pill-remove-in-inbox");
        assert_eq!(h.read(|a| a.location()), None, "removing the folder goes global");
    }
    let ids = h.visible();
    h.keys("escape");
    assert_eq!(h.read(|a| a.search_header()), None, "escape clears the search");
    ids
}

#[gpui_kit::gpui::test]
pub fn search_operators(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, search_box());
    let sorted = |h: &mut Harness<'_>, q: &str| {
        let mut v = search(h, q);
        v.sort();
        v
    };
    assert_eq!(sorted(&mut h, "from:dana"), vec![1, 2]);
    assert_eq!(sorted(&mut h, "from:OMAR@x.io"), vec![3, 4], "case-insensitive");
    assert_eq!(sorted(&mut h, "subject:invoice"), vec![1, 3]);
    assert_eq!(sorted(&mut h, "is:snoozed"), vec![3]);
    assert_eq!(sorted(&mut h, "is:archived"), vec![4]);
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
pub fn mute_hides_thread_and_unsubscribe_hides_sender(cx: &mut TestAppContext) {
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
    assert!(h.read(|a| a.dialog_open()), "unsubscribe asks about the sender's mail");
    h.keys("4");
    assert_eq!(h.read(|a| a.mailbox.unsubscribed().to_vec()), vec!["c@a.io".to_string()]);
    assert_eq!(h.read(|a| a.mailbox.hidden_count()), 2, "all of the sender's mail is hidden");
    assert_eq!(h.visible(), vec![1, 2, 5]);
    assert_eq!(h.total(), 5, "nothing is deleted");
    h.assert_invariant("unsubscribe");
    h.keys("u");
    assert!(h.read(|a| a.mailbox.unsubscribed().is_empty()));
    assert_eq!(h.visible().len(), 5);
}
