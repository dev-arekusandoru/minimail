//! Headless UI tests for the list strip: opening views from the sidebar, the preview and
//! pin rules, per-tab state, cycling and closing on the focused pane, what the reader's
//! preview follows, and a tab dragged in the strip. Mailboxes are built inline.

use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext, ElementId, InputEvent as _, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Point, TestAppContext,
    point, px,
};
use mail_classifier::model::{Location, Mailbox, MessageId, TriageState};

#[path = "ui_ext/harness.rs"]
#[allow(dead_code)]
mod harness;
use harness::{Harness, harness_with, json, msg};

/// Thread 1: messages 1, 2 ("Plans"), both archived; thread 2: message 3 ("Lunch");
/// thread 3: message 4 ("Budget"). Inbox order, newest first: 4, 3; Archive: 2, 1.
fn mailbox() -> Mailbox {
    Mailbox::from_json(&json(&[
        msg(1, 1, "alice@example.com", "Plans", 1, "Archived"),
        msg(2, 1, "bob@example.com", "Re: Plans", 2, "Archived"),
        msg(3, 2, "cy@example.com", "Lunch", 3, "Inbox"),
        msg(4, 3, "di@example.com", "Budget", 4, "Inbox"),
    ]))
    .expect("valid mailbox json")
}

const INBOX: &str = "in:inbox";
const ARCHIVE: &str = "in:archived account:personal";
const TRASH: &str = "in:deleted account:personal";
const RECEIPTS: &str = "in:receipts account:personal";
const FAMILY: &str = "in:family account:personal";
const TRAVEL: &str = "in:travel account:personal";

/// A point on the list pane's own background (the list group's content, below its strip):
/// the sidebar takes 190px and the list ~42% of what is left, so the list runs to ~x=614 in
/// the 1200x800 harness window.
const ON_THE_LIST: (f32, f32) = (350., 300.);
/// A point inside the reader pane.
const IN_THE_READER: (f32, f32) = (900., 400.);

