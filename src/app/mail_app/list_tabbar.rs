//! The list strip's tab: one tab per open view, named after the location it shows.
//!
//! The dock's list group holds a panel per view and draws each tab from that panel's title,
//! so the reader tab bar's look lives next door (see [`super::reader_tabbar`]) with one
//! difference: a list tab is named by its location alone — "Inbox", "All Inboxes" — the same
//! string the sidebar row shows, and the preview is italic with no close button.

use super::*;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::Sizable as _;

impl MailApp {
    /// The tab the list group draws for the panel showing `query`.
    pub(in crate::app::mail_app) fn list_tab(&self, query: Query, cx: &mut Context<Self>) -> AnyElement {
        let Some(ix) = self.list_tabs.index_of(&query) else { return div().into_any_element() };
        let preview = !self.list_tabs.tabs()[ix].pinned;
        let title = sidebar::row_label(&self.list_views[ix].folder, &self.mailbox);
        let label = div()
            .id(("list-tab", ix))
            .when(preview, |d| d.italic())
            .child(title)
            .test_support()
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.click_list_tab(ix, event.click_count(), event.modifiers().platform, window, cx);
            }));
        let row = div().flex().items_center().gap_1().child(label);
        // The last tab has nothing to fall back to, so it is not offered a close button.
        if self.list_tabs.len() < 2 {
            return row.into_any_element();
        }
        row.child(
            Button::new(("list-tab-close", ix))
                .icon(IconName::Close)
                .ghost()
                .xsmall()
                .w(px(16.))
                .h(px(16.))
                .px_0()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.close_list_tab(ix, window, cx);
                })),
        )
        .into_any_element()
    }
}