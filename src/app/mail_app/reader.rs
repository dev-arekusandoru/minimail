use super::*;

impl MailApp {
    pub(super) fn render_reader(&self, cx: &Context<Self>) -> AnyElement {
        let t = theme::active(cx);
        let stacked = self.panes.orientation() == PaneLayout::Stacked;
        let pane = div().flex_1().min_w_0().min_h_0().flex().flex_col().px_5().py_4()
            .when(!stacked, |d| d.h_full())
            .when(stacked, |d| d.w_full());
        if let Some((handled, secs)) = self.session_end {
            return pane
                .items_center()
                .justify_center()
                .child(SessionCard::finished(handled, secs))
                .into_any_element();
        }
        let Some(msg) = self.opened.and_then(|id| self.mailbox.get(id)) else {
            return pane
                .items_center()
                .justify_center()
                .text_size(px(13.))
                .text_color(t.text_muted)
                .child("Click a message or press enter to open")
                .into_any_element();
        };
        let state = self.mailbox.state_of(msg.id).unwrap_or_default();
        let newest = self.newest();
        let mut thread: Vec<&Message> = self
            .mailbox
            .messages()
            .iter()
            .filter(|m| m.thread_id == msg.thread_id)
            .collect();
        thread.sort_by(|a, b| a.received.cmp(&b.received).then(a.id.cmp(&b.id)));
        let thread_len = thread.len();
        let pos = thread.iter().position(|m| m.id == msg.id).unwrap_or(0);
        let summary = self.summary_shown();
        pane.gap_3()
            .when_some(self.session.as_ref(), |d, s| {
                d.child(SessionCard::new(s.index + 1, s.ids.len()))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .text_size(px(16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(msg.subject.clone()),
                    )
                    .child(
                        div()
                            .px_2()
                            .rounded_sm()
                            .text_xs()
                            .border_1()
                            .border_color(t.state_color(state).opacity(0.6))
                            .text_color(t.state_color(state))
                            .bg(t.state_color(state).opacity(0.12))
                            .child(state.label()),
                    ),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(t.text_muted)
                    .child(format!(
                        "{} <{}> → {} · {}",
                        msg.from_name,
                        msg.from_email,
                        msg.to,
                        Self::clock_label(&msg.received, &newest)
                    )),
            )
            .child(
                div()
                    .id("reader-body")
                    .flex_1()
                    .overflow_y_scroll()
                    .text_size(px(13.))
                    .line_height(relative(1.5))
                    .child(msg.body.clone()),
            )
            .when_some(summary, |d, s| d.child(SummaryCard::new(&s)))
            .when(thread_len > 1, |d| {
                d.child(
                    div()
                        .flex_none()
                        .pt_2()
                        .border_t_1()
                        .border_color(t.border)
                        .flex()
                        .flex_col()
                        .text_size(px(12.))
                        .child(
                            div()
                                .pb_1()
                                .flex()
                                .items_center()
                                .justify_between()
                                .text_size(px(11.))
                                .text_color(t.text_muted)
                                .child(format!("THREAD · {} of {thread_len}", pos + 1))
                                .child(
                                    div()
                                        .flex()
                                        .gap_1()
                                        .child(
                                            button("thread-prev", "‹ prev", "Previous message in thread", "[", cx)
                                                .on_click(run(PrevInThread)),
                                        )
                                        .child(
                                            button("thread-next", "next ›", "Next message in thread", "]", cx)
                                                .on_click(run(NextInThread)),
                                        ),
                                ),
                        )
                        .children(thread.into_iter().map(|m| {
                            let here = m.id == msg.id;
                            div()
                                .h(px(22.))
                                .flex()
                                .items_center()
                                .gap_2()
                                .text_color(if here { t.text } else { t.text_muted })
                                .child(
                                    div()
                                        .w(px(110.))
                                        .flex_none()
                                        .truncate()
                                        .child(m.from_name.clone()),
                                )
                                .child(div().flex_1().truncate().child(m.body.replace('\n', " ")))
                                .child(
                                    div()
                                        .flex_none()
                                        .text_size(px(11.))
                                        .child(Self::clock_label(&m.received, &newest)),
                                )
                        })),
                )
            })
            .into_any_element()
    }
}

/// `Wed Oct 7 08:00` (UTC) for snooze return times.
pub(super) fn format_when(ts: Timestamp) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    let days = ts.div_euclid(DAY);
    let secs = ts.rem_euclid(DAY);
    let weekday = WEEKDAYS[(days + 4).rem_euclid(7) as usize];
    // Civil-from-days (Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    format!(
        "{weekday} {} {day} {:02}:{:02}",
        MONTHS[(month - 1) as usize],
        secs / 3600,
        secs % 3600 / 60
    )
}