impl Harness<'_> {
    /// The strip as the model holds it: `(query, pinned)` per tab, left to right.
    fn tabs(&mut self) -> Vec<(String, bool)> {
        self.read(|a| {
            a.list_tabs
                .tabs()
                .iter()
                .map(|tab| (tab.query.describe(), tab.pinned))
                .collect()
        })
    }
    /// Which tab is showing, as its slot.
    fn active(&mut self) -> usize {
        self.read(|a| a.list_tabs.active_index())
    }
    /// What the tab in slot `ix` keeps: cursor index, selection, scroll row.
    fn state(&mut self, ix: usize) -> (usize, Vec<MessageId>, usize) {
        self.read(|a| a.tab_state(ix))
    }
    fn opened(&mut self) -> Option<MessageId> {
        self.read(|a| a.opened())
    }
    fn row(&mut self, id: MessageId) {
        self.click(("row", id as usize));
    }
    /// Click the sidebar row at `id` with the command key held, the way the app pins a view.
    fn cmd_click(&mut self, id: impl Into<ElementId>) {
        let id = id.into();
        let bounds = self
            .cx
            .update_window(self.window, |_, window, _| window.find(id).bounds())
            .expect("window alive");
        self.cx.run_until_parked();
        let at = point(
            px(f32::from(bounds.origin.x) + f32::from(bounds.size.width) / 2.),
            px(f32::from(bounds.origin.y) + f32::from(bounds.size.height) / 2.),
        );
        let window = self.window;
        let modifiers = Modifiers { platform: true, ..Default::default() };
        self.cx
            .update_window(window, |_, window, cx| {
                window.dispatch_event(
                    MouseDownEvent {
                        button: MouseButton::Left,
                        position: at,
                        modifiers,
                        click_count: 1,
                        first_mouse: false,
                    }
                    .to_platform_input(),
                    cx,
                );
                window.dispatch_event(
                    MouseUpEvent { button: MouseButton::Left, position: at, modifiers, click_count: 1 }
                        .to_platform_input(),
                    cx,
                );
            })
            .expect("window alive");
        self.cx.run_until_parked();
    }
    /// Drag tab slot `from` onto slot `to` in the list strip, the way the kit's dock test does.
    fn drag_tab(&mut self, from: usize, to: usize) {
        self.cx
            .update_window(self.window, |_, window, cx| window.within("tab-bar").drag_to(from, to, cx))
            .expect("window alive");
        self.cx.run_until_parked();
        // The drag edits the dock's own tree; the app reads it back on its next frame.
        self.cx.update_window(self.window, |_, window, cx| window.draw(cx)).ok();
        self.cx.run_until_parked();
    }

    /// Centre of the element `id`, as a point to start or end a gesture on.
    fn centre(&mut self, id: impl Into<ElementId>) -> Point<gpui_kit::Pixels> {
        let id = id.into();
        let bounds = self
            .cx
            .update_window(self.window, |_, window, _| window.find(id).bounds())
            .expect("window alive");
        self.cx.run_until_parked();
        point(
            px(f32::from(bounds.origin.x) + f32::from(bounds.size.width) / 2.),
            px(f32::from(bounds.origin.y) + f32::from(bounds.size.height) / 2.),
        )
    }

    /// Press on the element `from` and release at `to`, the way a drag is performed. The drop
    /// is a dock edit the app reads back on its next frame, so draw once after it.
    fn drag(&mut self, from: impl Into<ElementId>, to: Point<gpui_kit::Pixels>) {
        let from = self.centre(from);
        self.cx
            .update_window(self.window, |_, window, cx| window.drag(from, to, cx))
            .expect("window alive");
        self.cx.run_until_parked();
        self.cx.update_window(self.window, |_, window, cx| window.draw(cx)).ok();
        self.cx.run_until_parked();
    }

    /// Drag the sidebar row `from` onto the list pane's background.
    fn drag_to_list(&mut self, from: impl Into<ElementId>) {
        self.drag(from, point(px(ON_THE_LIST.0), px(ON_THE_LIST.1)));
    }
}

#[gpui_kit::gpui::test]
fn a_sidebar_click_opens_a_preview_tab_and_the_next_replaces_it(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox());
    assert_eq!(h.tabs(), vec![(INBOX.to_owned(), true)], "a window opens on All Inboxes");

    h.click("sidebar-archive-personal"); // Personal · Archive
    assert_eq!(
        h.tabs(),
        vec![(INBOX.to_owned(), true), (ARCHIVE.to_owned(), false)],
        "the second view opens as the preview"
    );
    assert_eq!(h.active(), 1);

    h.click("sidebar-trash-personal"); // Personal · Trash
    assert_eq!(
        h.tabs(),
        vec![(INBOX.to_owned(), true), (TRASH.to_owned(), false)],
        "the preview was replaced in place, not added to"
    );
    assert_eq!(h.active(), 1);
    assert_eq!(h.read(|a| a.location()), Some(Location::Trash("personal".into())));
}

#[gpui_kit::gpui::test]
fn a_cmd_click_pins_the_view_it_opens(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox());
    h.cmd_click("sidebar-archive-personal");
    assert_eq!(
        h.tabs(),
        vec![(INBOX.to_owned(), true), (ARCHIVE.to_owned(), true)],
        "a cmd-click pins the tab it opened"
    );

    // A plain click after it now appends rather than replacing.
    h.click("sidebar-trash-personal");
    assert_eq!(h.tabs().len(), 3, "the pinned tab is kept, the preview takes the new view");
    assert_eq!(h.tabs()[2], (TRASH.to_owned(), false));
}

