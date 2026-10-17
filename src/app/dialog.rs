//! Generic keyboard-first choice dialog: a title, a one-line message, and one to
//! five options. The default option is picked by `enter` (shown as an enter keycap); the
//! others keep their digit keys.
//!
//! The hosting kit `Dialog` owns `escape` (cancel). The dialog holds no
//! business logic: it reports the chosen index through [`DialogEvent`] and the
//! owner decides what that index means.
use gpui_kit::component::ActiveTheme as _;

use crate::app::ui::shortcut;
use crate::theme::ThemeColor;
use gpui_kit::{prelude::FluentBuilder as _, *};

/// Key context of a choice dialog. `1`–`5` (non-default options) and `enter` (the default
/// option) are bound under `ChoiceDialog && !Input`.
pub const DIALOG_CONTEXT: &str = "ChoiceDialog";

gpui_kit::actions!(
    dialog,
    [Choice1, Choice2, Choice3, Choice4, Choice5, DialogConfirm]
);

pub enum DialogEvent {
    /// Index into the option list the dialog was built with.
    Choose(usize),
}

/// One option: the key that picks it, a label and an optional detail line.
pub struct DialogOption {
    pub shortcut: SharedString,
    pub label: SharedString,
    pub detail: SharedString,
}

impl DialogOption {
    pub fn new(
        shortcut: &str,
        label: impl Into<SharedString>,
        detail: impl Into<SharedString>,
    ) -> Self {
        Self {
            shortcut: shortcut.to_owned().into(),
            label: label.into(),
            detail: detail.into(),
        }
    }
}

pub struct ChoiceDialog {
    focus: FocusHandle,
    title: SharedString,
    message: SharedString,
    options: Vec<DialogOption>,
    default: usize,
}

impl ChoiceDialog {
    /// A dialog with 1–5 `options`; `default` is the option `enter` picks.
    pub fn new(
        title: impl Into<SharedString>,
        message: impl Into<SharedString>,
        options: Vec<DialogOption>,
        default: usize,
        cx: &mut Context<Self>,
    ) -> Self {
        assert!(
            (1..=5).contains(&options.len()),
            "a choice dialog needs 1–5 options"
        );
        let default = default.min(options.len().saturating_sub(1));
        Self {
            focus: cx.focus_handle(),
            title: title.into(),
            message: message.into(),
            options,
            default,
        }
    }

    pub fn title(&self) -> String {
        self.title.to_string()
    }

    pub fn options(&self) -> Vec<String> {
        self.options
            .iter()
            .map(|o| o.label.to_string())
            .collect()
    }

    /// Digit keys pick every option but the default, which is `enter`'s.
    fn choose_digit(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix != self.default {
            self.choose(ix, cx);
        }
    }

    fn choose(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix < self.options.len() {
            cx.emit(DialogEvent::Choose(ix));
        }
    }

    fn row(t: &ThemeColor, option: &DialogOption, is_default: bool) -> Div {
        let hover = t.list_hover;
        div()
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .py_1()
            .rounded_sm()
            .text_sm()
            .text_color(t.foreground)
            .when(is_default, |d| d.bg(t.list_hover))
            .hover(move |el| el.bg(hover))
            .child(shortcut(if is_default { "enter" } else { &option.shortcut }))

            .child(div().flex_1().child(option.label.clone()))
            .when(!option.detail.is_empty(), |d| {
                d.child(
                    div()
                        .flex_none()
                        .text_xs()
                        .text_color(t.muted_foreground)
                        .child(option.detail.clone()),
                )
            })
    }
}

impl Focusable for ChoiceDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl EventEmitter<DialogEvent> for ChoiceDialog {}

impl Render for ChoiceDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        let default = self.default;
        div()
            .key_context(DIALOG_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Choice1, _, cx| this.choose_digit(0, cx)))
            .on_action(cx.listener(|this, _: &Choice2, _, cx| this.choose_digit(1, cx)))
            .on_action(cx.listener(|this, _: &Choice3, _, cx| this.choose_digit(2, cx)))
            .on_action(cx.listener(|this, _: &Choice4, _, cx| this.choose_digit(3, cx)))
            .on_action(cx.listener(|this, _: &Choice5, _, cx| this.choose_digit(4, cx)))
            .on_action(cx.listener(move |this, _: &DialogConfirm, _, cx| this.choose(default, cx)))
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .w_full()
            .id("choice-dialog")
            .test_support()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.title.clone()),
            )
            .when(!self.message.is_empty(), |d| {
                d.child(
                    div()
                        .text_xs()
                        .text_color(t.muted_foreground)
                        .child(self.message.clone()),
                )
            })
            .children(self.options.iter().enumerate().map(|(i, option)| {
                Self::row(t, option, i == default)
                    .id(("dialog-option", i))
                    .test_support()
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| this.choose(i, cx)))
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_xs()
                    .text_color(t.muted_foreground)
                    .child(shortcut("escape"))
                    .child("cancel"),
            )
    }
}
