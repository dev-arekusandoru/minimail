//! Group-by-thread list mode and thread navigation for the root view.
//!
//! Semantics: with grouping on, State panels show one row per thread (a thread with a
//! single message in the panel stays an ordinary row). A collapsed header stands for every
//! message of that thread *in this panel*: `e`/`w`/`i`/`l`, `x` and shift-selection apply to
//! all of them in one undo step. Sender-wide actions and mute are unchanged (mute already
//! covers the whole thread). Threads split across states show only the panel's messages.
//! `]` / `[` walk the whole thread in date order regardless of the grouping setting.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{ListMode, MailApp};
use crate::app::row;
use crate::model::MessageId;
use crate::threads::{self, Row};

impl MailApp {
    /// Grouped rows are in effect: setting on, a state panel, not in a triage session.
    pub(super) fn grouped(&self) -> bool {
        self.group_threads && self.mode == ListMode::State && !self.in_session()
    }

    /// The list rows when grouped (empty otherwise): threads collapsed or expanded.
    pub fn rows(&self) -> Vec<Row> {
        if !self.grouped() {
            return Vec::new();
        }
        let ids = self.mailbox.ids_in(self.triage.view);
        let groups = threads::group(&ids, |id| self.mailbox.get(id));
        threads::rows(&groups, &self.expanded)
    }

    /// Index of the cursor row, clamped to the rows.
    pub fn row_cursor(&self) -> usize {
        self.row_cursor.min(self.rows().len().saturating_sub(1))
    }

    /// Row under the cursor (grouped mode).
    pub(super) fn cursor_row(&self) -> Option<Row> {
        let rows = self.rows();
        let at = self.row_cursor.min(rows.len().saturating_sub(1));
        rows.into_iter().nth(at)
    }

    /// Message under the cursor, whatever the list mode.
    pub fn cursor_message(&self) -> Option<MessageId> {
        self.cursor_id()
    }

    pub(super) fn row_selected(&self, row: &Row) -> bool {
        let ids = row.ids();
        ids.iter().all(|id| self.triage.is_selected(*id))
    }

