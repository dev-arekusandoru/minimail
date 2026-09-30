//! The find bar under the tab bar: input, match count, option toggles and step / close buttons.

use super::super::*;
use super::parts::Look;
use gpui_kit::assets::IconName;
use gpui_kit::component::input::Input;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, Sizable as _};

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

        let button = |id: &'static str, icon: IconName, tip: &'static str, on: bool, action: Box<dyn Action>| {
            let (selection, accent, hover) = (t.selection, t.accent, t.hover);
            div()
                .id(id)
                .test_support()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .w(px(22.))
                .h(px(22.))
                .rounded_sm()
                .cursor_pointer()
                .when(on, move |d| d.bg(selection))
                .hover(move |d| d.bg(hover))
                .tooltip(move |window, cx| Tooltip::new(tip).build(window, cx))
                .child(Icon::new(icon).with_size(px(14.)).text_color(if on { accent } else { t.text_muted }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(move |event, window, cx| run_boxed(action.boxed_clone(), event, window, cx))
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
                .child(button("find-case", IconName::CaseSensitive, "Match case (alt-c)", options.case_sensitive, Box::new(ToggleFindCase)))
                .child(button("find-word", IconName::WholeWord, "Whole word (alt-w)", options.whole_word, Box::new(ToggleFindWord)))
                .child(button("find-regex", IconName::Regex, "Regular expression (alt-r)", options.regex, Box::new(ToggleFindRegex)))
                .child(button("find-prev", IconName::ChevronUp, "Previous match (shift-enter)", false, Box::new(FindPrev)))
                .child(button("find-next", IconName::ChevronDown, "Next match (enter)", false, Box::new(FindNext)))
                .child(button("find-close", IconName::Close, "Close (esc)", false, Box::new(CloseFind)))
                .into_any_element(),
        )
    }
}

/// Dispatch `action` from a button click (`ui::run` takes a concrete action).
fn run_boxed(action: Box<dyn Action>, _: &ClickEvent, window: &mut Window, cx: &mut App) {
    window.dispatch_action(action, cx);
}
