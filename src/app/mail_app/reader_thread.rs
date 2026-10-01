//! The other messages of the opened thread: the section title and the compact collapsed line.
//! Expanded messages reuse `message_surface`.

use super::super::*;
use super::parts::{Look, Role};
use crate::reading;
use gpui_kit::component::collapsible::Collapsible;

/// Height of a collapsed thread line.
pub(super) const COLLAPSED_H: f32 = 46.;

impl MailApp {
    /// `THREAD · N MORE` with the expand-all / collapse-all toggle (`shift-o`).
    pub(super) fn thread_title(&self, opened: &Message, others: &[MessageId], look: &Look<'_>, cx: &Context<Self>) -> AnyElement {
        let t = &look.t;
        let all_open = others.iter().all(|id| self.reader.is_expanded(opened.thread_id, *id));
        div()
            .h(px(22.))
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .child(look.mono(format!("THREAD · {} MORE", others.len()), t.muted_foreground))
            .child(
                look.link(
                    "reader-thread-toggle",
                    if all_open { "COLLAPSE ALL" } else { "EXPAND ALL" },
                    Some("shift-o"),
                )
                .on_click(cx.listener(|this, _, _, cx| this.toggle_thread_expansion(cx))),
            )
            .into_any_element()
    }

    /// A non-opened thread message: its collapsed line, revealing the full surface when the
    /// app-owned disclosure state says so. No animation, so find-driven expansion stays in step
    /// with the scroll offsets.
    pub(super) fn thread_message(
        &self,
        m: &Message,
        expanded: bool,
        newest: &str,
        look: &Look<'_>,
        cx: &Context<Self>,
    ) -> AnyElement {
        Collapsible::new()
            .open(expanded)
            .child(self.collapsed_line(m, newest, look, cx))
            .content(self.message_surface(m, Role::Thread, look, cx))
            .into_any_element()
    }

    /// One compact line: sender, muted snippet, paperclip when it has attachments, mono date.
    pub(super) fn collapsed_line(&self, m: &Message, newest: &str, look: &Look<'_>, cx: &Context<Self>) -> AnyElement {
        let t = &look.t;
        let mid = m.id;
        let hover = t.list_hover;
        let text = reading::reader_text(m);
        div()
            .id(("reader-thread-msg", m.id as usize))
            .test_support()
            .h(px(COLLAPSED_H))
            .flex()
            .items_center()
            .gap_3()
            .px_3()
            .rounded_md()
            .border_1()
            .border_color(t.border)
            .bg(t.secondary)
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_reader_expanded(mid, cx)))
            .child(
                div()
                    .w(px(150.))
                    .flex_none()
                    .truncate()
                    .text_size(px(13.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(t.foreground)
                    .child(row::sender_label(m)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(12.))
                    .text_color(t.muted_foreground)
                    .child(crate::preview::snippet(&text)),
            )
            .when(!m.attachments.is_empty(), |d| d.child(icons::icon(Glyph::Attachment, t, 13.)))
            .child(look.mono(Self::clock_label(&m.received, newest), t.muted_foreground).flex_none())
            .into_any_element()
    }
}