#[gpui_kit::gpui::test]
fn clicking_a_view_that_is_already_pinned_activates_its_tab(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox());
    h.cmd_click("sidebar-archive-personal"); // pin Archive
    h.click("sidebar-trash-personal"); // the preview takes Trash
    assert_eq!(h.active(), 2);

    h.click("sidebar-archive-personal");
    assert_eq!(h.tabs().len(), 3, "no second tab for a view that is already open");
    assert_eq!(h.active(), 1, "its own tab was activated instead");
    assert_eq!(h.read(|a| a.location()), Some(Location::Archive("personal".into())));
}

#[gpui_kit::gpui::test]
fn every_tab_keeps_its_own_cursor_selection_and_scroll(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox());
    h.cmd_click("sidebar-archive-personal"); // pin Archive
    h.click("sidebar-trash-personal"); // the preview takes Trash, which is empty
    assert_eq!(h.state(2), (0, Vec::new(), 0), "an empty view starts at the top");

    // Give the Archive tab a cursor and a selection of its own.
    h.click("sidebar-archive-personal");
    assert_eq!(h.active(), 1);
    h.keys("x j x");
    let archive = h.state(1);
    assert_eq!(h.cursor(), Some(1));
    assert!(!archive.1.is_empty(), "the Archive tab holds a selection of its own");

    // Onto the Trash tab, and back: nothing of the Archive tab moved.
    h.click("sidebar-trash-personal");
    assert_eq!(h.active(), 2);
    assert_eq!(h.state(2), (0, Vec::new(), 0), "the other tab's state is untouched");
    h.click("sidebar-archive-personal");
    assert_eq!(h.state(1), archive, "the Archive tab kept its cursor, selection and scroll");
}

#[gpui_kit::gpui::test]
fn ctrl_tab_cycles_the_focused_pane_and_cmd_w_closes_its_tab(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox());
    h.cmd_click("sidebar-archive-personal"); // pin Archive
    h.click("sidebar-trash-personal"); // the preview takes Trash
    assert_eq!(h.tabs().len(), 3);

    // The list was the last pane touched, so the keys are the list's.
    h.keys("ctrl-tab");
    assert_eq!(h.active(), 0, "the list strip cycled");
    h.keys("ctrl-tab");
    assert_eq!(h.active(), 1);
    h.keys("cmd-w");
    assert_eq!(
        h.tabs(),
        vec![(INBOX.to_owned(), true), (TRASH.to_owned(), false)],
        "cmd-w closed the focused list tab"
    );
    h.keys("cmd-w");
    assert_eq!(h.tabs(), vec![(INBOX.to_owned(), true)], "the last tab cannot be closed");

    // With the reader focused the same keys are the reader's.
    h.row(4);
    h.keys("enter"); // landing on the message the preview already shows pins its tab
    h.row(3);
    h.keys("enter");
    let other = h.read(|a| a.tabs.tabs()[1].msg);
    h.keys("ctrl-tab");
    h.keys("cmd-w");
    assert_eq!(h.read(|a| a.tabs.tabs().len()), 1, "cmd-w closed the reader's tab");
    assert_eq!(h.opened(), Some(other), "and the neighbour took over");
    assert_eq!(h.tabs().len(), 1, "the list strip was left alone");
}

#[gpui_kit::gpui::test]
fn the_readers_preview_follows_the_active_list_tab(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox());
    h.cmd_click("sidebar-archive-personal"); // pin Archive
    h.click(("list-tab", 0usize));
    h.row(4);
    h.keys("enter");
    h.keys("enter"); // landing on the message the preview already shows pins its tab
    assert_eq!(h.opened(), Some(4));

    // Onto the pinned Archive tab: the reader's preview follows that tab's cursor.
    h.click(("list-tab", 1usize));
    assert_eq!(h.active(), 1);
    assert_eq!(h.opened(), Some(2), "the preview shows the tab's cursor message");
    assert_eq!(
        h.read(|a| a.tabs.tabs().iter().map(|tab| tab.msg).collect::<Vec<_>>()),
        vec![4, 2],
        "the pinned reader tab is still open, and the preview took the cursor's thread"
    );
}

