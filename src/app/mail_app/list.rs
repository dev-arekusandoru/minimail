use crate::day_groups::Item;
use gpui_kit::component::ActiveTheme as _;
use super::*;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::base::component_traits::Disableable as _;
use gpui_kit::component::empty::EmptyContent;

impl MailApp {
    /// The list area before any account is linked: what to do next, and the one button to do it.
    fn render_onboarding(&self, cx: &Context<Self>) -> AnyElement {
        let configured = crate::provider::gmail::ClientConfig::load().is_some();
        let muted = cx.theme().muted_foreground;
        crate::app::ui::empty(
            IconName::Mails,
            "No accounts yet",
            "Add a Gmail account to see your mail here.",
        )
        .content(
            EmptyContent::new()
                .child(
                    Button::new("onboarding-add-gmail")
                        .label("Add Gmail account…")
                        .disabled(!configured)
                        .on_click(run(AddGmailAccount)),
                )
                .when(!configured, |c| {
                    c.child(div().text_xs().text_color(muted).child(crate::app::settings::GMAIL_ENV_HINT))
                }),
        )
        .into_any_element()
    }

    pub(super) fn render_list(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let slot = self.slot();
        let model = self.list_model();
        let count = model.items.len();
        let row_count = model.row_count();
        // Mailbox changes that never scroll (sync, snooze expiry) can leave the cursor past the end.
        if self.cursor_ix() > row_count.saturating_sub(1) {
            self.move_cursor(0);
        }
        self.sync_list(count, cx);
        // Near the end of the list, ask the server for the next page of older mail.
        let cursor = self.cursor_ix();
        let near_end = self.view().list_visible_end.get().saturating_add(LOAD_MORE_MARGIN) >= count
            || model.item_of_row(cursor) + LOAD_MORE_MARGIN >= count;
        self.maybe_load_older(near_end);
        let selected = self.triage().selected().len();
        let pills = self.render_pills(cx);
        let t = cx.theme();
        let header = div()
            .flex_none()
            .flex_col()
            .gap_2()
            .px_3()
            .py_2()
            .text_size(px(11.))
            .text_color(t.muted_foreground)
            .child(pills)
            .when(selected > 0, |d| {
                d.child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap_2()
                        .text_color(t.primary)
                        .child(format!("{selected} selected"))
                        .child(
                            button("btn-clear-selection", "Clear", "Clear selection", "escape", cx)
                                .on_click(run(ClearSelection)),
                        )
                        .when(selected >= 2, |d| {
                            d.child(div().flex_none().child(self.menu_trigger(
                                MenuKind::Selection,
                                icon_button(
                                    "btn-selection-more",
                                    IconName::Ellipsis,
                                    "Actions for the selected messages",
                                    "",
                                    cx,
                                ),
                                cx,
                            )))
                        }),
                )
            });
        let body = if self.mailbox.accounts().is_empty() {
            self.render_onboarding(cx)
        } else if count == 0 {
            crate::app::ui::empty(
                IconName::Inbox,
                format!("No mail in {}", self.location_label()),
                "Press / to search all mail, or pick another mailbox in the sidebar.",
            )
            .into_any_element()
        } else {
            div()
                .w_full()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .child(
                    list(
                        self.view().list_state.clone(),
                        cx.processor(move |this, ix: usize, _window, cx| {
                            // Rows are drawn later, outside this panel's render: whose list
                            // they belong to has to come with them.
                            let before = this.enter_slot(slot);
                            let row = this.render_list_row(ix, cx);
                            this.leave_slot(before);
                            row
                        }),
                    )
                    .w_full()
                    .flex_1(),
                )
                .when(self.loading_older(), |d| {
                    d.child(
                        div()
                            .flex_none()
                            .px_3()
                            .py_1()
                            .text_size(px(11.))
                            .text_color(cx.theme().muted_foreground)
                            .child("Loading older mail…"),
                    )
                })
                .into_any_element()
        };
        // The resizable panel owns the list's size; it just fills it.
        div()
            .size_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .child(header)
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
            row_style: self.row_style,
            collapsed_days: self.mailbox.collapsed_days().len(),
            // Sub-pixel pane resizes do not change the layout.
            width: self.list_w.round() as i32,
        };
        let view = self.view_mut();
        if view.list_shape.is_none() {
            // Remember where the viewport ends, so a scroll near the last row can
            // ask the server for the next page of older mail.
            let end = view.list_visible_end.clone();
            view.list_state.set_scroll_handler(move |event, _, _| end.set(event.visible_range.end));
        }
        if view.list_shape == Some(shape) {
            return;
        }
        let same_rows = view.list_shape.is_some_and(|old| old.count == count);
        if same_rows {
            view.list_state.remeasure();
        } else {
            let top = view.list_state.logical_scroll_top();
            view.list_state.reset(count);
            view.list_state.scroll_to(ListOffset {
                item_ix: top.item_ix.min(count),
                offset_in_item: top.offset_in_item,
            });
        }
        view.list_shape = Some(shape);
    }

    /// Item `list_ix` of the list: a day header, a thread header or a message row, measured to
    /// its own content. `ix` handed to the row renderers is the row (cursor) index.
    fn render_list_row(&self, list_ix: usize, cx: &Context<Self>) -> AnyElement {
        let model = self.list_model();
        match model.items.get(list_ix) {
            Some(Item::Header(day)) => self.render_day_header(*day, &model.days[*day], cx),
            Some(Item::Row(ix)) => self.render_list_item(*ix, cx),
            // The list shrank under this index between the layout pass and rendering.
            None => div().into_any_element(),
        }
    }

    /// Row `ix` (cursor space), by density and by kind (message, thread header, thread child).
    fn render_list_item(&self, ix: usize, cx: &Context<Self>) -> AnyElement {
        let newest = self.newest();
        let compact = self.row_style.compact();
        if self.grouped() {
            match self.rows().get(ix) {
                Some(row @ Row::Header { .. }) if compact => {
                    return self.render_compact_header(row, ix, &newest, cx).into_any_element();
                }
                Some(row @ Row::Header { .. }) => {
                    return self.render_group_header(row, ix, &newest, cx).into_any_element();
                }
                Some(row @ Row::Child { .. }) if compact => {
                    return self.render_compact_child(row, ix, &newest, cx).into_any_element();
                }
                Some(row @ Row::Child { .. }) => {
                    return self.render_thread_child(row, ix, &newest, cx).into_any_element();
                }
                Some(Row::Single(id)) => {
                    if let Some(msg) = self.mailbox.get(*id) {
                        return self.render_message_row(msg, ix, &newest, cx);
                    }
                }
                None => {}
            }
        } else if let Some(msg) = self.visible_ids().get(ix).and_then(|id| self.mailbox.get(*id)) {
            return self.render_message_row(msg, ix, &newest, cx);
        }
        div().into_any_element()
    }

    fn render_message_row(&self, msg: &Message, ix: usize, newest: &str, cx: &Context<Self>) -> AnyElement {
        if self.row_style.compact() {
            self.render_compact_row(msg, ix, newest, cx).into_any_element()
        } else {
            self.render_row(msg, ix, newest, cx).into_any_element()
        }
    }
}

/// What the list is currently measured against. A difference in any field means the cached
/// row heights are stale.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct ListShape {
    /// Items the list holds: day headers plus messages, or headers plus their expanded children.
    pub count: usize,
    pub grouped: bool,
    pub preview_lines: u8,
    /// Density, leading element and chips decide every row's height.
    pub row_style: crate::row_style::RowStyle,
    pub collapsed_days: usize,
    /// Pane width in whole pixels.
    pub width: i32,
}
