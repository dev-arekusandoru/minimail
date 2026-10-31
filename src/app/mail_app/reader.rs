//! The reader pane: the opened thread as one chronological timeline (oldest first) on a
//! state-colored rail, the opened message expanded in place. The pieces live in `reader_*.rs`.
use gpui_kit::component::{scroll::Scrollbar, ActiveTheme as _};

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

use parts::{Look, Role, rail_row};

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
    /// Scroll to the inline composer on the next render. It is the last child of the
    /// thread, so a reply opened from a message further up would otherwise land below the
    /// fold with its Send and Close buttons out of sight.
    pub(super) reveal_composer: bool,
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
    /// The reader body: the find bar (when open) above the opened thread. The tab bar is
    /// the dock's now — one tab per open thread, drawn by each reader panel's title.
    pub(super) fn render_reader(&self, cx: &Context<Self>) -> AnyElement {
        if let Some(pane) = self.render_draft_tab() {
            return pane;
        }
        let pane = self.render_reader_pane(cx);
        if self.slot() == Slot::Triage || self.tabs.is_empty() {
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
            .children(if self.find_open() { self.find_bar(&Look::new(cx)) } else { None })
            .child(pane)
            .into_any_element()
    }

    /// The active tab is a draft: the composer fills the reader pane. The tab itself is the
    /// dock's, and there is no thread behind the draft, so no find bar either.
    pub(super) fn render_draft_tab(&self) -> Option<AnyElement> {
        let draft = self.tabs.active_draft()?;
        let composer = self.composers.get(&draft)?.clone();
        let stacked = self.panes.orientation() == PaneLayout::Stacked;
        Some(
            div()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .flex()
                .flex_col()
                .when(!stacked, |d| d.h_full())
                .when(stacked, |d| d.w_full())
                .child(composer)
                .into_any_element(),
        )
    }

    /// The reply composer shown inline under the conversation it answers, as a card matching
    /// the message cards. It is the same entity the tab would show, so expanding moves it
    /// rather than copying it.
    pub(super) fn inline_composer_element(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let draft = self.inline_composer_for_thread()?;
        let composer = self.composers.get(&draft)?.clone();
        let t = cx.theme();
        Some(
            div()
                .id("reader-reply-card").test_support()
                .flex_none()
                .h(px(420.))
                .rounded_md()
                .overflow_hidden()
                .border_1()
                .border_color(t.border)
                .bg(message::message_card_bg(t))
                .child(composer)
                .into_any_element(),
        )
    }

    fn render_reader_pane(&self, cx: &Context<Self>) -> AnyElement {
        let t = cx.theme();
        let stacked = self.panes.orientation() == PaneLayout::Stacked;
        let pane = div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .px_5()
            .py_4()
            .when(!stacked, |d| d.h_full())
            .when(stacked, |d| d.w_full());
        let Some(msg) = self.opened().and_then(|id| self.mailbox.get(id)) else {
            if self.mailbox.accounts().is_empty() {
                return pane.into_any_element();
            }
            return pane
                .child(crate::app::ui::empty(
                    gpui_kit::assets::IconName::MailOpen,
                    "No message open",
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .justify_center()
                        .gap_1()
                        .child("Click a message or press")
                        .child(crate::app::ui::shortcut("enter"))
                        .child("to open"),
                ))
                .into_any_element();
        };
        let look = Look::new(cx);
        let newest = self.newest();
        let others = crate::reading::thread_others(self.mailbox.messages(), msg);
        let order = crate::threads::thread_order(self.mailbox.messages(), msg.thread_id);
        // Thread messages in timeline order (oldest first). The opened one is the expanded
        // surface wherever it falls; it is never moved.
        let inline = self.inline_composer_element(cx);
        let lone = others.is_empty();
        let (title, mut rows): (Option<AnyElement>, Vec<AnyElement>) = if lone {
            // A lone message has no thread, so no rail: the surface takes the full width.
            (
                None,
                vec![self.message_surface(msg, Role::Opened, &look, cx)],
            )
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
                    // The inline reply is the last row: the rail continues down to its dot.
                    let last = i + 1 == count && inline.is_none();
                    rail_row(t, Some((dot, color)), i == 0, last, el).into_any_element()
                })
                .collect();
            (Some(self.thread_title(msg, &others, &look, cx)), rows)
        };
        // The reply is a card after the last one: on the rail in a thread, below the message
        // with a gap when there is no thread.
        if let Some(el) = inline {
            rows.push(if lone {
                div().mt_3().child(el).into_any_element()
            } else {
                rail_row(t, Some((DOT_FULL, t.primary)), false, true, el).into_any_element()
            });
        }

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
                    if m.segment == crate::find::Segment::Subject {
                        0
                    } else {
                        lead + at
                    }
                });
                pane.scroll.scroll_to_top_of_item(target);
                pane.find_reveal = FindReveal::Line;
            }
            if std::mem::take(&mut pane.reveal_composer) {
                // The composer is the last child, so the bottom is where it starts.
                pane.scroll.scroll_to_bottom();
            }
            pane.scroll.clone()
        };

        // The scroll container fills the whole pane, so the wheel works in its margins too: the
        // pane's padding moves onto the content (side margins on each row, top and bottom on
        // the first and last).
        let column = |el: AnyElement| div().w_full().px_5().child(el);
        let count = usize::from(summary.is_some()) + usize::from(title.is_some()) + rows.len();
        let edges = move |el: Div, i: usize| el.when(i == 0, |d| d.pt_4()).when(i + 1 == count, |d| d.pb_4());
        pane.p_0()
            .child(
                // The scrollbar is a sibling of the scrolled area, over a relative wrapper the
                // same size as it: as a child it would ride the content's scroll offset, so the
                // thumb would cancel its own movement and read as stuck at the top.
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .id("reader-scroll")
                            // A click anywhere in the thread makes the tab permanent.
                            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                                this.pin_active_tab(cx);
                            }))
                            .track_scroll(&scroll)
                            .size_full()
                            .overflow_y_scroll()
                            .flex()
                            .flex_col()
                            .when(true, |d| {
                                let mut i = 0;
                                let mut d = d;
                                if let Some(s) = summary {
                                    d = d.child(edges(column(SummaryCard::new(&s).into_any_element()), i).pb_3());
                                    i += 1;
                                }
                                if let Some(el) = title {
                                    d = d.child(edges(column(el), i).pb_3());
                                    i += 1;
                                }
                                for el in rows {
                                    d = d.child(edges(column(el), i));
                                    i += 1;
                                }
                                d
                            }),
                    )
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .child(Scrollbar::vertical(&scroll).id("reader-scrollbar").viewport_from_layout()),
                    ),
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