#[gpui_kit::gpui::test]
fn dragging_a_tab_in_the_list_strip_reorders_the_model(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox());
    h.cmd_click("sidebar-archive-personal"); // pin Archive
    h.cmd_click("sidebar-trash-personal"); // pin Trash
    assert_eq!(h.tabs().len(), 3);

    h.drag_tab(0, 2);
    assert_eq!(
        h.tabs(),
        vec![
            (ARCHIVE.to_owned(), true),
            (TRASH.to_owned(), true),
            (INBOX.to_owned(), true),
        ],
        "the model followed the strip"
    );
}

#[gpui_kit::gpui::test]
fn a_click_in_the_reader_content_gives_it_the_tab_keys(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox());
    h.cmd_click("sidebar-archive-personal"); // pin Archive, so the list strip has tabs to cycle
    h.click("sidebar-trash-personal"); // the preview takes Trash
    h.click(("list-tab", 0usize)); // back to the inbox, where the rows are

    // Two reader tabs to cycle between and close.
    h.row(4);
    h.keys("enter"); // landing on the previewed message pins its tab
    h.row(3);
    h.keys("enter");
    assert_eq!(h.read(|a| a.tabs.tabs().len()), 2, "the reader has two tabs");

    // A list row click leaves the keys on the list.
    h.row(4);
    h.keys("ctrl-tab");
    assert_eq!(h.active(), 1, "the list strip cycled");
    h.keys("cmd-w");
    assert_eq!(h.tabs().len(), 2, "cmd-w closed a list tab");
    assert_eq!(h.read(|a| a.tabs.tabs().len()), 2, "the reader strip was left alone");

    // A click in the reader's content is what moves them to the reader — no tab or
    // row, just the message the reader is showing.
    h.click("reader-body");
    let before = h.opened();
    h.keys("ctrl-tab");
    assert_ne!(h.opened(), before, "the reader strip cycled");
    h.keys("cmd-w");
    assert_eq!(h.read(|a| a.tabs.tabs().len()), 1, "cmd-w closed a reader tab");
    assert_eq!(h.tabs().len(), 2, "the list strip was left alone");
}

#[gpui_kit::gpui::test]
fn dragging_a_folder_onto_the_list_opens_a_pinned_tab_and_activates_it_again(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox());
    h.drag_to_list("sidebar-folder-1"); // Receipts, a personal folder
    assert_eq!(
        h.tabs(),
        vec![(INBOX.to_owned(), true), (RECEIPTS.to_owned(), true)],
        "the folder opened as a new pinned tab"
    );
    assert_eq!(h.active(), 1, "and its tab is showing");
    assert_eq!(h.read(|a| a.location()), Some(Location::Folder(1)));

    // Its view is open now, so a second drop activates that tab instead of opening another.
    h.drag_to_list("sidebar-folder-1");
    assert_eq!(h.tabs().len(), 2, "no second tab for a view that is already open");
    assert_eq!(h.active(), 1, "its own tab was activated instead");

    // A folder dropped on the reader is not the list's: nothing happens.
    h.drag("sidebar-folder-2", point(px(IN_THE_READER.0), px(IN_THE_READER.1)));
    assert_eq!(h.tabs().len(), 2, "a drop on the reader does not open a list tab");
    assert!(!h.tabs().iter().any(|(query, _)| query == TRAVEL), "no tab for the reader drop");
}

