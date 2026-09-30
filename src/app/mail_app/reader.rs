//! The reader pane: the opened thread as one chronological timeline (oldest first) on a
//! state-colored rail, the opened message expanded in place, plus the optional action toolbar.
//! The pieces live in `reader_*.rs`.

use super::*;

#[path = "reader_message.rs"]
mod message;
#[path = "reader_parts.rs"]
mod parts;
#[path = "reader_thread.rs"]
mod thread;

use parts::{rail_row, Look, Role};

/// Widest the reader column grows; longer lines stop being readable.
const READER_MAX_W: f32 = 860.;
/// Rail dot centers: inside a full surface (first line of its header) and in a collapsed line.
const DOT_FULL: f32 = 27.;
const DOT_COLLAPSED: f32 = thread::COLLAPSED_H / 2.;

impl MailApp {
    pub(super) fn render_reader(&self, cx: &Context<Self>) -> AnyElement {
        let t = theme::active(cx);
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
        let Some(msg) = self.opened.and_then(|id| self.mailbox.get(id)) else {
            self.reader_revealed.set(None);
            return pane
                .items_center()
                .justify_center()
                .text_size(px(13.))
                .text_color(t.text_muted)
                .child("Click a message or press enter to open")
                .into_any_element();
        };
        let look = Look::new(self.reader_toolbar, cx);
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
                    let color = t.state_color(m.state);
                    let (dot, el) = if m.id == msg.id {
                        (DOT_FULL, self.message_surface(m, Role::Opened, &look, cx))
                    } else if self.reader.is_expanded(m.thread_id, m.id) {
                        (DOT_FULL, self.message_surface(m, Role::Thread, &look, cx))
                    } else {
                        (DOT_COLLAPSED, self.collapsed_line(m, &newest, &look, cx))
                    };
                    rail_row(&t, Some((dot, color)), i == 0, i + 1 == count, el)
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
        if self.reader_revealed.replace(Some(msg.id)) != Some(msg.id) {
            self.reader_scroll.scroll_to_top_of_item(lead + at);
        }

        let column = |el: AnyElement| div().w_full().max_w(px(READER_MAX_W)).child(el);
        pane.gap_3()
            .when_some(self.session.as_ref(), |d, s| {
                d.child(SessionCard::new(s.index + 1, s.ids.len()))
            })
            .when(self.reader_toolbar, |d| d.child(self.reader_toolbar_row(&look, cx)))
            .child(
                div()
                    .id("reader-scroll")
                    .track_scroll(&self.reader_scroll)
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .when_some(summary, |d, s| d.child(column(SummaryCard::new(&s).into_any_element()).pb_3()))
                    .when_some(title, |d, el| d.child(column(el).pb_3()))
                    .children(rows.into_iter().map(column)),
            )
            .into_any_element()
    }

    /// Thread-level actions with their keys. Only drawn when the toolbar setting is on; the
    /// footer hint bar carries the same keys otherwise.
    fn reader_toolbar_row(&self, look: &Look, cx: &Context<Self>) -> Div {
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap_2()
            .child(look.action_button("btn-reader-archive", "Archive", "Archive", "e", Archive, cx))
            .child(look.action_button("btn-reader-file", "File", "File into a folder", "f", File, cx))
            .child(look.action_button("btn-reader-delete", "Delete", "Delete", "d", Delete, cx))
            .child(look.action_button(
                "btn-reader-snooze",
                "Snooze…",
                "Snooze",
                "s",
                OpenSnoozePicker,
                cx,
            ))
    }
}

/// `Wed Oct 7 08:00` (UTC) for snooze return times.
pub(super) fn format_when(ts: Timestamp) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    let days = ts.div_euclid(DAY);
    let secs = ts.rem_euclid(DAY);
    let weekday = WEEKDAYS[(days + 4).rem_euclid(7) as usize];
    // Civil-from-days (Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    format!(
        "{weekday} {} {day} {:02}:{:02}",
        MONTHS[(month - 1) as usize],
        secs / 3600,
        secs % 3600 / 60
    )
}
