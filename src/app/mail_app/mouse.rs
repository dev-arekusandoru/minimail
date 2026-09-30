//! Mouse entry points of the root view: row clicks, checkboxes, suggestion badges and the
//! toolbar. Buttons dispatch the same actions the keys do; row clicks reuse the cursor and
//! selection primitives behind `j`/`k`/`shift-j`/`x`.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{ListMode, MailApp};
use crate::app::actions::*;
use crate::app::ui::{button, run};

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
    pub(super) fn row_checkbox(&self, ix: usize, selected: bool, cx: &Context<Self>) -> Stateful<Div> {
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
            .on_click(cx.listener(move |this, _, window, cx| this.toggle_row(ix, window, cx)))
    }

    /// Search box, global buttons and the message/selection action bar.
    pub(super) fn render_toolbar(&self, cx: &Context<Self>) -> AnyElement {
        let t = crate::theme::active(cx);
        let searching = matches!(self.mode, ListMode::Search(_));
        let search_text = self.search_header().unwrap_or_else(|| "Search…".into());
        let sep = || div().flex_none().w(px(1.)).h(px(14.)).bg(t.border);
        let label = |text: &'static str| {
            div().flex_none().text_size(px(11.)).text_color(t.text_muted).child(text)
        };
        let b = |id: &'static str, text: &'static str, tip: &str, key: &str| {
            button(id, text, tip, key, cx)
        };

        let search = div()
            .id("search-box")
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

        let top = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(search)
            .when(searching, |d| {
                d.child(b("search-clear", "Clear search", "Leave search", "escape").on_click(run(ClearSelection)))
            })
            .child(sep())
            .child(b("btn-palette", "Commands", "Command palette", "cmd-k").on_click(run(ToggleCommandPalette)))
            .child(b("btn-session", "Triage session", "Start a triage session", "t").on_click(run(StartSession)))
            .child(b("btn-undo", "Undo", "Undo (recalls an unsent reply first)", "u").on_click(run(Undo)))
            .child(b("btn-classify", "Classify", "Classify visible mail again", "c").on_click(run(ClassifyVisible)))
            .child(b("btn-rules", "Rules", "Sender rules", "shift-r").on_click(run(ToggleRules)))
            .child(b("btn-settings", "Settings", "Settings", "cmd-,").on_click(run(ToggleSettings)))
            .child(b("btn-help", "?", "All shortcuts", "?").on_click(run(ToggleHelp)));

        let actions = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(b("btn-done", "Done", "Mark done", "e").on_click(run(MarkDone)))
            .child(b("btn-waiting", "Waiting", "Mark waiting", "w").on_click(run(MarkWaiting)))
            .child(b("btn-inbox", "Inbox", "Move to inbox", "i").on_click(run(MoveToInbox)))
            .child(b("btn-later", "Later…", "Later, with a return time", "l").on_click(run(OpenSnoozePicker)))
            .child(b("btn-select", "Select", "Toggle select", "x").on_click(run(ToggleSelect)))
            .child(sep())
            .child(b("btn-reply", "Reply", "Reply", "r").on_click(run(Reply)))
            .child(b("btn-summarize", "Summarize", "Summarize thread", "s").on_click(run(SummarizeThread)))
            .child(b("btn-accept", "Accept AI", "Accept AI badges", "y").on_click(run(AcceptSuggestions)))
            .child(b("btn-reject", "Reject AI", "Reject AI badges", "n").on_click(run(RejectSuggestions)))
            .child(b("btn-mute", "Mute", "Mute thread", "m").on_click(run(MuteThread)))
            .child(b("btn-unsubscribe", "Unsubscribe", "Unsubscribe from sender", "shift-u").on_click(run(Unsubscribe)))
            .child(sep())
            .child(label("All from sender:"))
            .child(b("btn-sender-done", "Done", "Mark all from sender done", "shift-e").on_click(run(SenderDone)))
            .child(b("btn-sender-waiting", "Waiting", "Mark all from sender waiting", "shift-w").on_click(run(SenderWaiting)))
            .child(b("btn-sender-inbox", "Inbox", "Move all from sender to inbox", "shift-i").on_click(run(SenderInbox)))
            .child(b("btn-sender-later", "Later", "Mark all from sender later", "shift-l").on_click(run(SenderLater)));

        div()
            .flex()
            .flex_col()
            .flex_none()
            .gap_1()
            .px_3()
            .py_2()
            .bg(t.sidebar)
            .border_b_1()
            .border_color(t.border)
            .child(top)
            .child(actions)
            .into_any_element()
    }
}