#[gpui_kit::gpui::test]
fn dropping_a_folder_on_a_list_tab_opens_it_as_a_pinned_tab(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox());
    h.drag_to_list("sidebar-folder-1"); // Receipts, so the strip has a second tab to aim at
    assert_eq!(h.tabs().len(), 2);

    // Dropped on the first tab's title — our own element — rather than the pane behind it.
    let onto = h.centre(("list-tab", 0usize));
    h.drag("sidebar-folder-3", onto); // Family
    assert_eq!(
        h.tabs(),
        vec![
            (INBOX.to_owned(), true),
            (RECEIPTS.to_owned(), true),
            (FAMILY.to_owned(), true),
        ],
        "the folder dropped on a tab title opened as a pinned tab"
    );
    assert_eq!(h.active(), 2);
}
impl Harness<'_> {
    /// Drag the list row of `id` onto the title of tab slot `ix`.
    fn drag_row_to_tab(&mut self, id: MessageId, ix: usize) {
        let onto = self.centre(("list-tab", ix));
        self.drag(("row", id as usize), onto);
    }
    /// Drag the list row of `id` onto the list pane's background.
    fn drag_row_to_list(&mut self, id: MessageId) {
        self.drag(("row", id as usize), point(px(ON_THE_LIST.0), px(ON_THE_LIST.1)));
    }
    /// Undo one step straight on the mailbox; false when there was nothing to undo.
    fn undo_step(&mut self) -> bool {
        self.app.update(self.cx, |a, _| a.mailbox.undo())
    }
    /// Pin the given sidebar rows as tabs left to right after the Inbox, then show the Inbox.
    fn pin_then_inbox(&mut self, rows: &[&'static str]) {
        for row in rows {
            self.cmd_click(*row);
        }
        self.click(("list-tab", 0usize));
        assert_eq!(self.active(), 0);
    }
}

/// A harness whose setup undo steps (none of which touch a message's state) are dropped, so an
/// empty undo stack means a drop pushed nothing.
fn fresh(cx: &mut TestAppContext) -> Harness<'_> {
    let mut h = harness_with(cx, mailbox());
    while h.undo_step() {}
    h
}

#[gpui_kit::gpui::test]
fn dropping_a_message_on_an_archive_tab_archives_it_in_one_undo_step(cx: &mut TestAppContext) {
    let mut h = fresh(cx);
    h.pin_then_inbox(&["sidebar-archive-personal"]);
    h.drag_row_to_tab(4, 1);
    assert_eq!(h.state_of(4), TriageState::Archived);
    assert_eq!(h.state_of(3), TriageState::Inbox, "only the dragged message moved");
    assert!(h.toast().contains("Marked 1 archived"), "toast: {}", h.toast());
    assert_eq!(h.active(), 0, "the drop does not switch tabs");

    assert!(h.undo_step());
    assert_eq!(h.state_of(4), TriageState::Inbox, "one undo restores it");
    assert!(!h.undo_step(), "and it was the only step");
}

#[gpui_kit::gpui::test]
fn dropping_a_message_on_a_trash_tab_deletes_it(cx: &mut TestAppContext) {
    let mut h = fresh(cx);
    h.pin_then_inbox(&["sidebar-trash-personal"]);
    h.drag_row_to_tab(3, 1);
    assert_eq!(h.state_of(3), TriageState::Deleted);
}

#[gpui_kit::gpui::test]
fn dropping_a_message_on_a_folder_tab_files_it_there(cx: &mut TestAppContext) {
    let mut h = fresh(cx);
    h.pin_then_inbox(&["sidebar-folder-1"]); // Receipts
    h.drag_row_to_tab(4, 1);
    assert_eq!(h.state_of(4), TriageState::Filed(1));
    assert!(h.undo_step());
    assert_eq!(h.state_of(4), TriageState::Inbox);
}

#[gpui_kit::gpui::test]
fn dropping_a_message_on_the_inbox_tab_moves_an_archived_one_back(cx: &mut TestAppContext) {
    let mut h = fresh(cx);
    h.cmd_click("sidebar-inbox-personal");
    h.cmd_click("sidebar-archive-personal");
    assert_eq!(h.active(), 2);
    h.drag_row_to_tab(2, 1);
    assert_eq!(h.state_of(2), TriageState::Inbox);
    assert!(h.toast().contains("Moved 1 to inbox"), "toast: {}", h.toast());
}

