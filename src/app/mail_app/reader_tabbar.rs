//! The reader's tab bar: one tab per thread, italic while it is the preview.

use super::super::*;
use super::parts::Look;
use crate::tabs;
use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::tab::{Tab, TabBar};

/// Longest tab title, in characters.
const TITLE_MAX: usize = 28;
/// Monogram size inside a tab.
const AVATAR: f32 = 16.;

impl MailApp {
    pub(super) fn tab_bar(&self, look: &Look<'_>, cx: &Context<Self>) -> AnyElement {
        let messages = self.mailbox.messages();
        let children = self.tabs.tabs().iter().enumerate().map(|(ix, tab)| {
            // A draft tab names the draft; a thread tab takes the thread's latest message,
            // like its list header.
            let draft = tab.draft_id();
            let latest = tab
                .thread_id()
                .and_then(|thread| {
                    crate::threads::thread_order(messages, thread)
                        .last()
                        .and_then(|id| self.mailbox.get(*id))
                })
                .or_else(|| tab.msg().and_then(|id| self.mailbox.get(id)));
            let subject = match draft {
                Some(id) => self
                    .composer
                    .record(id)
                    .map(|r| r.subject.clone())
                    .unwrap_or_default(),
                None => latest.map_or("", |m| m.subject.as_str()).to_owned(),
            };
            let title = SharedString::from(if draft.is_some() && subject.trim().is_empty() {
                "New message".to_owned()
            } else {
                tabs::title(&subject, TITLE_MAX)
            });
            let thread = tab.thread_id().unwrap_or(u32::MAX) as usize;
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
            // Avatar, title and close share one row inside the tab's padded label box, so
            // both edge insets match and the gaps between the three are equal.
            let row = div()
                .flex()
                .items_center()
                .gap_1()
                .when(self.tab_avatars && draft.is_none(), |row| {
                    row.child(match latest {
                        Some(m) => div()
                            .id(("reader-tab-avatar", thread))
                            .test_support()
                            .flex_shrink_0()
                            .child(look.monogram(&m.from_name, &m.from_email, AVATAR))
                            .into_any_element(),
                        None => div().into_any_element(),
                    })
                })
                .child(label)
                .child(close);
            Tab::new()
                .aria_label(title)
                .child(row)
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
