//! Shared look of a list row: geometry that depends on the preview setting, the panel width,
//! and the cursor / open / checked visual states. Message rows and thread headers both use
//! [`frame`] so the states read the same everywhere; each row sets its own height, derived
//! from its content by [`message_row_height`] or [`thread_row_height`].

use crate::app::icons::{self, Glyph};
use crate::theme::Theme;
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

/// Apply the row geometry and the visual states of `v` to a row container. The caller sets the
/// height, so a thread header can be shorter than the message rows around it.
pub fn frame(row: Stateful<Div>, v: RowVisual, t: &Theme) -> crate::app::ui::Observable {
    let hover = t.hover;
    row.test_support().relative()
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
