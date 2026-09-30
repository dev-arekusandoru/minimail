//! Mouse entry points of the root view: row clicks, checkboxes, suggestion badges, the
//! quiet header and the contextual action bar. Buttons dispatch the same actions the keys
//! do; row clicks reuse the cursor and selection primitives behind `j`/`k`/`shift-j`/`x`.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{BAR_H, HEADER_H, ListMode, MailApp, MenuKind, PaneLayout};
use crate::model::TriageState;
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

    /// Click on the row checkbox: like moving there and pressing `x`.
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

    /// Selection checkbox for row `ix`. Sits beside (not inside) the clickable row body.
    pub(super) fn row_checkbox(&self, ix: usize, selected: bool, cx: &Context<Self>) -> crate::app::ui::Observable {
        let t = crate::theme::active(cx);
        div()
            .id(("row-check", ix))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .w(px(14.))
            .h(px(14.))
            .rounded_sm()
            .border_1()
            .border_color(if selected { t.accent } else { t.border })
            .when(selected, |d| d.bg(t.accent))
            .text_color(t.on_accent)
            .text_size(px(10.))
            .cursor_pointer()
            .tooltip(|window, cx| {
                gpui_kit::component::tooltip::Tooltip::new("Select (x)").build(window, cx)
            })
            .child(if selected { "✓" } else { "" })
            .test_support()
            .on_click(cx.listener(move |this, _, window, cx| this.toggle_row(ix, window, cx)))
    }

    /// The quiet header: search on the left, pane layout, Triage and the overflow menu
    /// on the right. One row, no wrapping.
    pub(super) fn render_header(&self, cx: &Context<Self>) -> AnyElement {
        let t = crate::theme::active(cx);
        let searching = matches!(self.mode, ListMode::Search(_));
        let search_text = self.search_header().unwrap_or_else(|| "Search…".into());
        let more_open = self.menu_is(MenuKind::Global);

        let search = div()
            .id("search-box")
            .test_support()
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .w(px(260.))
            .h(px(22.))
            .px_2()
            .rounded_sm()
            .border_1()
            .border_color(t.border)
            .bg(t.surface)
            .text_size(px(11.))
            .text_color(if searching { t.text } else { t.text_muted })
            .cursor_pointer()
            .child(search_text)
            .child(div().text_color(t.text_muted).child("/"))
            .on_click(run(OpenSearch));

        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .h(px(HEADER_H))
            .px_3()
            .bg(t.sidebar)
            .border_b_1()
            .border_color(t.border)
            .child(search)
            .when(searching, |d| {
                d.child(button("search-clear", "Clear", "Leave search", "escape", cx).on_click(run(ClearSelection)))
            })
            .child(div().flex_1())
            .child(
                button("btn-layout", self.layout_glyph(), "Stack the panes the other way", "alt-l", cx)
                    .on_click(run(TogglePaneLayout)),
            )
            .child(button("btn-session", "Triage", "Start a triage session", "t", cx).on_click(run(StartSession)))
            .child(
                button("btn-more", "More ▾", "More actions", "", cx)
                    .when(more_open, |b| b.bg(t.selection))
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_menu(MenuKind::Global, window, cx))),
            )
            .into_any_element()
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
            .child(button("btn-done", "Done", "Mark done", "e", cx).on_click(run(MarkDone)))
            .child(button("btn-waiting", "Waiting", "Mark waiting", "w", cx).on_click(run(MarkWaiting)))
            .when(self.triage.view != TriageState::Inbox, |d| {
                d.child(button("btn-inbox", "Inbox", "Move to inbox", "i", cx).on_click(run(MoveToInbox)))
            })
            .child(button("btn-later", "Later…", "Later, with a return time", "l", cx).on_click(run(OpenSnoozePicker)))
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
        let top = if self.menu_is(MenuKind::Global) {
            HEADER_H
        } else {
            HEADER_H + BAR_H
        };
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
                        .right(px(12.))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(panel),
                )
                .into_any_element(),
        )
    }

    /// Toolbar glyph for the pane layout button: what pressing it switches to.
    pub(super) fn layout_glyph(&self) -> &'static str {
        match self.panes.orientation() {
            PaneLayout::SideBySide => "▤",
            PaneLayout::Stacked => "▥",
        }
    }
}