    /// Shift-selection over rows, like `shift-j`/`shift-k`.
    pub(super) fn extend_by(&mut self, delta: isize) {
        if !self.grouped() {
            self.triage.extend(&self.mailbox, delta);
            return;
        }
        let rows = self.rows();
        if rows.is_empty() {
            return;
        }
        if self.triage.selected().is_empty() {
            self.row_anchor = None;
        }
        let cur = self.row_cursor.min(rows.len() - 1);
        let anchor = *self.row_anchor.get_or_insert(cur);
        let next = (cur as isize + delta).clamp(0, rows.len() as isize - 1) as usize;
        self.row_cursor = next;
        let (lo, hi) = (anchor.min(next), anchor.max(next));
        let mut ids: Vec<MessageId> = Vec::new();
        for row in &rows[lo..=hi] {
            for id in row.ids() {
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
        }
        self.triage.set_selection(ids);
    }

    /// `x`: toggle the cursor row (every message of a thread row) in the selection.
    pub(super) fn toggle_select_cursor(&mut self) {
        if !self.grouped() {
            self.triage.toggle_select(&self.mailbox);
            return;
        }
        let Some(row) = self.cursor_row() else { return };
        let mut sel = self.triage.selected();
        if self.row_selected(&row) {
            sel.retain(|id| !row.ids().contains(id));
        } else {
            for id in row.ids() {
                if !sel.contains(&id) {
                    sel.push(id);
                }
            }
        }
        self.triage.set_selection(sel);
    }

    /// Expand (`true`) or collapse the thread under the cursor. Collapsing from a child
    /// row puts the cursor on the header.
    pub(super) fn set_expanded(&mut self, expand: bool) {
        if !self.grouped() {
            return;
        }
        let Some(row) = self.cursor_row() else { return };
        let Some(thread) = row.thread_id() else { return };
        if expand {
            self.expanded.insert(thread);
        } else {
            self.expanded.remove(&thread);
            if let Some(at) = self
                .rows()
                .iter()
                .position(|r| matches!(r, Row::Header { thread_id, .. } if *thread_id == thread))
            {
                self.row_cursor = at;
            }
        }
    }

    /// Chevron click / toggle for the header of `thread`.
    pub(super) fn toggle_thread(&mut self, thread: u32) {
        if !self.expanded.remove(&thread) {
            self.expanded.insert(thread);
        }
        let len = self.rows().len();
        self.row_cursor = self.row_cursor.min(len.saturating_sub(1));
    }

    /// `enter`: open the cursor message; a collapsed thread header also expands.
    pub(super) fn open_cursor(&mut self) {
        if self.grouped()
            && let Some(Row::Header { expanded: false, thread_id, .. }) = self.cursor_row()
        {
            self.expanded.insert(thread_id);
        }
        if let Some(id) = self.cursor_id() {
            self.opened = Some(id);
        }
    }

    /// Put the list cursor on message `id` when it is in the current list. `reveal` expands
    /// its thread in grouped mode so the message row itself is reachable.
    pub(super) fn focus_message(&mut self, id: MessageId, reveal: bool) {
        if self.in_session() {
            return;
        }
        if self.grouped() {
            let thread = self.mailbox.get(id).map(|m| m.thread_id);
            let in_panel = self.mailbox.ids_in(self.triage.view).contains(&id);
            if reveal && in_panel && let Some(t) = thread {
                self.expanded.insert(t);
            }
            let rows = self.rows();
            let exact = rows.iter().position(|r| {
                matches!(r, Row::Single(x) | Row::Child { id: x, .. } if *x == id)
            });
            let header = || {
                rows.iter()
                    .position(|r| matches!(r, Row::Header { ids, .. } if ids.contains(&id)))
            };
            if let Some(at) = exact.or_else(header) {
                self.row_cursor = at;
            }
            return;
        }
        let ids = self.visible_ids();
        if let Some(at) = ids.iter().position(|x| *x == id) {
            if self.mode == ListMode::State {
                self.triage.set_cursor(&self.mailbox, at);
            } else {
                self.alt_cursor = at;
            }
        }
    }

    /// Switch grouping, keeping the cursor on the same message.
    pub(super) fn set_grouping(&mut self, on: bool) {
        if self.group_threads == on {
            return;
        }
        let cur = self.cursor_id();
        self.group_threads = on;
        self.triage.clear_selection();
        self.row_anchor = None;
        if let Some(id) = cur {
            self.focus_message(id, false);
        }
    }

    pub(super) fn toggle_grouping(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_grouping(!self.group_threads);
        let text = if self.group_threads { "Grouped by thread" } else { "Ungrouped" };
        self.show_toast(text.into(), window, cx);
        self.scroll_to_cursor();
        cx.notify();
    }

    /// `]` / `[`: open the next/previous message of the current thread in date order.
    pub(super) fn step_thread(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(from) = self.opened.or_else(|| self.cursor_id()) else {
            return;
        };
        let Some(thread) = self.mailbox.get(from).map(|m| m.thread_id) else {
            return;
        };
        let order = threads::thread_order(self.mailbox.messages(), thread);
        match threads::step(&order, from, delta) {
            Some(target) => {
                self.opened = Some(target);
                self.focus_message(target, true);
                self.scroll_to_cursor();
            }
            None => {
                let text = if delta > 0 { "Last message in thread" } else { "First message in thread" };
                self.show_toast(text.into(), window, cx);
            }
        }
        cx.notify();
    }
}

impl MailApp {
    /// Thread header row: chevron, latest sender, subject, participants, count badge, the
    /// newest date and the snippet of the newest message while the Preview setting asks for
    /// one. It sizes to that content, not to the message-row height. Click selects it; the
    /// chevron expands/collapses.
    pub(super) fn render_group_header(
        &self,
        row: &Row,
        ix: usize,
        newest: &str,
        cx: &Context<Self>,
    ) -> crate::app::ui::Observable {
        let Row::Header { thread_id, ids, expanded } = row else {
            unreachable!("render_group_header takes header rows")
        };
        let t = crate::theme::active(cx);
        let latest = self.mailbox.get(ids[0]);
        let (from, subject, date) = latest
            .map(|m| (m.from_name.clone(), m.subject.clone(), Self::clock_label(&m.received, newest)))
            .unwrap_or_default();
        let people = threads::participants(ids, |id| self.mailbox.get(id)).join(", ");
        let selected_count = ids.iter().filter(|id| self.triage.is_selected(**id)).count();
        let unread = ids.iter().any(|id| {
            !self.read.contains(id) && self.mailbox.state_of(*id) == Some(crate::model::TriageState::Inbox)
        });
        let urgent = unread && ids.iter().any(|id| {
            !self.read.contains(id)
                && self.mailbox.state_of(*id) == Some(crate::model::TriageState::Inbox)
                && self.mailbox.tags(*id).iter().any(|tag| matches!(tag, crate::model::Tag::Urgent(_)))
        });
        let visual = crate::app::row::RowVisual {
            cursor: ix == self.cursor_ix(),
            open: self.opened.is_some_and(|id| ids.contains(&id)),
            selected: selected_count == ids.len() && selected_count > 0,
            partial: selected_count > 0 && selected_count < ids.len(),
            unread,
            urgent,
        };
        let thread = *thread_id;
        let preview = row::thread_preview_lines(self.preview_lines);
        let snippet = latest.map(|m| crate::preview::snippet(&m.body)).unwrap_or_default();
        crate::app::row::frame(
            div()
                .id(("thread-row", thread as usize))
                .h(px(self.thread_row_h())),
            visual,
            &t,
        )
            .child(self.row_selection_target(ix, cx))
            .child(
                div()
                    .id(("thread-chevron", thread as usize))
                    .flex_none()
                    .w(px(12.))
                    .text_color(t.text_muted)
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.cursor_to(ix);
                        this.toggle_thread(thread);
                        window.focus(&this.focus_handle, cx);
                        cx.notify();
                    }))
                    .child(if *expanded { "▾" } else { "▸" }),
            )
            .child(
                div()
                    .id(("thread-body", thread as usize))
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                        this.click_row(ix, ev, window, cx)
                    }))
                    .child(
                        div()
                            .h(px(row::LINE_H))
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .w(px(108.))
                                    .flex_none()
                                    .truncate()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(from),
                            )
                            .child(div().flex_1().truncate().text_color(t.text_muted).child(subject))
                            .child(
                                div()
                                    .w(px(90.))
                                    .flex_none()
                                    .truncate()
                                    .text_size(px(11.))
                                    .text_color(t.text_muted)
                                    .child(people),
                            ),
                    )
                    .when(preview > 0, |d| {
                        d.child(
                            div()
                                .h(px(row::PREVIEW_LINE_H * f32::from(preview)))
                                .line_clamp(preview as usize)
                                .text_size(px(12.))
                                .line_height(px(row::PREVIEW_LINE_H))
                                .text_color(t.text_muted)
                                .child(snippet),
                        )
                    }),
            )
            .child(
                div()
                    .flex_none()
                    .px_1()
                    .rounded_sm()
                    .text_size(px(11.))
                    .border_1()
                    .border_color(t.border)
                    .text_color(t.text_muted)
                    .child(format!("{}", ids.len())),
            )
            .child(
                div()
                    .id(("thread-date", thread as usize))
                    .flex_none()
                    .text_size(px(11.))
                    .text_color(t.text_muted)
                    .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                        this.click_row(ix, ev, window, cx)
                    }))
                    .child(date),
            )
    }
}
