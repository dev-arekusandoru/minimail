//! The Drafts pane: the list the per-account sidebar row opens.
//!
//! Drafts are not messages, so they get their own row rendering rather than going through the
//! message list. The geometry mirrors a message row (recipient and time, subject, snippet),
//! the same cursor/selection frame is used, and Enter or a click reopens the draft in a
//! composer tab. One account's drafts are shown at a time; the sidebar's count is the same
//! set.
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::prelude::FluentBuilder as _;

use super::*;

impl MailApp {
    /// Ask before deleting the draft under the cursor, the pane's `Delete`/backspace path.
    /// The composer's own discard control goes through the same confirmation.
    pub(super) fn confirm_discard_cursor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(row) = self.cursor_draft() {
            self.confirm_discard(row.id, window, cx);
        }
    }

    /// Open the draft at row `ix` of the pane, moving the cursor there first so keyboard and
    /// mouse leave the same state.
    pub(super) fn open_draft_at(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.draft_rows().into_iter().nth(ix) else {
            return;
        };
        self.draft_cursor = ix;
        self.reopen_draft(row.id, window, cx);
    }

    /// Row `ix` of the Drafts pane: recipient and time, subject with its state, snippet.
    pub(super) fn render_draft_row(&self, ix: usize, cx: &Context<Self>) -> AnyElement {
        let Some(row) = self.draft_rows().into_iter().nth(ix) else {
            return div().into_any_element();
        };
        let t = cx.theme();
        let visual = RowVisual {
            cursor: ix == self.draft_cursor_ix_row(),
            ..Default::default()
        };
        let lines = self.preview_lines;
        let when = super::reader::format_when(row.updated_at, &self.local_now());
        let state_note = row.state_note();
        let click = cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.open_draft_at(ix, window, cx);
        });
        row::frame(
            div().id(("draft-row", ix)).h(px(self.message_row_h())),
            visual,
            t,
        )
        .on_click(click)
        .child(
            div()
                .id(("draft-body", ix))
                .flex_1()
                .min_w_0()
                .h_full()
                .flex()
                .flex_col()
                .justify_center()
                .overflow_hidden()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .text_sm()
                                .child(row.recipient_label().to_owned()),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_xs()
                                .text_color(t.muted_foreground)
                                .child(when),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(row.subject_label().to_owned()),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_xs()
                                .text_color(t.muted_foreground)
                                .child(state_note),
                        ),
                )
                .when(lines > 0 && !row.snippet.is_empty(), |d| {
                    d.child(
                        div()
                            .text_xs()
                            .text_color(t.muted_foreground)
                            .overflow_hidden()
                            .max_h(px(f32::from(lines) * row::PREVIEW_LINE_H))
                            .child(row.snippet.clone()),
                    )
                }),
        )
        .into_any_element()
    }

    /// The pane cursor's clamped index, so a row can tell whether it is the cursor row.
    fn draft_cursor_ix_row(&self) -> usize {
        let len = self.draft_rows().len();
        self.draft_cursor.min(len.saturating_sub(1))
    }
}
