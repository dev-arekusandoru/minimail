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
/// `cursor` is keyboard/mouse focus (ring + cursor background); `open` is shown in the
/// reader with the same selected-bar treatment; `selected` marks bulk selection. Partial
/// thread selection uses a dimmer bar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RowVisual {
    pub cursor: bool,
    pub open: bool,
    pub selected: bool,
    pub partial: bool,
    pub unread: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarState {
    None,
    Selected,
    Partial,
}

impl RowVisual {
    pub fn bar_state(self) -> BarState {
        if self.selected || self.open {
            BarState::Selected
        } else if self.partial {
            BarState::Partial
        } else {
            BarState::None
        }
    }

    pub fn combined(self) -> bool {
        self.cursor && self.open
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

/// Background of a row in its resting state (`--rowbg` of the mockups): cursor, then the
/// open/selected tint (primary at .14 over the list background, opaque so marker and badge
/// cutouts can match it), else the list background.
pub fn row_bg(v: RowVisual, t: &ThemeColor) -> Hsla {
    if v.cursor {
        t.list_active
    } else if v.selected || v.open {
        t.background.blend(t.primary.opacity(0.14))
    } else {
        t.background
    }
}

/// The background a hovered row switches to; `None` when the state already decides it.
pub fn hover_bg(v: RowVisual, t: &ThemeColor) -> Option<Hsla> {
    (!v.cursor && !v.selected && !v.open).then_some(t.list_hover)
}

/// Apply the row geometry and the visual states of `v` to a row container: background, cursor
/// ring, left bar, hover, and joins the `row` hover group the marker cutout follows. Rows are
/// content-sized; the caller adds one `w_full` inner flow (padding 4px top/bottom).
pub fn frame(row: Stateful<Div>, v: RowVisual, t: &ThemeColor) -> crate::app::ui::Observable {
    let bar = match v.bar_state() {
        BarState::None => None,
        BarState::Selected => Some(t.primary),
        BarState::Partial => Some(t.primary.opacity(0.45)),
    };
    let hover = hover_bg(v, t);
    row.group(crate::app::row_parts::ROW_GROUP).test_support().relative()
        .w_full()
        .flex()
        .items_center()
        .pl(px(10.))
        .pr_2()
        .border_1()
        .border_color(if v.cursor { t.primary.opacity(0.8) } else { transparent_black() })
        .bg(row_bg(v, t))
        .when_some(bar, |d, color| {
            d.child(div().absolute().left_0().top_0().bottom_0().w(px(3.)).bg(color))
        })
        .when_some(hover, |d, bg| d.hover(move |s| s.bg(bg)))
}

