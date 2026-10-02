//! Drag & drop: dragging a list row onto the reader opens it, and a drop anywhere else
//! leaves the reader alone. Mailboxes are built inline, like the other UI suites.

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, ElementId, Point, TestAppContext, point, px};
use mail_classifier::model::{Mailbox, MessageId};

#[path = "ui_ext/harness.rs"]
#[allow(dead_code)]
mod harness;
use harness::{Harness, harness_with, json, msg};

/// A point inside the reader pane of the harness window (1200x800): the sidebar takes 190px and
/// the list ~42% of what is left, so the reader starts around x=614.
const IN_THE_READER: (f32, f32) = (900., 400.);

/// Thread 1: messages 1, 2 ("Plans"); thread 2: message 3 ("Lunch"); thread 3: message 4
/// ("Budget"). Inbox order, newest first: 4, 3, 2, 1.
fn mailbox() -> Mailbox {
    Mailbox::from_json(&json(&[
        msg(1, 1, "alice@example.com", "Plans", 1, "Inbox"),
        msg(2, 1, "bob@example.com", "Re: Plans", 2, "Inbox"),
        msg(3, 2, "cy@example.com", "Lunch", 3, "Inbox"),
        msg(4, 3, "di@example.com", "Budget", 4, "Inbox"),
    ]))
    .expect("valid mailbox json")
}

impl Harness<'_> {
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

    /// Press `from` and release at `to`, the way a drag is performed.
    fn drag(&mut self, from: impl Into<ElementId>, to: Point<gpui_kit::Pixels>) {
        let from = self.centre(from);
        self.cx
            .update_window(self.window, |_, window, cx| window.drag(from, to, cx))
            .expect("window alive");
        self.cx.run_until_parked();
    }

    /// Drag the row of `id` onto the reader pane.
    fn drag_row_to_reader(&mut self, id: MessageId) {
        self.drag(("row", id as usize), point(px(IN_THE_READER.0), px(IN_THE_READER.1)));
    }

    /// Double-click the tab of `thread`, which pins it.
    fn pin_tab(&mut self, thread: u32) {
        let id = ("reader-tab", thread as usize);
        self.cx.update_window(self.window, |_, window, cx| window.double_click(id, cx)).expect("window alive");
        self.cx.run_until_parked();
    }

    /// Drag tab slot `from` onto slot `to`, the way the kit's own dock test moves a tab.
    fn drag_tab(&mut self, from: usize, to: usize) {
        self.cx
            .update_window(self.window, |_, window, cx| window.within("tab-bar").drag_to(from, to, cx))
            .expect("window alive");
        self.cx.run_until_parked();
    }

    fn tabs(&mut self) -> Vec<(u32, bool)> {
        self.read(|a| a.tabs.tabs().iter().map(|t| (t.thread, t.pinned)).collect())
    }
    fn opened(&mut self) -> Option<MessageId> {
        self.read(|a| a.opened())
    }
}

#[gpui_kit::gpui::test]
fn dragging_a_row_onto_the_reader_opens_it(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox());
    h.drag_row_to_reader(3);
    assert_eq!(h.tabs(), vec![(2, false)], "opened as the preview tab");
    assert_eq!(h.opened(), Some(3));
}

#[gpui_kit::gpui::test]
fn dropping_another_row_replaces_the_preview(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox());
    h.drag_row_to_reader(3);
    h.drag_row_to_reader(4);
    assert_eq!(h.tabs(), vec![(3, false)], "the preview was replaced, not added to");
    assert_eq!(h.opened(), Some(4));
}

#[gpui_kit::gpui::test]
fn dropping_a_row_on_the_list_leaves_the_reader_alone(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox());
    // The list is a tab group too; a drop there is not a reader drop.
    let target = h.centre(("row", 4usize));
    h.drag(("row", 3usize), target);
    assert!(h.tabs().is_empty(), "nothing was opened");
    assert_eq!(h.opened(), None);
}

#[gpui_kit::gpui::test]
fn a_plain_click_still_opens_instead_of_dragging(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox());
    h.click(("row", 3usize));
    assert_eq!(h.opened(), Some(3), "the click was not swallowed by the drag listener");
}





#[gpui_kit::gpui::test]
fn dragging_a_tab_in_the_strip_reorders_the_model(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox());
    for (id, thread) in [(1, 1), (3, 2)] {
        h.drag_row_to_reader(id);
        h.pin_tab(thread);
    }
    h.drag_row_to_reader(4);
    assert_eq!(h.tabs(), vec![(1, true), (2, true), (3, false)]);

    h.drag_tab(0, 2);
    assert_eq!(h.tabs(), vec![(2, true), (3, false), (1, true)], "the model followed the strip");

    // The tab the drag left open is the one `cmd-w` closes, wherever it now sits.
    h.keys("cmd-w");
    assert_eq!(h.tabs(), vec![(2, true), (3, false)]);
}

