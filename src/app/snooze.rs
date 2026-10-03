//! Snooze picker: three presets plus a custom duration input ("3h", "2d", "30m").
//!
//! Holds no business logic: presets and the duration parser are injected, the
//! chosen return time leaves through [`SnoozeEvent`]. Hosted in a kit `Dialog`, which owns
//! the surface, backdrop and `escape`.
use gpui_kit::component::ActiveTheme as _;

use crate::app::ui::{button, shortcut};
use crate::app::chrome::format_time;
use crate::clock::Timestamp;
use crate::tz::Now;
use gpui_kit::{
    component::input::{Input, InputEvent, InputState},
    prelude::FluentBuilder as _,
    *,
};

/// Key context of the picker (bind `1`/`2`/`3`/`4` under `SnoozePicker && !Input`).
pub const SNOOZE_CONTEXT: &str = "SnoozePicker";

use crate::theme::ThemeColor;

gpui_kit::actions!(
    snooze,
    [SnoozePreset1, SnoozePreset2, SnoozePreset3, SnoozeCustom]
);

pub enum SnoozeEvent {
    Pick(Timestamp),
}

/// Parses a custom duration relative to `now` (e.g. `model::parse_snooze`).
pub type ParseSnooze = fn(&str, Timestamp) -> Option<Timestamp>;

pub struct SnoozePicker {
    focus: FocusHandle,
    presets: Vec<(SharedString, Timestamp)>,
    now: Now,
    parse: ParseSnooze,
    input: Entity<InputState>,
    custom: bool,
    invalid: bool,
}

impl SnoozePicker {
    pub fn new(
        presets: impl IntoIterator<Item = (impl Into<SharedString>, Timestamp)>,
        now: Now,
        parse: ParseSnooze,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("3h, 2d, 30m"));
        cx.subscribe(&input, |this, _input, event: &InputEvent, cx| match event {
            InputEvent::PressEnter { .. } => this.submit_custom(cx),
            InputEvent::Change => {
                this.invalid = false;
                cx.notify();
            }
            _ => {}
        })
        .detach();
        Self {
            focus: cx.focus_handle(),
            presets: presets.into_iter().map(|(l, t)| (l.into(), t)).collect(),
            now,
            parse,
            input,
            custom: false,
            invalid: false,
        }
    }

    /// Whether the custom input is showing.
    pub fn is_custom(&self) -> bool {
        self.custom
    }

    fn pick(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some((_, ts)) = self.presets.get(ix) {
            cx.emit(SnoozeEvent::Pick(*ts));
        }
    }

    /// `4`: show the custom duration input and focus it.
    fn open_custom(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.custom = true;
        window.focus(&self.input.focus_handle(cx), cx);
        cx.notify();
    }

    /// Enter in the custom input: parse it and pick, or flag it invalid.
    fn submit_custom(&mut self, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value().to_string();
        match (self.parse)(text.trim(), self.now.at()) {
            Some(ts) => cx.emit(SnoozeEvent::Pick(ts)),
            None => {
                self.invalid = true;
                cx.notify();
            }
        }
    }

    fn row(t: &ThemeColor, key: &str, label: impl IntoElement, detail: impl IntoElement) -> Div {
        let hover = t.list_hover;
        div()
            .flex()
            .items_center()
            .justify_between()
            .px_2()
            .py_1()
            .text_sm()
            .text_color(t.foreground)
            .rounded_sm()
            .hover(move |el| el.bg(hover))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(shortcut(key))
                    .child(label),
            )
            .child(div().text_xs().text_color(t.muted_foreground).child(detail))
    }
}

impl Focusable for SnoozePicker {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl EventEmitter<SnoozeEvent> for SnoozePicker {}

impl Render for SnoozePicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        let invalid = self.invalid;
        let now = &self.now;
        div()
            .key_context(SNOOZE_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &SnoozePreset1, _, cx| this.pick(0, cx)))
            .on_action(cx.listener(|this, _: &SnoozePreset2, _, cx| this.pick(1, cx)))
            .on_action(cx.listener(|this, _: &SnoozePreset3, _, cx| this.pick(2, cx)))
            .on_action(cx.listener(|this, _: &SnoozeCustom, window, cx| this.open_custom(window, cx)))
            .flex()
            .flex_col()
            .w_full()
            .p_2()
            .gap_1()
            .id("snooze-panel")
            .child(div().px_2().text_xs().text_color(t.muted_foreground).child("Snooze until…"))
            .children(self.presets.iter().enumerate().map(|(i, (label, ts))| {
                Self::row(
                    t,
                    &(i + 1).to_string(),
                    label.clone(),
                    SharedString::from(format_time(*ts, now)),
                )
                .id(("snooze-preset", i))
                .test_support()
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| this.pick(i, cx)))
            }))
            .child(
                Self::row(t, "4", "Custom", "")
                    .id("snooze-custom")
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, window, cx| this.open_custom(window, cx))),
            )
            .when(self.custom, |el| {
                el.child(Input::new(&self.input))
                    .child(
                        div().flex().justify_end().child(
                            button("snooze-set", "Set", "Snooze until the typed time", "enter", cx)
                                .on_click(cx.listener(|this, _, _, cx| this.submit_custom(cx))),
                        ),
                    )
                    .when(invalid, |el| {
                        el.child(
                            div()
                                .px_2()
                                .text_xs()
                                .text_color(t.danger)
                                .child("Try 30m, 3h or 2d"),
                        )
                    })
            })
    }
}
