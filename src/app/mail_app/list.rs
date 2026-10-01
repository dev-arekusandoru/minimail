use gpui_kit::component::ActiveTheme as _;
use super::*;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::base::component_traits::Disableable as _;
use gpui_kit::component::empty::{Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyTitle};

impl MailApp {
    /// The list area before any account is linked: what to do next, and the one button to do it.
    fn render_onboarding(&self, cx: &Context<Self>) -> AnyElement {
        let configured = crate::provider::gmail::ClientConfig::from_env().is_some();
        let muted = cx.theme().muted_foreground;
        Empty::new()
            .header(
                EmptyHeader::new()
                    .title(EmptyTitle::new().child("No accounts yet"))
                    .description(EmptyDescription::new().child("Add a Gmail account to see your mail here.")),
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
        let count = self.visible_ids().len();
        let row_count = if self.grouped() { self.rows().len() } else { count };
        self.sync_list(row_count, cx);
        // Near the end of the list, ask the server for the next page of older mail.
        let cursor = self.cursor_ix();
        let near_end = self.list_visible_end.get().saturating_add(LOAD_MORE_MARGIN) >= row_count
            || cursor + LOAD_MORE_MARGIN >= row_count;
        self.maybe_load_older(near_end);
        let selected = self.triage.selected().len();
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
            let title = format!("No mail in {}", self.location_label());
            Empty::new()
                .header(EmptyHeader::new().title(EmptyTitle::new().child(title)))
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
                        self.list_state.clone(),
                        cx.processor(|this, ix: usize, _window, cx| this.render_list_row(ix, cx)),
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
        if self.list_shape.is_none() {
            // Remember where the viewport ends, so a scroll near the last row can
            // ask the server for the next page of older mail.
            let end = self.list_visible_end.clone();
            self.list_state.set_scroll_handler(move |event, _, _| end.set(event.visible_range.end));
        }
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
