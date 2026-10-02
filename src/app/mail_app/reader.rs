//! The reader pane: the opened thread as one chronological timeline (oldest first) on a
//! state-colored rail, the opened message expanded in place. The pieces live in `reader_*.rs`.
use gpui_kit::component::{scroll::ScrollableElement as _, ActiveTheme as _};

use super::*;

#[path = "reader_findbar.rs"]
mod findbar;
#[path = "reader_message.rs"]
mod message;
#[path = "reader_parts.rs"]
mod parts;
#[path = "reader_tabbar.rs"]
mod tabbar;
#[path = "reader_thread.rs"]
mod thread;

use parts::{rail_row, Look, Role};

/// Rail dot centers: inside a full surface (first line of its header) and in a collapsed line.
const DOT_FULL: f32 = 27.;
const DOT_COLLAPSED: f32 = thread::COLLAPSED_H / 2.;

/// What the reader remembers per tabbed thread besides its disclosure state: the scroll position,
/// and the message it last scrolled to (a different opened message triggers a reveal).
#[derive(Default)]
pub(super) struct ReaderPane {
    pub(super) scroll: ScrollHandle,
    revealed: Option<MessageId>,
    /// Where the current find match is in being scrolled into view.
    pub(super) find_reveal: FindReveal,
}

/// Bringing the current find match into view takes two frames: scroll its message to the top
/// (needs the row layout), then nudge the exact line into the viewport (needs the painted text).
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum FindReveal {
    #[default]
    Idle,
    /// Scroll the match's message into view on the next render.
    Message,
    /// The message is in view: place the match's own line next frame.
    Line,
}

impl MailApp {
    /// The reader: the tab bar (outside a triage session) above the opened thread.
    pub(super) fn render_reader(&self, cx: &Context<Self>) -> AnyElement {
        let pane = self.render_reader_pane(cx);
        if self.in_session() || self.session_end.is_some() || self.tabs.is_empty() {
            return pane;
        }
        let stacked = self.panes.orientation() == PaneLayout::Stacked;
        div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .when(!stacked, |d| d.h_full())
            .when(stacked, |d| d.w_full())
            .child(self.tab_bar(&Look::new(cx), cx))
            .children(if self.find_open() { self.find_bar(&Look::new(cx)) } else { None })
            .child(pane)
            .into_any_element()
    }

    fn render_reader_pane(&self, cx: &Context<Self>) -> AnyElement {
        let t = cx.theme();
        let stacked = self.panes.orientation() == PaneLayout::Stacked;
        let pane = div().flex_1().min_w_0().min_h_0().flex().flex_col().px_5().py_4()
            .when(!stacked, |d| d.h_full())
            .when(stacked, |d| d.w_full());
        if let Some((handled, secs)) = self.session_end {
            return pane
                .items_center()
                .justify_center()
                .child(SessionCard::finished(handled, secs))
                .into_any_element();
        }
        let Some(msg) = self.opened().and_then(|id| self.mailbox.get(id)) else {
            if self.mailbox.accounts().is_empty() {
                return pane.into_any_element();
            }
            return pane
                .items_center()
                .justify_center()
                .flex()
                .gap_1()
                .text_size(px(13.))
                .text_color(t.muted_foreground)
                .child("Click a message or press")
                .child(crate::app::ui::shortcut("enter"))
                .child("to open")
                .into_any_element();
        };
        let look = Look::new(cx);
        let newest = self.newest();
        let others = crate::reading::thread_others(self.mailbox.messages(), msg);
        let order = crate::threads::thread_order(self.mailbox.messages(), msg.thread_id);

        // Thread messages in timeline order (oldest first). The opened one is the expanded
        // surface wherever it falls; it is never moved.
        let (title, rows): (Option<AnyElement>, Vec<AnyElement>) = if others.is_empty() {
            // A lone message has no thread, so no rail: the surface takes the full width.
            (None, vec![self.message_surface(msg, Role::Opened, &look, cx)])
        } else {
            let count = order.len();
            let rows = order
                .iter()
                .filter_map(|id| self.mailbox.get(*id))
                .enumerate()
                .map(|(i, m)| {
                    let color = theme::state_color(t, m.state);
                    let (dot, el) = if m.id == msg.id {
                        (DOT_FULL, self.message_surface(m, Role::Opened, &look, cx))
                    } else {
                        let expanded = self.reader.is_expanded(m.thread_id, m.id);
                        let dot = if expanded { DOT_FULL } else { DOT_COLLAPSED };
                        (dot, self.thread_message(m, expanded, &newest, &look, cx))
                    };
                    rail_row(t, Some((dot, color)), i == 0, i + 1 == count, el)
                        .into_any_element()
                })
                .collect();
            (Some(self.thread_title(msg, &others, &look, cx)), rows)
        };

        // Scroll container children, in order: summary card, thread title, message rows. The
        // opened message's child index is what `scroll_to_top_of_item` needs.
        let summary = self.summary_shown();
        let lead = usize::from(summary.is_some()) + usize::from(title.is_some());
        let at = order.iter().position(|&id| id == msg.id).unwrap_or(0);
        let scroll = {
            let mut panes = self.reader_panes.borrow_mut();
            let pane = panes.entry(msg.thread_id).or_default();
            if pane.revealed.replace(msg.id) != Some(msg.id) {
                pane.scroll.scroll_to_top_of_item(lead + at);
            }
            if pane.find_reveal == FindReveal::Message {
                let target = self.find_current(msg.thread_id).map_or(0, |m| {
                    let at = order.iter().position(|&id| id == m.msg).unwrap_or(0);
                    if m.segment == crate::find::Segment::Subject { 0 } else { lead + at }
                });
                pane.scroll.scroll_to_top_of_item(target);
                pane.find_reveal = FindReveal::Line;
            }
            pane.scroll.clone()
        };

        let column = |el: AnyElement| div().w_full().child(el);
        pane.gap_3()
            .when_some(self.session.as_ref(), |d, s| {
                d.child(SessionCard::new(s.index + 1, s.ids.len()))
            })
            .child(
                div()
                    .id("reader-scroll")
                    // A click anywhere in the thread makes the tab permanent.
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                        this.pin_active_tab(cx);
                    }))
                    .track_scroll(&scroll)
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .when_some(summary, |d, s| d.child(column(SummaryCard::new(&s).into_any_element()).pb_3()))
                    .when_some(title, |d, el| d.child(column(el).pb_3()))
                    .children(rows.into_iter().map(column))
                    .vertical_scrollbar(&scroll),
            )
            .into_any_element()
    }
}

/// `Wed Oct 7 08:00` on the user's own clock, for snooze return times.
pub(super) fn format_when(ts: Timestamp, now: &Now) -> String {
    let (days, _) = now.parts(ts);
    let (_, month, day) = now.civil(ts);
    format!(
        "{} {} {day} {}",
        WEEKDAYS[Now::weekday(days)],
        MONTHS[(month - 1) as usize],
        now.hhmm(ts)
    )
}
