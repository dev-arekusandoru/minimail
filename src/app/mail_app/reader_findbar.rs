//! The find bar under the tab bar: input, match count, option toggles and step / close buttons.

use super::super::*;
use super::parts::Look;
use crate::app::ui::key_tooltip;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::{Selectable as _, Sizable as _};

impl MailApp {
    /// The bar of the active tab, when its find is open (and no session hides the tabs).
    pub(super) fn find_bar(&self, look: &Look) -> Option<AnyElement> {
        let thread = self.tabs.active()?.thread;
        let ft = self.finds.get(&thread)?;
        let t = &look.t;
        let total = ft.matches.len();
        let empty = ft.find.query.is_empty();
        let (count, problem) = if ft.invalid {
            ("Invalid regex".to_owned(), true)
        } else if empty {
            (String::new(), false)
        } else if total == 0 {
            ("No results".to_owned(), true)
        } else {
            (format!("{}/{total}", ft.find.current(total) + 1), false)
        };
        let options = ft.find.options;

        let button = |id: &'static str,
                      icon: IconName,
                      tip: &'static str,
                      key: &'static str,
                      on: bool,
                      action: Box<dyn Action>| {
            key_tooltip(
                Button::new(id)
                    .icon(icon)
                    .ghost()
                    .xsmall()
                    .h(px(22.))
                    .w(px(22.))
                    .px_0()
                    .selected(on)
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx)),
                tip,
                key,
            )
        };

        Some(
            div()
                .id("find-bar")
                .test_support()
                .key_context(FIND_CONTEXT)
                .flex()
                .flex_none()
                .items_center()
                .gap_1()
                .px_3()
                .py_1()
                .bg(t.surface)
                .border_b_1()
                .border_color(t.border)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .rounded_sm()
                        .border_1()
                        .border_color(if ft.invalid { t.error } else { t.border })
                        .child(Input::new(&ft.input).appearance(false).cleanable(false)),
                )
                .child(
                    div()
                        .id("find-count")
                        .flex_none()
                        .min_w(px(72.))
                        .px_1()
                        .child(look.mono(count, if problem { t.error } else { t.text_muted })),
                )
                .child(button("find-case", IconName::CaseSensitive, "Match case", "alt-c", options.case_sensitive, Box::new(ToggleFindCase)))
                .child(button("find-word", IconName::WholeWord, "Whole word", "alt-w", options.whole_word, Box::new(ToggleFindWord)))
                .child(button("find-regex", IconName::Regex, "Regular expression", "alt-r", options.regex, Box::new(ToggleFindRegex)))
                .child(button("find-prev", IconName::ChevronUp, "Previous match", "shift-enter", false, Box::new(FindPrev)))
                .child(button("find-next", IconName::ChevronDown, "Next match", "enter", false, Box::new(FindNext)))
                .child(button("find-close", IconName::Close, "Close", "escape", false, Box::new(CloseFind)))
                .into_any_element(),
        )
    }
}
