//! Small shared pieces of the reader: mono metadata text, keycaps, badges, key-labelled buttons
//! and the thread rail.

use super::super::*;
use crate::app::ui::{mono_font, Observable};
use crate::theme::Theme;
use gpui_kit::component::kbd::Kbd;
use std::sync::Arc;

/// Which surface of the reader a message is drawn as.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Role {
    /// The message the reader is opened on: always fully shown, owns the suggestion strip,
    /// the banners and the `reader-body` id.
    Opened,
    /// Another message of the thread, expanded.
    Thread,
}

/// Per-frame look shared by every reader piece: theme and mono family.
pub(super) struct Look {
    pub t: Arc<Theme>,
    pub mono: SharedString,
}

impl Look {
    pub fn new(cx: &App) -> Self {
        Self { t: theme::active(cx), mono: mono_font(cx) }
    }

    /// One line of mono metadata text.
    pub fn mono(&self, text: impl Into<SharedString>, color: Hsla) -> Div {
        div().font_family(self.mono.clone()).text_size(px(11.)).text_color(color).child(text.into())
    }

    /// A keycap for `key` (`"e"`, `"shift-o"`), drawn like the footer hint bar's.
    pub fn keycap(&self, key: &str) -> Kbd {
        let stroke = Keystroke::parse(key).unwrap_or_else(|_| Keystroke::parse("space").unwrap());
        Kbd::new(stroke).outline().font_family(self.mono.clone()).text_size(px(10.))
    }

    /// Clickable mono text such as `SHOW QUOTED TEXT`, with an optional keycap.
    pub fn link(
        &self,
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        key: Option<&str>,
    ) -> Observable {
        let hover = self.t.text;
        div()
            .id(id)
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .h(px(20.))
            .cursor_pointer()
            .font_family(self.mono.clone())
            .text_size(px(10.5))
            .text_color(self.t.text_muted)
            .hover(move |s| s.text_color(hover))
            .child(label.into())
            .when_some(key, |d, k| d.child(self.keycap(k)))
            .test_support()
    }

    /// Row-badge look (see `icons.rs`): tinted border and fill, mono label, optional glyph.
    pub fn badge(&self, label: impl Into<SharedString>, color: Hsla, glyph: Option<Glyph>) -> Div {
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .h(px(18.))
            .px(px(6.))
            .rounded_sm()
            .border_1()
            .border_color(color.opacity(0.5))
            .bg(color.opacity(0.1))
            .text_color(color)
            .font_family(self.mono.clone())
            .text_size(px(10.5))
            .when_some(glyph, |d, g| d.child(icons::icon(g, &self.t, 11.)))
            .child(label.into())
    }

    /// A button that dispatches `action` and shows its shortcut `key` (empty: none) on its face.
    pub fn action_button<A: Action + Clone>(
        &self,
        id: &'static str,
        label: &'static str,
        tip: &str,
        key: &str,
        action: A,
        cx: &App,
    ) -> Observable {
        button(id, label, tip, key, cx)
            .gap_1()
            .when(!key.is_empty(), |b| b.child(self.keycap(key)))
            .on_click(run(action))
    }
}

/// `2026-09-29 07:41` from an RFC 3339 timestamp, as written (no clock, no zone math).
pub(super) fn stamp(received: &str) -> String {
    match (received.get(..10), received.get(11..16)) {
        (Some(date), Some(time)) => format!("{date} {time}"),
        _ => received.to_owned(),
    }
}

/// Width of the rail gutter left of every surface.
const RAIL_W: f32 = 20.;
/// Diameter of a rail dot, border included.
const DOT: f32 = 11.;

/// One row of the thread rail: a gutter with the vertical line and an optional dot, then
/// `content`. `dot` is `(center y, color)`; rows without a dot (the thread title) just carry the
/// line. The line starts at the first dot and ends at the last one. Only threads get a rail, so
/// there are always at least two rows.
pub(super) fn rail_row(
    t: &Theme,
    dot: Option<(f32, Hsla)>,
    first: bool,
    last: bool,
    content: AnyElement,
) -> Div {
    let center = dot.map_or(0., |(c, _)| c);
    let line = div().absolute().left(px(RAIL_W / 2. - 0.5)).w(px(1.)).bg(t.border);
    let mut gutter = div().relative().flex_none().w(px(RAIL_W)).child(match (first, last) {
        (true, _) => line.top(px(center)).bottom_0(),
        (_, true) => line.top_0().h(px(center)),
        _ => line.top_0().bottom_0(),
    });
    if let Some((c, color)) = dot {
        gutter = gutter.child(
            div()
                .absolute()
                .left(px((RAIL_W - DOT) / 2.))
                .top(px(c - DOT / 2.))
                .w(px(DOT))
                .h(px(DOT))
                .rounded_full()
                .bg(color)
                .border_2()
                .border_color(t.background),
        );
    }
    div()
        .relative()
        .flex()
        .child(gutter)
        .child(div().flex_1().min_w_0().when(!last, |d| d.pb_3()).child(content))
}
