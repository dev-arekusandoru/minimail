//! Snooze picker: three presets plus a custom duration input ("3h", "2d", "30m").
//!
//! Holds no business logic: presets and the duration parser are injected, the
//! chosen return time leaves through [`SnoozeEvent`].

use crate::app::overlay::FitViewport as _;
use crate::app::chrome::format_time;
use crate::clock::Timestamp;
use gpui_kit::{
    component::input::{Input, InputEvent, InputState},
    prelude::FluentBuilder as _,
    *,
};

/// Key context of the picker (bind `1`/`2`/`3`/`4` under `SnoozePicker && !Input`, escape under `SnoozePicker`).
pub const SNOOZE_CONTEXT: &str = "SnoozePicker";

use crate::theme::{self, Theme};

gpui_kit::actions!(
    snooze,
    [SnoozePreset1, SnoozePreset2, SnoozePreset3, SnoozeCustom, SnoozeCancel]
);

pub enum SnoozeEvent {
    Pick(Timestamp),
    Cancel,
}

/// Parses a custom duration relative to `now` (e.g. `model::parse_snooze`).
pub type ParseSnooze = fn(&str, Timestamp) -> Option<Timestamp>;

pub struct SnoozePicker {
    focus: FocusHandle,
    presets: Vec<(SharedString, Timestamp)>,
    now: Timestamp,
    parse: ParseSnooze,
    input: Entity<InputState>,
    custom: bool,
    invalid: bool,
}

impl SnoozePicker {
    pub fn new(
        presets: impl IntoIterator<Item = (impl Into<SharedString>, Timestamp)>,
        now: Timestamp,
        parse: ParseSnooze,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("3h, 2d, 30m"));
        cx.subscribe(&input, |this, input, event: &InputEvent, cx| match event {
            InputEvent::PressEnter { .. } => {
                let text = input.read(cx).value().to_string();
                match (this.parse)(text.trim(), this.now) {
                    Some(ts) => cx.emit(SnoozeEvent::Pick(ts)),
                    None => {
                        this.invalid = true;
                        cx.notify();
                    }
                }
            }
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

    fn row(t: &Theme, key: &str, label: impl IntoElement, detail: impl IntoElement) -> Div {
        let hover = t.hover;
        div()
            .flex()
            .items_center()
            .justify_between()
            .px_2()
            .py_1()
            .text_sm()
            .text_color(t.text)
            .rounded_sm()
            .hover(move |el| el.bg(hover))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(div().text_color(t.accent).child(SharedString::from(key.to_owned())))
                    .child(label),
            )
            .child(div().text_xs().text_color(t.text_muted).child(detail))
    }
}

impl Focusable for SnoozePicker {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl EventEmitter<SnoozeEvent> for SnoozePicker {}

impl Render for SnoozePicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::active(cx);
        let invalid = self.invalid;
        div()
            .key_context(SNOOZE_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &SnoozePreset1, _, cx| this.pick(0, cx)))
            .on_action(cx.listener(|this, _: &SnoozePreset2, _, cx| this.pick(1, cx)))
            .on_action(cx.listener(|this, _: &SnoozePreset3, _, cx| this.pick(2, cx)))
            .on_action(cx.listener(|this, _: &SnoozeCustom, window, cx| {
                this.custom = true;
                window.focus(&this.input.focus_handle(cx), cx);
                cx.notify();
            }))
            .on_action(cx.listener(|_, _: &SnoozeCancel, _, cx| cx.emit(SnoozeEvent::Cancel)))
            .flex()
            .flex_col()
            .fit_viewport(window, 320.)
            .p_2()
            .gap_1()
            .bg(t.surface)
            .border_1()
            .border_color(t.border)
            .rounded_md()
            .id("snooze-panel")
            .overflow_y_scroll()
            .child(div().px_2().text_xs().text_color(t.text_muted).child("Snooze until…"))
            .children(self.presets.iter().enumerate().map(|(i, (label, ts))| {
                Self::row(
                    &t,
                    &(i + 1).to_string(),
                    label.clone(),
                    SharedString::from(format_time(*ts)),
                )
            }))
            .child(Self::row(&t, "4", "Custom", ""))
            .when(self.custom, |el| {
                el.child(Input::new(&self.input))
                    .when(invalid, |el| {
                        el.child(
                            div()
                                .px_2()
                                .text_xs()
                                .text_color(t.error)
                                .child("Try 30m, 3h or 2d"),
                        )
                    })
            })
    }
}
