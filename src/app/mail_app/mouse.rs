//! Mouse entry points for row clicks, left-edge selection, suggestion badges and
//! the contextual action bar.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{BAR_H, HEADER_H, LIST_HEADER_H, ListMode, MENU_W, MailApp, MenuKind, PaneLayout};
use crate::model::Location;
use crate::app::actions::*;
use crate::app::ui::{button, run};

/// Mouse-down listener for a modal backdrop: clicking outside the panel closes the modal like
/// `escape` does.
pub(super) fn close_on_backdrop(
    cx: &Context<MailApp>,
) -> impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static {
    cx.listener(|this: &mut MailApp, _: &MouseDownEvent, window, cx| this.close_modals(window, cx))
}

impl MailApp {
    /// Move the cursor to visible row `ix` (either list mode), clamped.
    pub(super) fn cursor_to(&mut self, ix: usize) {
        let delta = ix as isize - self.cursor_ix() as isize;
        self.move_cursor(delta);
    }

    /// Click on row `ix`: plain selects it and opens it, shift extends the selection from the
    /// cursor (like `shift-j`), cmd toggles it (like `x`). Double-click opens like `enter`.
    pub(super) fn click_row(
        &mut self,
        ix: usize,
        event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mods = event.modifiers();
        let selectable = self.mode == ListMode::State && !self.in_session();
        if selectable && mods.shift {
            let delta = ix as isize - self.cursor_ix() as isize;
            self.extend_by(delta);
        } else if selectable && mods.platform {
            self.cursor_to(ix);
            self.toggle_select_cursor();
        } else {
            self.triage.clear_selection();
            self.cursor_to(ix);
            if let Some(id) = self.cursor_id() {
                self.opened = Some(id);
            }
        }
        self.scroll_to_cursor();
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    /// Click on the row's left edge: like moving there and pressing `x`.
    pub(super) fn toggle_row(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.cursor_to(ix);
        if self.mode == ListMode::State && !self.in_session() {
            self.toggle_select_cursor();
        }
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    /// `y`: accept the AI suggestions pending on the message under the cursor.
    pub(super) fn accept_suggestions(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.cursor_id() {
            let now = self.now();
            self.mailbox.accept_suggestions(id, now);
            cx.notify();
        }
    }

    /// `n`: reject the AI suggestions pending on the message under the cursor.
    pub(super) fn reject_suggestions(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.cursor_id() {
            self.mailbox.reject_suggestions(id);
            cx.notify();
        }
    }

    /// Click (`accept`) or right-click (reject) on the badges of row `ix`.
    pub(super) fn suggestions_at(
        &mut self,
        ix: usize,
        accept: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cursor_to(ix);
        if accept {
            self.accept_suggestions(cx);
        } else {
            self.reject_suggestions(cx);
        }
        window.focus(&self.focus_handle, cx);
    }

    /// Full-height selection hit target at the row's left edge.
    pub(super) fn row_selection_target(&self, ix: usize, cx: &Context<Self>) -> crate::app::ui::Observable {
        let t = crate::theme::active(cx);
        let hover = t.selected.opacity(0.2);
        div()
            .id(("row-select", ix))
            .absolute()
            .left_0()
            .top_0()
            .bottom_0()
            .w(px(10.))
            .cursor_pointer()
            .hover(move |d| d.bg(hover))
            .tooltip(|window, cx| {
                gpui_kit::component::tooltip::Tooltip::new(
                    "Select (click) · cmd-click / shift-click on row",
                )
                .build(window, cx)
            })
            .test_support()
            .on_click(cx.listener(move |this, _, window, cx| this.toggle_row(ix, window, cx)))
    }


    /// Actions for whatever the list targets: the core triage buttons and the selection
    /// count, then — only where they apply — the AI suggestions, Reply and the rest.
    pub(super) fn render_context_bar(&self, cx: &Context<Self>) -> AnyElement {
        let t = crate::theme::active(cx);
        let selected = self.triage.selected().len();
        let pending = self.pending_suggestions();
        let more_open = self.menu_is(MenuKind::Message);

        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .h(px(BAR_H))
            .px_3()
            .bg(t.sidebar)
            .border_b_1()
            .border_color(t.border)
            .child(button("btn-archive", "Archive", "Archive", "e", cx).on_click(run(Archive)))
            .child(button("btn-file", "File…", "File into a folder", "f", cx).on_click(run(File)))
            .child(button("btn-delete", "Delete", "Delete", "d", cx).on_click(run(Delete)))
            .when(self.triage.view.location != Location::AllInboxes, |d| {
                d.child(button("btn-inbox", "Inbox", "Move to inbox", "i", cx).on_click(run(MoveToInbox)))
            })
            .child(button("btn-snooze", "Snooze…", "Snooze", "s", cx).on_click(run(OpenSnoozePicker)))
            .when(selected > 0, |d| {
                d.child(
                    div()
                        .flex_none()
                        .text_size(px(11.))
                        .text_color(t.accent)
                        .child(format!("{selected} selected")),
                )
            })
            .child(div().flex_1())
            .when(pending > 0, |d| {
                d.child(
                    button("btn-accept", "Accept AI", "Accept the AI suggestions on this message", "y", cx)
                        .on_click(run(AcceptSuggestions)),
                )
                .child(
                    button("btn-reject", "Reject AI", "Reject the AI suggestions on this message", "n", cx)
                        .on_click(run(RejectSuggestions)),
                )
            })
            .when(self.opened.is_some(), |d| {
                d.child(button("btn-reply", "Reply", "Reply to the open message", "r", cx).on_click(run(Reply)))
            })
            .child(
                button("btn-message-more", "More ▾", "More actions for this message", "", cx)
                    .when(more_open, |b| b.bg(t.selection))
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_menu(MenuKind::Message, window, cx))),
            )
            .into_any_element()
    }

    /// The popup menu layer: a backdrop that dismisses on a click, plus the open panel
    /// tucked under the bar whose trigger opened it.
    pub(super) fn render_menu(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let panel = self.menu_panel()?;
        let filter = self.menu_is(MenuKind::Filter);
        let top = match filter {
            true => self.list_header_bottom(),
            false if self.menu_is(MenuKind::Global) => HEADER_H,
            false => HEADER_H + BAR_H,
        };
        // The filter menu hangs under the Filter ▾ button at the list header's right edge.
        let left = self.sidebar_w + self.list_w - MENU_W - 12.;
        Some(
            div()
                .id("menu-backdrop")
                .test_support()
                .absolute()
                .inset_0()
                .occlude()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| this.close_menu(window, cx)),
                )
                .child(
                    div()
                        .absolute()
                        .top(px(top))
                        .when(filter, |d| d.left(px(left.max(self.sidebar_w + 8.))))
                        .when(!filter, |d| d.right(px(12.)))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(panel),
                )
                .into_any_element(),
        )
    }

    /// Distance from the window top to the bottom edge of the list header.
    fn list_header_bottom(&self) -> f32 {
        HEADER_H + if self.context_actions() { BAR_H } else { 0. } + LIST_HEADER_H
    }

    /// Toolbar glyph for the pane layout button: what pressing it switches to.
    pub(super) fn layout_glyph(&self) -> &'static str {
        match self.panes.orientation() {
            PaneLayout::SideBySide => "▤",
            PaneLayout::Stacked => "▥",
        }
    }
}
