//! Shared look of a list row: geometry that depends on the preview setting, the panel width,
//! and cursor / open / selection visual states.
//! [`frame`] keeps those states consistent across message rows and thread headers.

use crate::theme::{self, ThemeColor};
use gpui_kit::{prelude::FluentBuilder as _, *};

/// Height of the sender line and of the subject line.
pub const LINE_H: f32 = 18.;
/// Height of one preview line.
pub const PREVIEW_LINE_H: f32 = 16.;
/// Vertical padding of a message row (top + bottom).
const PAD_Y: f32 = 8.;
/// Vertical padding of a thread header row: tighter than a message row, it groups
/// messages rather than showing one.
const THREAD_PAD_Y: f32 = 6.;
/// Preview lines a thread header shows at most, however the `Preview lines` setting is set.
const THREAD_PREVIEW_MAX: u8 = 2;

/// Height of a message row for a `Preview lines` setting: sender/date line, subject line and
/// `preview_lines` snippet lines. Rows are measured by the list, so this only has to match
/// what [`frame`] lays out.
pub fn message_row_height(preview_lines: u8) -> f32 {
    PAD_Y + 2. * LINE_H + PREVIEW_LINE_H * f32::from(preview_lines)
}

/// Preview lines a thread header shows for a `Preview lines` setting: the snippet of its
/// newest message, capped so a header stays a header.
pub fn thread_preview_lines(preview_lines: u8) -> u8 {
    preview_lines.min(THREAD_PREVIEW_MAX)
}

/// Height of a thread header row: one compact line (sender, subject, participants, count,
/// date) plus the preview lines [`thread_preview_lines`] allows. Always shorter than
/// [`message_row_height`] for the same setting.
pub fn thread_row_height(preview_lines: u8) -> f32 {
    THREAD_PAD_Y + LINE_H + PREVIEW_LINE_H * f32::from(thread_preview_lines(preview_lines))
}

/// Width of the message list panel for a window `viewport` width, in pixels.
pub fn list_width(viewport: f32) -> f32 {
    (viewport * 0.42).clamp(360., 560.)
}

/// Which independent row states apply.
///
/// `cursor` is keyboard/mouse focus (ring + cursor background); `open` is shown in the
/// reader with the same selected-bar treatment; `selected` marks bulk selection. Partial
/// thread selection uses a dimmer bar; unread uses the unread or urgency color.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RowVisual {
    pub cursor: bool,
    pub open: bool,
    pub selected: bool,
    pub partial: bool,
    pub unread: bool,
    pub urgent: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarState {
    None,
    Unread,
    Urgent,
    Selected,
    Partial,
}

impl RowVisual {
    pub fn bar_state(self) -> BarState {
        if self.selected || self.open {
            BarState::Selected
        } else if self.partial {
            BarState::Partial
        } else if self.unread && self.urgent {
            BarState::Urgent
        } else if self.unread {
            BarState::Unread
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

/// Rows carry the sender's account icon (in the account color) only in the unified `All Inboxes`
/// view, where the account is otherwise ambiguous.
pub fn shows_account_icon(location: &crate::model::Location) -> bool {
    matches!(location, crate::model::Location::AllInboxes)
}

/// Apply the row geometry and the visual states of `v` to a row container. The caller sets the
/// height, so a thread header can be shorter than the message rows around it.
pub fn frame(row: Stateful<Div>, v: RowVisual, t: &ThemeColor) -> crate::app::ui::Observable {
    let hover = t.list_hover;
    let bar = match v.bar_state() {
        BarState::None => None,
        BarState::Unread => Some(theme::unread(t)),
        BarState::Urgent => Some(theme::urgent(t)),
        BarState::Selected => Some(t.primary),
        BarState::Partial => Some(t.primary.opacity(0.45)),
    };
    row.test_support().relative()
        .w_full()
        .flex()
        .items_center()
        .gap_2()
        .pl(px(10.))
        .pr_2()
        .border_1()
        .border_color(if v.cursor { t.primary.opacity(0.8) } else { transparent_black() })
        .when_some(bar, |d, color| {
            d.child(div().absolute().left_0().top_0().bottom_0().w(px(3.)).bg(color))
        })
        .when(v.cursor, |d| d.bg(t.list_active))
        .when((v.selected || v.open) && !v.cursor, |d| d.bg(t.primary.opacity(0.14)))
        .hover(move |s| {
            s.bg(if v.cursor {
                t.list_active
            } else if v.selected || v.open {
                t.primary.opacity(0.14)
            } else {
                hover
            })
        })
}

