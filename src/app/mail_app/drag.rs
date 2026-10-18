//! Drag sources: the chip that follows the pointer while a list row is dragged, and the
//! `on_drag` wiring every row shares.

use super::*;
use crate::dnd::{DropAction, FolderDrag, MailDrag, drop_action};
use gpui_kit::component::dock::{AnyDrag as DockDrag, DropTarget};
use gpui_kit::component::ActiveTheme as _;

/// Width of the chip.
const CHIP_W: f32 = 260.;

/// The chip that follows the pointer while a row is dragged: the row's sender and subject, and
/// how many messages the drop would move.
pub(super) struct MailDragPreview {
    sender: SharedString,
    subject: SharedString,
    count: usize,
}

impl MailDragPreview {
    pub(super) fn new(sender: SharedString, subject: SharedString, count: usize) -> Self {
        Self { sender, subject, count }
    }
}

impl Render for MailDragPreview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        div()
            .id("mail-drag-chip")
            .test_support()
            .w(px(CHIP_W))
            .px_2()
            .py_1()
            .rounded_md()
            .border_1()
            .border_color(t.border)
            .bg(t.background)
            .opacity(0.95)
            .flex()
            .items_center()
            .gap_2()
            .child(div().flex_none().text_sm().font_weight(FontWeight::SEMIBOLD).child(self.sender.clone()))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .text_color(t.muted_foreground)
                    .child(self.subject.clone()),
            )
            .when(self.count > 1, |d| {
                d.child(
                    div()
                        .flex_none()
                        .text_xs()
                        .text_color(t.muted_foreground)
                        .child(format!("+{}", self.count - 1)),
                )
            })
    }
}

/// The chip that follows the pointer while a sidebar folder row is dragged: the row's label.
pub(super) struct FolderDragPreview {
    label: SharedString,
}

impl FolderDragPreview {
    pub(super) fn new(label: SharedString) -> Self {
        Self { label }
    }
}

impl Render for FolderDragPreview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        div()
            .id("folder-drag-chip")
            .test_support()
            .px_2()
            .py_1()
            .rounded_md()
            .border_1()
            .border_color(t.border)
            .bg(t.background)
            .opacity(0.95)
            .text_sm()
            .child(self.label.clone())
    }
}

impl MailApp {
    /// `row` with a drag attached. `ids` are the messages the row stands for, `label` names it on
    /// the chip. A row that stands for nothing stays undraggable.
    pub(super) fn draggable(
        &self,
        row: Stateful<Div>,
        ids: &[MessageId],
        thread: Option<u32>,
        label: (SharedString, SharedString),
    ) -> Stateful<Div> {
        let Some(drag) = MailDrag::for_row(&self.triage().selected(), ids, thread) else {
            return row;
        };
        let (sender, subject) = label;
        let count = drag.count();
        row.on_drag(DockDrag::new(drag), move |_drag: &DockDrag, _offset, _window, cx| {
            cx.new(|_| MailDragPreview::new(sender.clone(), subject.clone(), count))
        })
    }

    /// A drag dropped on the reader: open the message the grabbed row carried. A drop on the
    /// list group, or with alt held, does something else — the latter pins the tab instead of
    /// opening it as the preview, the way `enter` does on a click.
    pub(super) fn drop_on_reader(
        &mut self,
        item: &DockDrag,
        target: &DropTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(drag) = item.value().downcast_ref::<MailDrag>() else {
            return;
        };
        if !self.panes.dock_ref().is_reader_node(target.node(), cx) {
            return;
        }
        let (anchor, pin) = (drag.anchor, window.modifiers().alt);
        if pin {
            self.pin_message(anchor);
        } else {
            self.open_message(anchor, false);
        }
        cx.notify();
    }

    /// A folder dropped on the list strip opens its view as a pinned tab; messages dropped on
    /// the list group are filed under the active tab. A drop anywhere else — the reader strip
    /// included — does nothing.
    pub(super) fn drop_on_list(
        &mut self,
        item: &DockDrag,
        target: &DropTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.panes.dock_ref().is_list_node(target.node(), cx) {
            return;
        }
        if let Some(drag) = item.value().downcast_ref::<FolderDrag>() {
            self.show_location_in_panel(drag.location.clone(), true, cx);
        } else if let Some(drag) = item.value().downcast_ref::<MailDrag>() {
            let location = self.list_views[self.list_tabs.active_index()].location().clone();
            self.drop_messages(&drag.ids, &location, window, cx);
        }
    }

    /// Messages dropped on a tab showing `location` (or on the pane, for the active tab): file
    /// them there, open the snooze picker, or do nothing, per [`drop_action`]. A drop that
    /// would move nothing leaves no undo step and no toast.
    pub(super) fn drop_messages(
        &mut self,
        ids: &[MessageId],
        location: &Location,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match drop_action(location) {
            DropAction::Nothing => {}
            DropAction::File(state) => {
                if ids.iter().any(|id| self.mailbox.state_of(*id).is_some_and(|s| s != state)) {
                    self.file_ids(ids, state, window, cx);
                }
            }
            DropAction::Snooze => {
                if self.modal_open() {
                    return;
                }
                self.open_snooze_for(Some(ids.to_vec()), window, cx);
            }
        }
    }
}
