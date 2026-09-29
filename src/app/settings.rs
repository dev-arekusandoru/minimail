//! Settings panel: per-question Auto/Review mode with threshold, and the summaries switch.
//!
//! Edits a local copy of the policy and reports every change; the owner applies it.

use crate::judge::{JudgePolicy, Mode, QuestionKey};
use gpui_kit::{prelude::FluentBuilder as _, *};

/// Key context of the panel.
pub const SETTINGS_CONTEXT: &str = "SettingsPanel";

const BG: u32 = 0x16171a;
const BORDER: u32 = 0x2a2c31;
const ROW_SELECTED: u32 = 0x25272c;
const MUTED: u32 = 0x80838a;
const TEXT: u32 = 0xd9dadd;
const ACCENT: u32 = 0x7dd3a8;

const STEP: f32 = 0.05;

gpui_kit::actions!(
    settings,
    [
        SettingsNext,
        SettingsPrev,
        SettingsToggleMode,
        SettingsThresholdUp,
        SettingsThresholdDown,
        SettingsClose
    ]
);

pub enum SettingsEvent {
    Changed(JudgePolicy, bool),
    Close,
}

pub struct SettingsPanel {
    focus: FocusHandle,
    policy: JudgePolicy,
    summaries: bool,
    /// Rows: one per question, then the summaries switch.
    cursor: usize,
}

impl SettingsPanel {
    pub fn new(policy: JudgePolicy, summaries: bool, cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            policy,
            summaries,
            cursor: 0,
        }
    }

    fn rows() -> usize {
        QuestionKey::ALL.len() + 1
    }

    fn changed(&self, cx: &mut Context<Self>) {
        cx.emit(SettingsEvent::Changed(self.policy.clone(), self.summaries));
        cx.notify();
    }

    fn toggle(&mut self, cx: &mut Context<Self>) {
        match QuestionKey::ALL.get(self.cursor) {
            Some(&key) => self.policy.toggle_mode(key),
            None => self.summaries = !self.summaries,
        }
        self.changed(cx);
    }

    fn nudge(&mut self, delta: f32, cx: &mut Context<Self>) {
        if let Some(&key) = QuestionKey::ALL.get(self.cursor)
            && matches!(self.policy.mode(key), Mode::Auto { .. })
        {
            self.policy.nudge_threshold(key, delta);
            self.changed(cx);
        }
    }
}

impl Focusable for SettingsPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl EventEmitter<SettingsEvent> for SettingsPanel {}

fn row(selected: bool, label: &'static str, value: String) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .px_2()
        .py_1()
        .rounded_sm()
        .text_sm()
        .text_color(rgb(TEXT))
        .when(selected, |el| el.bg(rgb(ROW_SELECTED)).text_color(rgb(ACCENT)))
        .child(label)
        .child(div().text_xs().text_color(rgb(MUTED)).child(SharedString::from(value)))
}

impl Render for SettingsPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let cursor = self.cursor;
        let questions = QuestionKey::ALL.iter().enumerate().map(|(i, &key)| {
            let value = match self.policy.mode(key) {
                Mode::Auto { threshold } => format!("auto ≥ {threshold:.2}"),
                Mode::Review => "review".to_owned(),
            };
            row(i == cursor, key.label(), value).into_any_element()
        });
        div()
            .key_context(SETTINGS_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &SettingsNext, _, cx| {
                this.cursor = (this.cursor + 1).min(Self::rows() - 1);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &SettingsPrev, _, cx| {
                this.cursor = this.cursor.saturating_sub(1);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &SettingsToggleMode, _, cx| this.toggle(cx)))
            .on_action(cx.listener(|this, _: &SettingsThresholdUp, _, cx| this.nudge(STEP, cx)))
            .on_action(cx.listener(|this, _: &SettingsThresholdDown, _, cx| this.nudge(-STEP, cx)))
            .on_action(cx.listener(|_, _: &SettingsClose, _, cx| cx.emit(SettingsEvent::Close)))
            .flex()
            .flex_col()
            .w(px(380.))
            .p_3()
            .gap_1()
            .bg(rgb(BG))
            .border_1()
            .border_color(rgb(BORDER))
            .rounded_md()
            .child(div().text_sm().text_color(rgb(ACCENT)).child("Settings"))
            .child(div().h(px(1.)).bg(rgb(BORDER)))
            .child(div().px_2().text_xs().text_color(rgb(MUTED)).child("Classifier: auto-apply or review"))
            .children(questions)
            .child(div().h(px(1.)).bg(rgb(BORDER)))
            .child(row(
                cursor == QuestionKey::ALL.len(),
                "Thread summaries",
                if self.summaries { "on" } else { "off" }.to_owned(),
            ))
    }
}
