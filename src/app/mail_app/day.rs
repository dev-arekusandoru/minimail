//! Day headers and the list's item model.
//!
//! The list holds day headers interleaved with rows (messages, or thread headers plus their
//! children). Cursor, selection and `j`/`k` live in *row* space and never see headers;
//! [`ListModel`] maps between the two spaces. Messages of collapsed days are not in
//! `visible_ids()` at all, so everything that works on rows skips them consistently.
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::{Icon, Sizable as _};

use super::*;
use crate::day_groups::{self, Day, DayKey, Item};
use crate::theme::ThemeColor;

/// Height of a day header.
const DAY_H: f32 = 22.;

/// The list as items: where day headers and rows sit.
pub(super) struct ListModel {
    pub days: Vec<Day>,
    pub items: Vec<Item>,
}

impl ListModel {
    /// List item index of row `ix` (the last item when `ix` is past the rows).
    pub fn item_of_row(&self, ix: usize) -> usize {
        self.items
            .iter()
            .position(|i| *i == Item::Row(ix))
            .unwrap_or(self.items.len().saturating_sub(1))
    }

    pub fn row_count(&self) -> usize {
        self.items.iter().filter(|i| matches!(i, Item::Row(_))).count()
    }
}

/// The rail segment of a bordered row, header or child: one full-height 1px line behind the
/// content, so the segments of neighbouring rows join into the timeline.
pub(super) fn rail_segment(t: &ThemeColor) -> Div {
    div().absolute().left(px(61.)).top(px(-1.)).bottom(px(-1.)).w(px(1.)).bg(t.border)
}

/// The same line for a day header, which has no border.
pub(super) fn day_rail_segment(t: &ThemeColor) -> Div {
    div().absolute().left(px(62.)).top_0().bottom_0().w(px(1.)).bg(t.border)
}

impl MailApp {
    /// Days and items of the current list.
    pub(super) fn list_model(&self) -> ListModel {
        let now = self.local_now();
        let all = self.mailbox.ids_matching(&self.triage().query, &now);
        let stamps: Vec<_> = all
            .iter()
            .map(|id| self.mailbox.get(*id).and_then(Message::received_at).unwrap_or(0))
            .collect();
        let days = day_groups::days(&now, &stamps);
        let row_days: Vec<DayKey> = if self.grouped() {
            let mut day = 0;
            self.rows()
                .iter()
                .map(|row| {
                    // Children sit in their header's day.
                    if !matches!(row, Row::Child { .. }) {
                        day = self.mailbox.day_of(row.primary(), &now);
                    }
                    day
                })
                .collect()
        } else {
            self.visible_ids().iter().map(|id| self.mailbox.day_of(*id, &now)).collect()
        };
        let items = day_groups::items(&days, &row_days, self.mailbox.collapsed_days());
        ListModel { days, items }
    }

    /// Gutter text of a rail row: the return time (`↩ …`) in Snoozed locations, else the
    /// clock time (`H:MM`, 24 hour, local) -- the day is on the header above.
    pub(super) fn gutter_label(&self, msg: &Message) -> String {
        let now = self.local_now();
        if let Some(until) = self.mailbox.snoozed_until(msg.id)
            && matches!(self.location(), Some(Location::Snoozed(_)))
        {
            return format!("↩ {}", format_when(until, &now));
        }
        match msg.received_at() {
            Some(ts) => {
                let secs = now.parts(ts).1;
                format!("{}:{:02}", secs / 3600, secs % 3600 / 60)
            }
            None => msg.received.get(11..16).unwrap_or("").to_string(),
        }
    }

    /// Collapse the day, or expand it again. Selected messages of a day that just
    /// disappeared are deselected, so no action can reach a row nobody can see.
    pub(super) fn toggle_day(&mut self, key: DayKey, cx: &mut Context<Self>) {
        let now = self.local_now();
        self.mailbox.toggle_day(key);
        let kept: Vec<MessageId> = self
            .triage()
            .selected()
            .into_iter()
            .filter(|id| !self.mailbox.collapsed_days().contains(&self.mailbox.day_of(*id, &now)))
            .collect();
        self.view_mut().triage.set_selection(kept);
        self.view_mut().row_anchor = None;
        self.move_cursor(0);
        cx.notify();
    }

    /// `ToggleDay`: collapse or expand the day of the cursor row.
    pub(super) fn toggle_cursor_day(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.cursor_id() {
            let key = self.mailbox.day_of(id, &self.local_now());
            self.toggle_day(key, cx);
        }
    }

    /// The header of `day` (`ix` is its index in the model's days): label, and when collapsed
    /// the message count; a chevron on hover, always while collapsed. Click toggles.
    pub(super) fn render_day_header(&self, ix: usize, day: &Day, cx: &Context<Self>) -> AnyElement {
        let t = cx.theme();
        let collapsed = self.mailbox.collapsed_days().contains(&day.key);
        let key = day.key;
        let label = day_groups::label(&self.local_now(), day.ts).to_uppercase();
        div()
            .id(("day", ix))
            .group("day-header")
            .test_support()
            .relative()
            .w_full()
            .h(px(DAY_H))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(4.))
            .pt(px(6.))
            .pl(px(11.))
            .pr(px(9.))
            .cursor_pointer()
            .font_family(crate::app::ui::mono_font(cx))
            .text_size(px(10.))
            .line_height(px(14.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(t.muted_foreground)
            .when(self.row_style.rail(), |d| d.child(day_rail_segment(t)))
            .child(div().relative().child(label))
            .when(collapsed, |d| d.child(div().relative().child(format!("· {}", day.count))))
            .child(
                div()
                    .relative()
                    .ml_auto()
                    .flex_none()
                    .opacity(if collapsed { 1. } else { 0. })
                    .group_hover("day-header", |s| s.opacity(1.))
                    .child(
                        Icon::new(if collapsed { IconName::ChevronRight } else { IconName::ChevronDown })
                            .with_size(px(12.)),
                    ),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_day(key, cx)))
            .into_any_element()
    }
}
