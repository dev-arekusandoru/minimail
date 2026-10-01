//! The reader's tab bar: one tab per thread, italic while it is the preview.

use super::super::*;
use super::parts::Look;
use crate::tabs;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::Sizable as _;

/// Longest tab title, in characters.
const TITLE_MAX: usize = 28;
/// Monogram size inside a tab.
const AVATAR: f32 = 16.;

impl MailApp {
    pub(super) fn tab_bar(&self, look: &Look<'_>, cx: &Context<Self>) -> AnyElement {
        let messages = self.mailbox.messages();
        let children = self.tabs.tabs().iter().enumerate().map(|(ix, tab)| {
            // The thread's latest message names it and sends its avatar, like its list header.
            let latest = crate::threads::thread_order(messages, tab.thread)
                .last()
                .and_then(|id| self.mailbox.get(*id))
                .or_else(|| self.mailbox.get(tab.msg));
            let title = SharedString::from(tabs::title(latest.map_or("", |m| m.subject.as_str()), TITLE_MAX));
            let thread = tab.thread as usize;
            let label = div()
                .id(("reader-tab", thread))
                .when(!tab.pinned, |d| d.italic())
                .child(title.clone())
                .test_support();
            let close = Button::new(("reader-tab-close", thread))
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
            Tab::new()
                .aria_label(title)
                .when(self.tab_avatars, |tab| {
                    tab.prefix(match latest {
                        Some(m) => div()
                            .id(("reader-tab-avatar", thread))
                            .test_support()
                            .child(look.monogram(&m.from_name, &m.from_email, AVATAR))
                            .into_any_element(),
                        None => div().into_any_element(),
                    })
                })
                .child(label)
                .suffix(close)
                .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                    this.click_tab(ix, event.click_count(), window, cx);
                }))
        });
        TabBar::new("reader-tabs")
            .selected_index(self.tabs.active_index().unwrap_or(0))
            .children(children)
            .into_any_element()
    }
}
