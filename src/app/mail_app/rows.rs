use super::*;

impl MailApp {
    pub(super) fn hint_mode(&self) -> HintMode {
        if self.compose.is_some() {
            HintMode::Compose
        } else if self.snooze.is_some() {
            HintMode::Snooze
        } else if self.settings.is_some() {
            HintMode::Settings
        } else if self.rules_panel.is_some() {
            HintMode::Rules
        } else if self.in_session() || self.session_end.is_some() {
            HintMode::Session(self.session_end.is_some())
        } else if self.mode == ListMode::Screener {
            HintMode::Screener
        } else {
            match self.triage.selected().len() {
                0 => HintMode::List,
                n => HintMode::Selection(n),
            }
        }
    }

    pub(super) fn clock_label(received: &str, newest: &str) -> String {
        const MONTHS: [&str; 12] = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];
        let date = received.get(..10).unwrap_or(received);
        if date == newest.get(..10).unwrap_or(newest) {
            return received.get(11..16).unwrap_or("").to_string();
        }
        let month = date
            .get(5..7)
            .and_then(|m| m.parse::<usize>().ok())
            .and_then(|m| MONTHS.get(m.wrapping_sub(1)))
            .unwrap_or(&"?");
        let day = date.get(8..10).and_then(|d| d.parse::<u32>().ok()).unwrap_or(0);
        format!("{month} {day}")
    }

    pub(super) fn newest(&self) -> String {
        self.mailbox
            .messages()
            .iter()
            .map(|m| m.received.as_str())
            .max()
            .unwrap_or("")
            .to_string()
    }

    /// Height of a message row (sender/date, subject, preview): the rows the list measures.
    pub(super) fn message_row_h(&self) -> f32 {
        row::message_row_height(self.preview_lines)
    }

    /// Height of a thread header row: one compact line plus the preview lines it shows.
    pub(super) fn thread_row_h(&self) -> f32 {
        row::thread_row_height(self.preview_lines)
    }

    /// Cursor / open / checked state of the row for message `id`.
    pub fn row_visual(&self, id: MessageId) -> RowVisual {
        RowVisual {
            cursor: self.cursor_id() == Some(id),
            open: self.opened == Some(id),
            checked: self.mode == ListMode::State && self.triage.is_selected(id),
        }
    }

    /// Icons of the right-hand cluster of message `id` that fit the list width: `(shown, hidden)`.
    pub fn row_icons(&self, id: MessageId, width: f32) -> (Vec<Glyph>, Vec<Glyph>) {
        let Some(msg) = self.mailbox.get(id) else {
            return (Vec::new(), Vec::new());
        };
        let pending = self.mailbox.pending(id);
        let return_time_shown = self.triage.view == TriageState::Later && self.mode == ListMode::State;
        let glyphs = icons::glyphs_for(&GlyphInputs {
            tags: self.mailbox.tags(id),
            pending: &pending,
            muted: self.mailbox.is_muted(msg.thread_id),
            new_sender: self.mailbox.is_screened(id),
            snoozed: self.mailbox.snoozed_until(id).is_some() && !return_time_shown,
            attachment: crate::preview::mentions_attachment(&msg.subject, &msg.body),
        });
        icons::split_overflow(&glyphs, icons::max_icons(width))
    }

    pub(super) fn row_click(
        ix: usize,
        cx: &Context<Self>,
    ) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
        cx.listener(move |this, ev: &ClickEvent, window, cx| this.click_row(ix, ev, window, cx))
    }

    /// Message row: sender + date, full-width subject, optional preview, icon cluster.
    pub(super) fn render_row(&self, msg: &Message, ix: usize, newest: &str, cx: &Context<Self>) -> crate::app::ui::Observable {
        let t = theme::active(cx);
        let mut visual = self.row_visual(msg.id);
        visual.cursor = ix == self.cursor_ix();
        let date = match self.mailbox.snoozed_until(msg.id) {
            Some(until) if self.triage.view == TriageState::Later && self.mode == ListMode::State => {
                format!("↩ {}", format_when(until))
            }
            _ => Self::clock_label(&msg.received, newest),
        };
        let pending = self.mailbox.pending(msg.id);
        let unread = !self.read.contains(&msg.id)
            && self.mailbox.state_of(msg.id) == Some(TriageState::Inbox);
        let (shown, hidden) = self.row_icons(msg.id, self.list_w);
        let very_urgent = shown.contains(&Glyph::UrgentHigh);
        let emphasis = if unread || visual.open { FontWeight::SEMIBOLD } else { FontWeight::NORMAL };
        let lines = self.preview_lines;
        let hint = icons::suggestion_summary(&pending);
        let ix_id = msg.id as usize;
        row::frame(div().id(("row", ix_id)).h(px(self.message_row_h())), visual, &t)
            .child(self.row_checkbox(ix, visual.checked, cx))
            .child(row::status_icon(visual.open, unread, very_urgent, &t))
            .child(
                div()
                    .id(("row-body", ix_id))
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .justify_center()
                    .overflow_hidden()
                    .on_click(Self::row_click(ix, cx))
                    .child(
                        div()
                            .h(px(18.))
                            .truncate()
                            .text_color(t.text)
                            .font_weight(emphasis)
                            .child(msg.from_name.clone()),
                    )
                    .child(
                        div()
                            .h(px(18.))
                            .truncate()
                            .text_color(t.text)
                            .font_weight(if unread { FontWeight::MEDIUM } else { FontWeight::NORMAL })
                            .child(msg.subject.clone()),
                    )
                    .when(lines > 0, |d| {
                        d.child(
                            div()
                                .h(px(row::PREVIEW_LINE_H * f32::from(lines)))
                                .line_clamp(lines as usize)
                                .text_size(px(12.))
                                .line_height(px(row::PREVIEW_LINE_H))
                                .text_color(t.text_muted)
                                .child(crate::preview::snippet(&msg.body)),
                        )
                    }),
            )
            .child(
                div()
                    .h_full()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .items_end()
                    .justify_start()
                    .pt(px(3.))
                    .child(
                        div()
                            .id(("row-date", ix_id))
                            .h(px(18.))
                            .flex()
                            .items_center()
                            .text_size(px(11.))
                            .text_color(if unread { t.accent } else { t.text_muted })
                            .on_click(Self::row_click(ix, cx))
                            .child(date),
                    )
                    .child(
                        div()
                            .id(("row-badges", ix_id))
                            .h(px(18.))
                            .flex()
                            .items_center()
                            .when(!pending.is_empty(), |d| {
                                d.on_click(cx.listener(move |this, _: &ClickEvent, w, cx| {
                                    this.suggestions_at(ix, true, w, cx)
                                }))
                                .on_mouse_down(
                                    MouseButton::Right,
                                    cx.listener(move |this, _: &MouseDownEvent, w, cx| {
                                        this.suggestions_at(ix, false, w, cx)
                                    }),
                                )
                            })
                            .child(icons::cluster(&shown, &hidden, &hint, &t, ix_id)),
                    ),
            )
    }
}
