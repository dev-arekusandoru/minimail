//! The Triage tab: a session lives in a zoomed dock group of its own, and nothing the user
//! arranged — list tabs, their cursors, selections and scroll, reader tabs — is touched by it.

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, ElementId, TestAppContext};
use mail_classifier::model::{Mailbox, MessageId, TriageState};

#[path = "ui_ext/harness.rs"]
#[allow(dead_code)]
mod harness;
use harness::{Harness, harness_with, json, msg};

/// Inbox, newest first: 5, 4, 3, 2. Message 1 is archived.
fn mailbox() -> Mailbox {
    Mailbox::from_json(&json(&[
        msg(1, 1, "alice@example.com", "Plans", 1, "Archived"),
        msg(2, 2, "bob@example.com", "Re: Plans", 2, "Inbox"),
        msg(3, 3, "cy@example.com", "Lunch", 3, "Inbox"),
        msg(4, 4, "di@example.com", "Budget", 4, "Inbox"),
        msg(5, 5, "ed@example.com", "Offsite", 5, "Inbox"),
    ]))
    .expect("valid mailbox json")
}

/// Everything the user arranged, as the model holds it.
#[derive(Debug, PartialEq)]
struct Arranged {
    tabs: Vec<(String, bool)>,
    active: usize,
    states: Vec<(usize, Vec<MessageId>, usize)>,
    readers: Vec<(u32, bool)>,
    reader_open: Option<MessageId>,
}

impl Harness<'_> {
    fn arranged(&mut self) -> Arranged {
        self.read(|a| Arranged {
            tabs: a.list_tabs.tabs().iter().map(|t| (t.query.describe(), t.pinned)).collect(),
            active: a.list_tabs.active_index(),
            states: (0..a.list_tabs.len()).map(|ix| a.tab_state(ix)).collect(),
            readers: a.tabs.tabs().iter().map(|t| (t.thread, t.pinned)).collect(),
            reader_open: a.tabs.opened(),
        })
    }
    /// Draw a frame, so the dock has caught up with the app.
    fn frame(&mut self) {
        self.cx.run_until_parked();
        self.cx
            .update_window(self.window, |_, window, cx| window.render_frame(cx))
            .expect("window alive");
        self.cx.run_until_parked();
    }
    fn on_screen(&mut self, id: impl Into<ElementId>) -> bool {
        self.frame();
        self.has(id)
    }
    fn zoomed(&mut self) -> Option<&'static str> {
        self.frame();
        self.cx.read_entity(&self.app, |a, cx| a.zoomed_group(cx))
    }
    fn groups(&mut self) -> usize {
        self.frame();
        self.cx.read_entity(&self.app, |a, cx| a.dock_groups(cx))
    }
}

/// A user with something to lose: the cursor down a row, a message selected, a reader tab open.
fn arranged_user(cx: &mut TestAppContext) -> Harness<'_> {
    let mut h = harness_with(cx, mailbox());
    h.keys("j x enter");
    h.frame();
    h
}

#[gpui_kit::gpui::test]
fn a_session_leaves_the_users_tabs_alone(cx: &mut TestAppContext) {
    let mut h = arranged_user(cx);
    let before = h.arranged();
    assert!(!before.states[0].1.is_empty(), "the setup selected a message");
    assert_eq!(before.readers.len(), 1, "the setup opened a reader tab");

    h.keys("t");
    assert_eq!(h.read(|a| a.session_progress()), Some((1, 4)));
    assert_eq!(h.arranged(), before, "starting a session changes nothing the user arranged");

    // Working through the queue moves the session's own cursor, never the user's.
    h.keys("j j");
    h.keys("escape");
    assert_eq!(h.read(|a| a.session_progress()), None);
    assert_eq!(h.arranged(), before, "ending it returns to exactly what they had");
}

