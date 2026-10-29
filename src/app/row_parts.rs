//! Building blocks shared by the Standard message row and the thread rows: the state marker,
//! chips, subject line and the `<b>sender</b> · preview` block. Geometry mirrors
//! `mockups/row-default.html`; colors are theme roles, never hexes.

use crate::app::row::{self, RowVisual};
use crate::theme::ThemeColor;
use gpui_kit::assets::IconName;
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::{prelude::FluentBuilder as _, *};

/// Hover group every row frame joins ([`row::frame`]); the marker's cutout follows it.
pub const ROW_GROUP: &str = "row";

/// Gutter of the rail layout: right-aligned 11px time, primary when unread.
pub fn gutter(text: impl Into<SharedString>, unread: bool, t: &ThemeColor) -> Div {
    div()
        .w(px(40.))
        .flex_none()
        .flex()
        .justify_end()
        .h(px(row::LINE_H))
        .items_center()
        .text_size(px(11.))
        .text_color(if unread { t.primary } else { t.muted_foreground })
        .child(text.into())
}

/// The 7px state marker of the rail layout, 18px high so it centres on the first line: a
/// filled dot when unread, a ring when read (both in `account` color when given), a checked
/// 13px circle when selected. The ring's hole is the real row background, hover included.
pub fn marker(v: RowVisual, account: Option<Hsla>, t: &ThemeColor) -> Div {
    let slot = div().w(px(7.)).h(px(row::LINE_H)).flex_none().relative();
    if v.selected {
        return slot.child(
            div()
                .absolute()
                .left(px(-3.))
                .top(px(2.5))
                .size(px(13.))
                .rounded_full()
                .bg(t.primary)
                .flex()
                .items_center()
                .justify_center()
                .child(Icon::new(IconName::Check).with_size(px(9.)).text_color(t.background)),
        );
    }
    let color = account.unwrap_or(if v.unread { t.primary } else { t.muted_foreground });
    let dot = div().absolute().left_0().top(px(5.5)).size(px(7.)).rounded_full().bg(color);
    if v.unread {
        return slot.child(dot);
    }
    let hole = div()
        .absolute()
        .left(px(1.5))
        .top(px(1.5))
        .size(px(4.))
        .rounded_full()
        .bg(row::row_bg(v, t))
        .when(row::hover_bg(v, t).is_some(), |d| {
            let hover = t.list_hover;
            d.group_hover(ROW_GROUP, move |s| s.bg(hover))
        });
    slot.child(dot.child(hole))
}

/// One suggested-folder or urgency chip.
pub struct Chip {
    pub icon: IconName,
    pub label: SharedString,
    pub color: Hsla,
}

/// The chips of a row: the suggested `folder` name, then the urgency chip for `urgency` levels
/// 4 (`Urgent`) and 5 (`Very urgent`); levels 1-3 carry none.
pub fn chips_for(folder: Option<String>, urgency: Option<u8>, t: &ThemeColor) -> Vec<Chip> {
    let folder = folder.map(|name| Chip {
        icon: IconName::FolderInput,
        label: name.into(),
        color: t.primary,
    });
    let urgent = match urgency {
        Some(5..) => Some(Chip { icon: IconName::Siren, label: "Very urgent".into(), color: t.danger }),
        Some(4) => Some(Chip { icon: IconName::Flame, label: "Urgent".into(), color: t.warning }),
        _ => None,
    };
    folder.into_iter().chain(urgent).collect()
}

/// The chips line (18px, 4px gap); `None` when the row has no chip, so it takes no height.
pub fn chips_line(chips: Vec<Chip>, t: &ThemeColor) -> Option<Div> {
    (!chips.is_empty()).then(|| {
        div().h(px(row::LINE_H)).flex().items_center().gap(px(4.)).children(chips.into_iter().map(|c| {
            div()
                .h(px(16.))
                .px(px(6.))
                .rounded(px(4.))
                .flex_none()
                .flex()
                .items_center()
                .gap(px(3.))
                .bg(t.accent)
                .text_size(px(11.))
                .text_color(c.color)
                .child(Icon::new(c.icon).with_size(px(11.)).text_color(c.color))
                .child(c.label)
        }))
    })
}

/// Line 1 of a row: optional account icon, the title (truncated, grows), optional trailing
/// element (the icon cluster in the rail layout). `title` carries its own weight/color.
pub fn title_line(account: Option<AnyElement>, title: Div, trailing: Option<AnyElement>) -> Div {
    div()
        .h(px(row::LINE_H))
        .flex()
        .items_center()
        .gap(px(4.))
        .min_w_0()
        .when_some(account, |d, icon| d.child(icon))
        .child(title.flex_1().min_w_0().truncate())
        .when_some(trailing, |d, t| d.child(div().flex_none().child(t)))
}

/// A title text div: foreground, semibold when `emphasis`.
pub fn title(text: impl Into<SharedString>, emphasis: bool, t: &ThemeColor) -> Div {
    let weight = if emphasis { FontWeight::SEMIBOLD } else { FontWeight::NORMAL };
    div().text_color(t.foreground).font_weight(weight).child(text.into())
}

/// The text block under line 1: `sender · snippet` (sender in foreground, semibold when
/// `emphasis`) in one flow clamped to `lines`, muted 12px/16px. `sender: None` shows the
/// snippet alone. `None` when `lines` is 0 or there is no text, so the row shrinks.
pub fn text_block(
    sender: Option<&str>,
    snippet: &str,
    lines: u8,
    emphasis: bool,
    t: &ThemeColor,
) -> Option<Div> {
    if lines == 0 {
        return None;
    }
    let (text, bold) = match (sender, snippet.is_empty()) {
        (Some(s), false) => (format!("{s} · {snippet}"), Some(0..s.len())),
        (Some(s), true) => (s.to_owned(), Some(0..s.len())),
        (None, _) => (snippet.to_owned(), None),
    };
    if text.is_empty() {
        return None;
    }
    let weight = if emphasis { FontWeight::SEMIBOLD } else { FontWeight::NORMAL };
    let highlights = bold.map(|range| {
        (range, HighlightStyle { color: Some(t.foreground), font_weight: Some(weight), ..Default::default() })
    });
    Some(
        div()
            .h(px(row::PREVIEW_LINE_H * f32::from(lines)))
            .line_clamp(lines as usize)
            .text_size(px(12.))
            .line_height(px(row::PREVIEW_LINE_H))
            .text_color(t.muted_foreground)
            .child(StyledText::new(text).with_highlights(highlights)),
    )
}
