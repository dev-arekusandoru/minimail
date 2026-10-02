//! The reader's tab: one tab per thread, italic while it is the preview.
//!
//! The dock's reader group holds a panel per open thread and draws each tab from that
//! panel's title, so the look the old tab bar had lives here — the monogram, the
//! italic-while-preview title, and our own close button, which stops propagating so
//! closing a tab does not also select it.

use super::super::*;
use super::parts::Look;
use crate::tabs;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::Sizable as _;

/// Longest tab title, in characters.
const TITLE_MAX: usize = 28;
/// Monogram size inside a tab.
const AVATAR: f32 = 16.;

impl MailApp {
    /// The tab the reader group draws for `thread`'s panel.
    pub(in crate::app::mail_app) fn reader_tab(&self, thread: u32, cx: &mut Context<Self>) -> AnyElement {
        let Some(ix) = self.tabs.index_of(thread) else { return div().into_any_element() };
        let tab = &self.tabs.tabs()[ix];
        let messages = self.mailbox.messages();
        // The thread's latest message names it and sends its avatar, like its list header.
        let latest = crate::threads::thread_order(messages, thread)
            .last()
            .and_then(|id| self.mailbox.get(*id))
            .or_else(|| self.mailbox.get(tab.msg));
        let title = SharedString::from(tabs::title(latest.map_or("", |m| m.subject.as_str()), TITLE_MAX));
        let label = div()
            .id(("reader-tab", thread as usize))
            .when(!tab.pinned, |d| d.italic())
            .child(title.clone())
            .test_support()
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.click_tab(ix, event.click_count(), window, cx);
            }));
        let close = Button::new(("reader-tab-close", thread as usize))
            .icon(IconName::Close)
            .ghost()
            .xsmall()
            .w(px(16.))
            .h(px(16.))
            .px_0()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.close_tab(ix, window, cx);
            }));
        // Avatar, title and close share one row inside the tab's padded label box, so
        // both edge insets match and the gaps between the three are equal.
        let look = Look::new(cx);
        div()
            .flex()
            .items_center()
            .gap_1()
            .when(self.tab_avatars, |row| {
                row.child(match latest {
                    Some(m) => div()
                        .id(("reader-tab-avatar", thread as usize))
                        .test_support()
                        .flex_shrink_0()
                        .child(look.monogram(&m.from_name, &m.from_email, AVATAR))
                        .into_any_element(),
                    None => div().into_any_element(),
                })
            })
            .child(label)
            .child(close)
            .into_any_element()
    }
}