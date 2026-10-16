//! The reader's tabs: one per open thread, plus one per open draft, italic while a tab is the
//! preview.
//!
//! The dock's reader group holds a panel per open tab and draws each tab from that panel's
//! title, so the look the old tab bar had lives here — the monogram (or the draft's pen
//! glyph), the italic-while-preview title, and our own close button, which stops propagating
//! so closing a tab does not also select it.

use super::super::*;
use super::parts::Look;
use crate::tabs;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _};

/// Longest tab title, in characters.
const TITLE_MAX: usize = 28;
/// Monogram size inside a tab.
const AVATAR: f32 = 16.;

impl MailApp {
    /// The tab the reader group draws for `thread`'s panel.
    pub(in crate::app::mail_app) fn reader_tab(&self, thread: u32, cx: &mut Context<Self>) -> AnyElement {
        let Some(ix) = self.tabs.index_of(thread) else { return div().into_any_element() };
        self.reader_tab_element(ix, cx)
    }

    /// The tab the reader group draws for `draft`'s panel: the composer's pen glyph and the
    /// draft's own subject (or `New message` while it is blank).
    pub(in crate::app::mail_app) fn reader_draft_tab(
        &self,
        draft: DraftId,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(ix) = self.tabs.index_of_draft(draft) else { return div().into_any_element() };
        self.reader_tab_element(ix, cx)
    }

    /// One reader tab, drawn from the tab at `ix`: a thread tab takes the thread's latest
    /// message, a draft tab its own subject. The id only has to be stable within one tab.
    fn reader_tab_element(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(tab) = self.tabs.tabs().get(ix) else { return div().into_any_element() };
        let draft = tab.draft_id();
        let latest = tab
            .thread_id()
            .and_then(|thread| {
                crate::threads::thread_order(self.mailbox.messages(), thread)
                    .last()
                    .and_then(|id| self.mailbox.get(*id))
            })
            .or_else(|| tab.msg().and_then(|id| self.mailbox.get(id)));
        let subject = match draft {
            Some(id) => self.composer.record(id).map(|r| r.subject.clone()).unwrap_or_default(),
            None => latest.map_or("", |m| m.subject.as_str()).to_owned(),
        };
        let title = SharedString::from(if draft.is_some() && subject.trim().is_empty() {
            "New message".to_owned()
        } else {
            tabs::title(&subject, TITLE_MAX)
        });
        // A thread tab is keyed by its thread so its ids are stable across reordering; a draft
        // has no thread, so it takes `u32::MAX` and the glyph is keyed by slot instead.
        let thread = tab.thread_id().unwrap_or(u32::MAX) as usize;
        let label = div()
            .id(("reader-tab", thread))
            .when(!tab.pinned, |d| d.italic())
            .child(title.clone())
            .test_support()
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.click_tab(ix, event.click_count(), window, cx);
            }));
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
        // The avatar (thread) or draft glyph, title and close share one row inside the tab's
        // padded label box, so both edge insets match and the gaps between them are equal.
        let look = Look::new(cx);
        div()
            .flex()
            .items_center()
            .gap_1()
            .when(self.tab_avatars, |row| {
                row.child(match (draft, latest) {
                    (Some(_), _) => div()
                        .id(("reader-tab-draft", ix))
                        .test_support()
                        .flex_shrink_0()
                        .child(
                            Icon::new(IconName::FilePen)
                                .with_size(px(AVATAR))
                                .text_color(cx.theme().muted_foreground),
                        )
                        .into_any_element(),
                    (None, Some(m)) => div()
                        .id(("reader-tab-avatar", thread))
                        .test_support()
                        .flex_shrink_0()
                        .child(look.monogram(&m.from_name, &m.from_email, AVATAR))
                        .into_any_element(),
                    (None, None) => div().into_any_element(),
                })
            })
            .child(label)
            .child(close)
            .into_any_element()
    }
}
