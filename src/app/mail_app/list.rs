use super::*;

impl MailApp {
    pub(super) fn render_list(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = theme::active(cx);
        let count = self.visible_ids().len();
        let row_count = if self.grouped() { self.rows().len() } else { count };
        self.sync_list(row_count, cx);
        let selected = self.triage.selected().len();
        let title = match &self.mode {
            ListMode::State if self.grouped() => format!(
                "{} · {count} · {} threads",
                self.location_label(),
                self.rows().iter().filter(|r| !matches!(r, Row::Child { .. })).count()
            ),
            ListMode::State => {
                format!("{} · {count}", self.location_label())
            }
            ListMode::Search(q) => format!("search: {q} · {count}"),
        };
        let filter_active = self.filter_count() > 0;
        let filter_label = self.filter_label();
        let chips = self.render_chips(cx);
        let header = div()
            .h(px(LIST_HEADER_H))
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .px_3()
            .text_size(px(11.))
            .text_color(t.text_muted)
            .child(div().flex_1().min_w_0().truncate().child(title))
            .when(selected > 0, |d| {
                d.child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap_2()
                        .text_color(t.accent)
                        .child(format!("{selected} selected"))
                        .child(
                            button("btn-clear-selection", "Clear", "Clear selection", "escape", cx)
                                .on_click(run(ClearSelection)),
                        ),
                )
            })
            .child(
                button("btn-filter", filter_label, "Filter mail by tag, kind or account", "", cx)
                    .when(filter_active, |b| b.bg(t.selection).text_color(t.accent))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_menu(MenuKind::Filter, window, cx)
                    })),
            );
        let body = if count == 0 {
            let empty = match &self.mode {
                ListMode::State => div().child(format!("No mail in {}", self.location_label())).into_any_element(),
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
            list(
                self.list_state.clone(),
                cx.processor(|this, ix: usize, _window, cx| this.render_list_row(ix, cx)),
            )
            .w_full()
            .flex_1()
            .into_any_element()
        };
        let stacked = self.panes.orientation() == PaneLayout::Stacked;
        div()
            .flex_none()
            .flex()
            .flex_col()
            // Side by side the divider owns the width, stacked it owns the height;
            // the hairline between the panes is the divider's own.
            .when(stacked, |d| d.w_full().h(px(self.list_h)).min_h_0())
            .when(!stacked, |d| d.w(px(self.list_w)).h_full().min_w_0())
            .child(header)
            .when_some(chips, |d, chips| d.child(chips))
            .child(body)
            .into_any_element()
    }

    /// Bring the list state in line with what the list now holds, so no row keeps a height
    /// measured for an older preview setting, pane width, grouping or row count. Anything
    /// that changes those bumps the shape and is re-measured on the next render.
    fn sync_list(&mut self, count: usize, _cx: &mut Context<Self>) {
        let shape = ListShape {
            count,
            grouped: self.grouped(),
            preview_lines: self.preview_lines,
            // Sub-pixel pane resizes do not change the layout.
            width: self.list_w.round() as i32,
        };
        if self.list_shape == Some(shape) {
            return;
        }
        let same_rows = self.list_shape.is_some_and(|old| old.count == count);
        if same_rows {
            self.list_state.remeasure();
        } else {
            let top = self.list_state.logical_scroll_top();
            self.list_state.reset(count);
            self.list_state.scroll_to(ListOffset {
                item_ix: top.item_ix.min(count),
                offset_in_item: top.offset_in_item,
            });
        }
        self.list_shape = Some(shape);
    }

    /// Row `ix` of the list: a thread header or a message row, measured to its own content.
    fn render_list_row(&self, ix: usize, cx: &Context<Self>) -> AnyElement {
        let newest = self.newest();
        if self.grouped() {
            match self.rows().get(ix) {
                Some(row @ Row::Header { .. }) => {
                    return self.render_group_header(row, ix, &newest, cx).into_any_element();
                }
                Some(row) => {
                    if let Some(msg) = self.mailbox.get(row.primary()) {
                        let child = matches!(row, Row::Child { .. });
                        let r = self.render_row(msg, ix, &newest, cx);
                        let indented = if child { r.pl(px(34.)) } else { r };
                        return indented.into_any_element();
                    }
                }
                None => {}
            }
        } else if let Some(msg) = self.visible_ids().get(ix).and_then(|id| self.mailbox.get(*id)) {
            return self.render_row(msg, ix, &newest, cx).into_any_element();
        }
        // The row list shrank under this index between the layout pass and rendering.
        div().into_any_element()
    }
}

/// What the list is currently measured against. A difference in any field means the cached
/// row heights are stale.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct ListShape {
    /// Rows the list holds: messages, or headers plus their expanded children.
    pub count: usize,
    pub grouped: bool,
    pub preview_lines: u8,
    /// Pane width in whole pixels.
    pub width: i32,
}
