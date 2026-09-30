use super::*;

impl MailApp {
    pub(super) fn render_list(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = theme::active(cx);
        let count = self.visible_ids().len();
        let selected = self.triage.selected().len();
        let title = match &self.mode {
            ListMode::State if self.grouped() => format!(
                "{} · {count} · {} threads",
                self.triage.view.label().to_uppercase(),
                self.rows().iter().filter(|r| !matches!(r, Row::Child { .. })).count()
            ),
            ListMode::State => format!("{} · {count}", self.triage.view.label().to_uppercase()),
            ListMode::Screener => format!("SCREENER · {count}"),
            ListMode::Search(q) => format!("search: {q} · {count}"),
        };
        let header = div()
            .h(px(28.))
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .px_3()
            .text_size(px(11.))
            .text_color(t.text_muted)
            .child(title)
            .when(selected > 0, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_color(t.accent)
                        .child(format!("{selected} selected"))
                        .child(
                            button("btn-clear-selection", "Clear", "Clear selection", "escape", cx)
                                .on_click(run(ClearSelection)),
                        ),
                )
            });
        let body = if count == 0 {
            let view = self.triage.view;
            let empty = match &self.mode {
                ListMode::State => EmptyState::new(view).into_any_element(),
                ListMode::Screener => div().child("No new senders").into_any_element(),
                ListMode::Search(_) => div().child("No matches").into_any_element(),
            };
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(t.text_muted)
                .child(empty)
                .into_any_element()
        } else {
            uniform_list(
                "messages",
                if self.grouped() { self.rows().len() } else { count },
                cx.processor(|this, range: std::ops::Range<usize>, _window, cx| {
                    let grouped = this.grouped();
                    let ids = if grouped { Vec::new() } else { this.visible_ids() };
                    let group_rows = if grouped { this.rows() } else { Vec::new() };
                    let newest = this.newest();
                    let mut rows = Vec::with_capacity(range.len());
                    for ix in range {
                        if grouped {
                            match group_rows.get(ix) {
                                Some(row @ Row::Header { .. }) => {
                                    rows.push(this.render_group_header(row, ix, &newest, cx));
                                }
                                Some(row) => {
                                    if let Some(msg) = this.mailbox.get(row.primary()) {
                                        let child = matches!(row, Row::Child { .. });
                                        let r = this.render_row(msg, ix, &newest, cx);
                                        rows.push(if child { r.pl(px(34.)) } else { r });
                                    }
                                }
                                None => {}
                            }
                        } else if let Some(msg) = ids.get(ix).and_then(|id| this.mailbox.get(*id)) {
                            rows.push(this.render_row(msg, ix, &newest, cx));
                        }
                    }
                    rows
                }),
            )
            .track_scroll(&self.list_scroll)
            .flex_1()
            .into_any_element()
        };
        div()
            .w(px(self.list_w))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .border_r_1()
            .border_color(t.border)
            .child(header)
            .when(self.mode == ListMode::Screener, |d| {
                d.child(ScreenerHeader::new(count))
            })
            .child(body)
            .into_any_element()
    }
}
