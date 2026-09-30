//! Shared look of a list row: geometry that depends on the preview setting, the panel width,
//! and the cursor / open / checked visual states. Message rows and thread headers both use
//! [`frame`] so the states read the same everywhere.

use crate::app::icons::{self, Glyph};
use crate::theme::Theme;
use gpui_kit::{prelude::FluentBuilder as _, *};

/// Height of the sender line and of the subject line.
const LINE_H: f32 = 18.;
/// Height of one preview line.
pub const PREVIEW_LINE_H: f32 = 16.;
/// Vertical padding of a row (top + bottom).
const PAD_Y: f32 = 8.;

/// Uniform row height for a `Preview lines` setting (`uniform_list` needs one height).
pub fn row_height(preview_lines: u8) -> f32 {
    PAD_Y + 2. * LINE_H + PREVIEW_LINE_H * f32::from(preview_lines)
}

/// Width of the message list panel for a window `viewport` width, in pixels.
pub fn list_width(viewport: f32) -> f32 {
    (viewport * 0.42).clamp(360., 560.)
}

/// Which of the three independent row states apply.
///
/// - `cursor`: keyboard/mouse focus. Ring in the accent color plus the cursor background.
/// - `open`: message shown in the reader. Persistent tint, solid accent bar on the left edge,
///   open-envelope icon.
/// - `checked`: ticked for a bulk action. Accent wash plus a filled checkbox.
///
/// Cursor and open on the same row combine: ring + bar + open tint.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RowVisual {
    pub cursor: bool,
    pub open: bool,
    pub checked: bool,
}

impl RowVisual {
    pub fn combined(self) -> bool {
        self.cursor && self.open
    }
}

/// Apply the row geometry and the visual states of `v` to a row container.
pub fn frame(row: Stateful<Div>, v: RowVisual, t: &Theme, height: f32) -> Stateful<Div> {
    let hover = t.hover;
    row.relative()
        .h(px(height))
        .w_full()
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .text_size(px(13.))
        .border_1()
        .border_color(if v.cursor { t.accent.opacity(0.75) } else { transparent_black() })
        .when(v.cursor && !v.open, |d| d.bg(t.row_cursor))
        .when(v.open, |d| d.bg(t.row_open))
        .when(v.checked, |d| d.bg(t.accent.opacity(0.16)))
        .when(v.open, |d| {
            d.child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .bottom_0()
                    .w(px(3.))
                    .bg(t.accent),
            )
        })
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
}

/// Leading status slot: open envelope for the open row, closed envelope for unread mail
/// (urgent-tinted when very urgent), empty otherwise.
pub fn status_icon(open: bool, unread: bool, very_urgent: bool, t: &Theme) -> Div {
    let slot = div().w(px(14.)).flex_none().flex().items_center().justify_center();
    if open {
        slot.child(icons::icon(Glyph::Open, t, 13.))
    } else if unread {
        let tint = if very_urgent { t.urgent } else { t.accent };
        slot.child(icons::icon(Glyph::Unread, t, 13.).text_color(tint))
    } else {
        slot
    }
}