#[gpui_kit::gpui::test]
fn the_session_runs_in_a_zoomed_group_beside_the_sidebar(cx: &mut TestAppContext) {
    let mut h = arranged_user(cx);
    assert_eq!(h.zoomed(), None);
    assert_eq!(h.groups(), 2);
    assert!(h.on_screen("sidebar-divider"), "the sidebar is on screen");
    assert!(h.has(("reader-tab", 4usize)), "the user's reader tab is drawn");

    h.keys("t");
    assert_eq!(h.groups(), 3, "the session adds a group of its own");
    assert_eq!(h.zoomed(), Some("triage"), "and fills the dock with it");
    assert!(h.on_screen("sidebar-divider"), "the sidebar stays on screen through the zoom");
    assert!(h.has(("row", 5usize)), "the session's list is drawn");
    assert!(!h.has(("reader-tab", 4usize)), "the user's reader group is out of the way");
    assert_eq!(h.read(|a| a.opened()), Some(5), "the session reader shows its message");

    h.keys("escape");
    assert_eq!(h.groups(), 2, "ending the session removes the group");
    assert_eq!(h.zoomed(), None, "and gives the dock back");
    assert!(h.on_screen("sidebar-divider"));
}

#[gpui_kit::gpui::test]
fn the_end_summary_goes_with_the_group(cx: &mut TestAppContext) {
    let mut h = arranged_user(cx);
    let before = h.arranged();
    h.keys("t e");
    h.keys("escape");
    assert!(h.read(|a| a.session_end()).is_some(), "the end card is up");
    assert_eq!(h.groups(), 3, "the Triage group stays while its summary does");
    assert_eq!(h.zoomed(), Some("triage"));

    h.keys("escape");
    assert_eq!(h.read(|a| a.session_end()), None);
    assert_eq!(h.groups(), 2);
    assert_eq!(h.zoomed(), None);
    // Archiving the cursor message moved the cursor, and the preview follows it.
    let after = h.arranged();
    assert_eq!((after.tabs, after.active), (before.tabs, before.active));
}

#[gpui_kit::gpui::test]
fn finishing_the_queue_returns_the_layout(cx: &mut TestAppContext) {
    let mut h = arranged_user(cx);
    let before = h.arranged();
    h.keys("t e e e e");
    assert_eq!(h.read(|a| a.session_end()).map(|(handled, _)| handled), Some(4));
    assert_eq!(h.zoomed(), Some("triage"));
    h.keys("escape");
    assert_eq!(h.zoomed(), None);
    assert_eq!(h.groups(), 2);
    // Archiving moved mail out of the inbox, which retires a preview tab as it always does,
    // so only the list strip is compared.
    let after = h.arranged();
    assert_eq!((after.tabs, after.active), (before.tabs, before.active));
}

#[gpui_kit::gpui::test]
fn alt_z_zooms_the_focused_group(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox());
    h.keys("j");
    assert_eq!(h.zoomed(), None);

    h.keys("alt-z");
    assert_eq!(h.zoomed(), Some("list"), "the list has the focus");
    assert!(h.on_screen("sidebar-divider"), "the sidebar stays");
    h.keys("alt-z");
    assert_eq!(h.zoomed(), None);

    h.keys("enter");
    h.keys("alt-z");
    assert_eq!(h.zoomed(), Some("reader"), "opening a message moved the focus to the reader");
    h.keys("alt-z");
    assert_eq!(h.zoomed(), None);
}

#[gpui_kit::gpui::test]
fn alt_z_toggles_the_triage_group_out_and_back(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox());
    h.keys("t");
    assert_eq!(h.zoomed(), Some("triage"));
    h.keys("alt-z");
    assert_eq!(h.zoomed(), None);
    assert_eq!(h.groups(), 3, "unzoomed, the group is still there");
    h.keys("alt-z");
    assert_eq!(h.zoomed(), Some("triage"));
}

#[gpui_kit::gpui::test]
fn a_zoom_survives_a_relayout(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox());
    h.keys("alt-z");
    assert_eq!(h.zoomed(), Some("list"));
    h.keys("alt-l");
    assert_eq!(h.zoomed(), Some("list"), "re-laying the centre keeps the group zoomed");
}

#[gpui_kit::gpui::test]
fn a_message_triaged_in_a_session_is_filed_and_undone_once(cx: &mut TestAppContext) {
    let mut h = arranged_user(cx);
    h.keys("t");
    assert_eq!(h.read(|a| a.opened()), Some(5));
    h.keys("e");
    assert_eq!(h.state_of(5), TriageState::Archived);
    assert_eq!(h.read(|a| a.session_progress()), Some((2, 4)));
    h.keys("u");
    assert_eq!(h.state_of(5), TriageState::Inbox, "one undo restores it");
    h.assert_invariant("triage tab");
}
