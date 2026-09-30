//! Settings panel: per-question Auto/Review mode with threshold, and the summaries switch.
//!
//! Edits a local copy of the policy and reports every change; the owner applies it.

use crate::app::overlay::FitViewport as _;
use crate::judge::{JudgePolicy, Mode, QuestionKey};
use crate::theme;
use gpui_kit::{prelude::FluentBuilder as _, *};

/// Key context of the panel.
pub const SETTINGS_CONTEXT: &str = "SettingsPanel";

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
    /// "Group by thread" switched.
    Grouping(bool),
    Close,
}

pub struct SettingsPanel {
    focus: FocusHandle,
    policy: JudgePolicy,
    summaries: bool,
    group: bool,
    /// Rows: one per question, then the summaries switch.
    cursor: usize,
}

impl SettingsPanel {
    pub fn new(policy: JudgePolicy, summaries: bool, group: bool, cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            policy,
            summaries,
            group,
            cursor: 0,
        }
    }

    /// Rows: one per question, the summaries switch, the theme picker, then group-by-thread.
    fn rows() -> usize {
        QuestionKey::ALL.len() + 3
    }

    fn theme_row() -> usize {
        QuestionKey::ALL.len() + 1
    }

    fn group_row() -> usize {
        QuestionKey::ALL.len() + 2
    }

    fn changed(&self, cx: &mut Context<Self>) {
        cx.emit(SettingsEvent::Changed(self.policy.clone(), self.summaries));
        cx.notify();
    }

    /// Switch to the next (`step > 0`) or previous theme, wrapping around.
    fn cycle_theme(&mut self, step: isize, cx: &mut Context<Self>) {
        let names = theme::names(cx);
        if names.is_empty() {
            return;
        }
        let current = theme::active(cx).name.clone();
        let at = names.iter().position(|n| *n == current).unwrap_or(0) as isize;
        let next = (at + step).rem_euclid(names.len() as isize) as usize;
        theme::set_active(cx, &names[next]);
        self.changed(cx);
    }

    fn toggle(&mut self, cx: &mut Context<Self>) {
        if self.cursor == Self::theme_row() {
            self.cycle_theme(1, cx);
            return;
        }
        if self.cursor == Self::group_row() {
            self.group = !self.group;
            cx.emit(SettingsEvent::Grouping(self.group));
            cx.notify();
            return;
        }
        match QuestionKey::ALL.get(self.cursor) {
            Some(&key) => self.policy.toggle_mode(key),
            None => self.summaries = !self.summaries,
        }
        self.changed(cx);
    }

    fn nudge(&mut self, delta: f32, cx: &mut Context<Self>) {
        if self.cursor == Self::theme_row() {
            self.cycle_theme(if delta > 0.0 { 1 } else { -1 }, cx);
        } else if let Some(&key) = QuestionKey::ALL.get(self.cursor)
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

fn row(t: &theme::Theme, selected: bool, label: &'static str, value: String) -> impl IntoElement {
    let hover = t.hover;
    div()
        .flex()
        .items_center()
        .justify_between()
        .px_2()
        .py_1()
        .rounded_sm()
        .text_sm()
        .text_color(t.text)
        .hover(move |el| el.bg(hover))
        .when(selected, |el| el.bg(t.selection).text_color(t.accent))
        .child(label)
        .child(div().text_xs().text_color(t.text_muted).child(SharedString::from(value)))
}

impl Render for SettingsPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::active(cx);
        let cursor = self.cursor;
        let questions = QuestionKey::ALL.iter().enumerate().map(|(i, &key)| {
            let value = match self.policy.mode(key) {
                Mode::Auto { threshold } => format!("auto ≥ {threshold:.2}"),
                Mode::Review => "review".to_owned(),
            };
            row(&t, i == cursor, key.label(), value).into_any_element()
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
            .fit_viewport(window, 380.)
            .p_3()
            .gap_1()
            .bg(t.surface)
            .border_1()
            .border_color(t.border)
            .rounded_md()
            .id("settings-panel")
            .overflow_y_scroll()
            .child(div().text_sm().text_color(t.accent).child("Settings"))
            .child(div().h(px(1.)).bg(t.border))
            .child(div().px_2().text_xs().text_color(t.text_muted).child("Classifier: auto-apply or review"))
            .children(questions)
            .child(div().h(px(1.)).bg(t.border))
            .child(row(
                &t,
                cursor == QuestionKey::ALL.len(),
                "Thread summaries",
                if self.summaries { "on" } else { "off" }.to_owned(),
            ))
            .child(
                div()
                    .id("theme-row")
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.cursor = Self::theme_row();
                        this.cycle_theme(1, cx);
                    }))
                    .child(row(&t, cursor == Self::theme_row(), "Theme", t.name.clone())),
            )
            .child(
                div()
                    .id("group-row")
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.cursor = Self::group_row();
                        this.toggle(cx);
                    }))
                    .child(row(
                        &t,
                        cursor == Self::group_row(),
                        "Group by thread",
                        if self.group { "on" } else { "off" }.to_owned(),
                    )),
            )
    }
}
