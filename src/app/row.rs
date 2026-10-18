//! Shared look of a list row: geometry that depends on the preview setting, the panel width,
//! and cursor / open / selection visual states.
//! [`frame`] keeps those states consistent across message rows and thread headers.

use crate::theme::ThemeColor;
use gpui_kit::{prelude::FluentBuilder as _, *};

/// Height of the sender line and of the subject line.
pub const LINE_H: f32 = 18.;
/// Height of one preview line.
pub const PREVIEW_LINE_H: f32 = 16.;

/// Width of the message list panel for a window `viewport` width, in pixels.
pub fn list_width(viewport: f32) -> f32 {
    (viewport * 0.42).clamp(360., 560.)
}

/// Which independent row states apply.
///
/// `cursor` is the list cursor and `open` the message shown in the reader (the two are kept in
/// step by `follow_cursor`); `selected` marks bulk selection (also the checked dot/tile). Any
/// of the three gives the row the same active background.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RowVisual {
    pub cursor: bool,
    pub open: bool,
    pub selected: bool,
    pub unread: bool,
}

impl RowVisual {
    /// The row carries the active (cursor / open / selected) background.
    pub fn active(self) -> bool {
        self.cursor || self.open || self.selected
    }
}

/// The sender line of a row: outgoing mail is labelled by its recipient instead of "You".
pub fn sender_label(msg: &crate::model::Message) -> String {
    if msg.outgoing {
        format!("To: {}", msg.to)
    } else {
        msg.from_name.clone()
    }
}

/// Rows carry the sender's account icon (in the account color) only in the cross-account views (`All Inboxes`,
/// `All Archive`, ...), where the account is otherwise ambiguous.
pub fn shows_account_icon(location: &crate::model::Location) -> bool {
    use crate::model::Location::*;
    matches!(location, AllInboxes | AllSnoozed | AllSent | AllArchive | AllTrash)
}

/// Background of a row in its resting state (`--rowbg` of the mockups): the active step for the
/// cursor / open row, else the list background. Opaque, so marker and badge cutouts can match.
pub fn row_bg(v: RowVisual, t: &ThemeColor) -> Hsla {
    if v.active() { active_bg(t) } else { t.background }
}

/// Neutral grey one step stronger than hover (hover blended 6% toward the foreground).
pub fn active_bg(t: &ThemeColor) -> Hsla {
    t.list_hover.blend(t.foreground.opacity(0.06))
}

/// The background a hovered row switches to; `None` when the row is already active.
pub fn hover_bg(v: RowVisual, t: &ThemeColor) -> Option<Hsla> {
    (!v.active()).then_some(t.list_hover)
}

/// Apply the row geometry and the visual state of `v` to a row container: background, hover,
/// and joins the `row` hover group the marker cutout follows. Rows are content-sized; the
/// caller adds one `w_full` inner flow (padding 4px top/bottom).
pub fn frame(row: Stateful<Div>, v: RowVisual, t: &ThemeColor) -> crate::app::ui::Observable {
    let hover = hover_bg(v, t);
    row.group(crate::app::row_parts::ROW_GROUP).test_support().relative()
        .w_full()
        .flex()
        .items_center()
        .pl(px(10.))
        .pr_2()
        .border_1()
        .border_color(transparent_black())
        .bg(row_bg(v, t))
        .when_some(hover, |d, bg| d.hover(move |s| s.bg(bg)))
}