#[gpui_kit::gpui::test]
fn dropping_on_the_list_background_files_under_the_active_tab(cx: &mut TestAppContext) {
    let mut h = fresh(cx);
    h.cmd_click("sidebar-archive-personal");
    assert_eq!(h.active(), 1);
    // Search the Archive tab for inbox mail, so a message that is not archived is on screen.
    h.keys("/");
    h.type_text("in:inbox");
    h.keys("enter");
    assert_eq!(h.state_of(4), TriageState::Inbox);
    assert!(h.visible().contains(&4), "the inbox message is on screen: {:?}", h.visible());
    h.drag_row_to_list(4);
    assert_eq!(h.state_of(4), TriageState::Archived, "filed under the active (Archive) tab");
    assert!(h.undo_step());
    assert_eq!(h.state_of(4), TriageState::Inbox);
}

#[gpui_kit::gpui::test]
fn a_drop_that_moves_nothing_leaves_no_undo_step_and_no_toast(cx: &mut TestAppContext) {
    let mut h = fresh(cx);
    // Message 4 already is in the inbox: dropped on the Inbox tab and on the Inbox background.
    h.pin_then_inbox(&["sidebar-inbox-personal"]);
    h.click(("list-tab", 1usize));
    h.drag_row_to_tab(4, 1);
    h.drag_row_to_list(4);
    assert_eq!(h.state_of(4), TriageState::Inbox);
    assert_eq!(h.toast(), "");
    assert!(!h.undo_step(), "no undo step was pushed");
}

#[gpui_kit::gpui::test]
fn dropping_on_a_sent_or_all_inboxes_tab_changes_nothing(cx: &mut TestAppContext) {
    let mut h = fresh(cx);
    h.pin_then_inbox(&["sidebar-sent-personal"]);
    h.drag_row_to_tab(4, 1); // Sent
    h.drag_row_to_tab(4, 0); // All Inboxes, the first tab
    assert_eq!(h.state_of(4), TriageState::Inbox);
    assert_eq!(h.toast(), "");
    assert!(!h.undo_step(), "no undo step was pushed");
}

#[gpui_kit::gpui::test]
fn dropping_a_message_on_a_snoozed_tab_opens_the_snooze_picker_for_it(cx: &mut TestAppContext) {
    let mut h = fresh(cx);
    h.pin_then_inbox(&["sidebar-snoozed-personal"]);
    h.drag_row_to_tab(4, 1);
    assert!(h.read(|a| a.snooze_open()), "the picker opened");
    assert_eq!(h.state_of(4), TriageState::Inbox, "nothing is snoozed until a time is picked");
    assert!(!h.undo_step());

    h.keys("1");
    assert_eq!(h.state_of(4), TriageState::Snoozed);
    assert_eq!(h.state_of(3), TriageState::Inbox, "the dragged message alone");
    assert!(h.undo_step());
    assert_eq!(h.state_of(4), TriageState::Inbox);
}

#[gpui_kit::gpui::test]
fn dragging_a_selected_row_moves_the_whole_selection_in_one_undo_step(cx: &mut TestAppContext) {
    let mut h = fresh(cx);
    h.pin_then_inbox(&["sidebar-archive-personal"]);
    h.keys("x j x");
    assert_eq!(h.read(|a| a.triage().selected()).len(), 2);
    h.drag_row_to_tab(4, 1);
    assert_eq!(h.state_of(3), TriageState::Archived);
    assert_eq!(h.state_of(4), TriageState::Archived);
    assert!(h.read(|a| a.triage().selected()).is_empty(), "the selection was cleared");
    assert!(h.undo_step());
    assert_eq!(h.state_of(3), TriageState::Inbox);
    assert_eq!(h.state_of(4), TriageState::Inbox);
    assert!(!h.undo_step(), "both moved in a single step");
}
