//! Small presentational panels: rules list, session card, summary card, rule banner.
use gpui_kit::component::ActiveTheme as _;

use crate::app::chrome::format_time;
use crate::clock::Timestamp;
use crate::model::TriageState;
use crate::rules::Rule;
use crate::app::actions::{AcceptRule, ClearSelection, DismissRule};
use crate::app::ui::{button, run, shortcut};
use crate::summary::ThreadSummary;
use gpui_kit::{
    component::{alert::Alert, progress::Progress, scroll::ScrollableElement as _, separator::Separator},
    prelude::FluentBuilder as _,
    *,
};

/// Key context of the rules panel.
pub const RULES_CONTEXT: &str = "RulesPanel";

use crate::theme;

gpui_kit::actions!(rules_panel, [RulesNext, RulesPrev, RulesRevoke]);

fn state_name(state: TriageState) -> String {
    state.label().to_lowercase()
}

pub enum RulesEvent {
    Revoke(usize),
    Close,
}

/// List of active sender rules with revoke.
pub struct RulesPanel {
    focus: FocusHandle,
    rules: Vec<Rule>,
    cursor: usize,
}

impl RulesPanel {
    pub fn new(rules: Vec<Rule>, cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            rules,
            cursor: 0,
        }
    }

    /// Replace the list (after a revoke); the cursor stays in range.
    pub fn set_rules(&mut self, rules: Vec<Rule>, cx: &mut Context<Self>) {
        self.rules = rules;
        self.cursor = self.cursor.min(self.rules.len().saturating_sub(1));
        cx.notify();
    }
}

impl Focusable for RulesPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl EventEmitter<RulesEvent> for RulesPanel {}

impl Render for RulesPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        let hover = t.list_hover;
        let cursor = self.cursor;
        div()
            .key_context(RULES_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &RulesNext, _, cx| {
                this.cursor = (this.cursor + 1).min(this.rules.len().saturating_sub(1));
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &RulesPrev, _, cx| {
                this.cursor = this.cursor.saturating_sub(1);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &RulesRevoke, _, cx| {
                if this.cursor < this.rules.len() {
                    cx.emit(RulesEvent::Revoke(this.cursor));
                }
            }))
            .flex()
            .flex_col()
            .w_full()
            .max_h(px(f32::from(window.viewport_size().height) * 0.8))
            .p_3()
            .gap_1()
            .id("rules-panel")
            .overflow_y_scrollbar()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_sm().text_color(t.primary).child("Rules"))
                    .child(
                        button("rules-close", "Close", "Close", "escape", cx)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(RulesEvent::Close))),
                    ),
            )
            .child(Separator::horizontal())
            .when(self.rules.is_empty(), |el| {
                el.child(
                    div()
                        .px_2()
                        .text_xs()
                        .text_color(t.muted_foreground)
                        .child("No rules yet. Repeat a sender-wide action to get a suggestion."),
                )
            })
            .children(self.rules.iter().enumerate().map(|(i, rule)| {
                div()
                    .id(("rule", i))
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .text_sm()
                    .text_color(t.foreground)
                    .cursor_pointer()
                    .hover(move |el| el.bg(hover))
                    .when(i == cursor, |el| el.bg(t.list_active).text_color(t.primary))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.cursor = i;
                        cx.notify();
                    }))
                    .child(SharedString::from(rule.sender.clone()))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme::state_color(t, rule.state))
                                    .child(SharedString::from(format!("→ {}", state_name(rule.state)))),
                            )
                            .child(
                                button(("rule-revoke", i), "Revoke", "Revoke rule", "backspace", cx)
                                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(RulesEvent::Revoke(i)))),
                            ),
                    )
            }))
    }
}

/// Session progress, or the end card once finished.
#[derive(IntoElement)]
pub struct SessionCard {
    /// `(position, total)` while running, `(handled, seconds)` once finished.
    progress: Option<(usize, usize)>,
    finished: Option<(usize, Timestamp)>,
}

impl SessionCard {
    /// Progress label `"{position}/{total}"` (position is 1-based).
    pub fn new(position: usize, total: usize) -> Self {
        Self {
            progress: Some((position, total)),
            finished: None,
        }
    }

    /// End card: `"N handled · Mm Ss"`.
    pub fn finished(handled: usize, secs: Timestamp) -> Self {
        Self {
            progress: None,
            finished: Some((handled, secs)),
        }
    }
}

impl RenderOnce for SessionCard {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.theme();
        match (self.finished, self.progress) {
            (Some((handled, secs)), _) => div()
                .flex()
                .flex_col()
                .items_center()
                .gap_1()
                .p_6()
                .child(div().text_color(t.success).child("Session complete"))
                .child(
                    div()
                        .text_sm()
                        .text_color(t.foreground)
                        .child(SharedString::from(format!(
                            "{handled} handled · {}m {}s",
                            secs / 60,
                            secs % 60
                        ))),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .text_xs()
                        .text_color(t.muted_foreground)
                        .child("Press")
                        .child(shortcut("escape"))
                        .child("to close"),
                )
                .child(button("session-close", "Close", "Close session summary", "escape", cx).on_click(run(ClearSelection))),
            (None, progress) => {
                let (pos, total) = progress.unwrap_or((0, 0));
                let pct = if total == 0 { 0. } else { pos as f32 / total as f32 * 100. };
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .py_1()
                    .text_xs()
                    .text_color(t.muted_foreground)
                    .child(div().flex_1().min_w_0().child(
                        Progress::new("session-progress")
                            .value(pct)
                            .accessibility_label(SharedString::from(format!("{pos}/{total}"))),
                    ))
                    .child(button("session-end", "End session", "End the triage session", "escape", cx).on_click(run(ClearSelection)))
            }
        }
    }
}

/// Opt-in thread summary: text, action items, dates.
#[derive(IntoElement)]
pub struct SummaryCard {
    summary: ThreadSummary,
}

impl SummaryCard {
    pub fn new(summary: &ThreadSummary) -> Self {
        Self {
            summary: summary.clone(),
        }
    }
}

impl RenderOnce for SummaryCard {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.theme();
        let list = |title: &'static str, items: Vec<String>| {
            div().when(!items.is_empty(), |el| {
                el.flex()
                    .flex_col()
                    .child(div().text_xs().text_color(t.muted_foreground).child(title))
                    .children(items.into_iter().map(|i| {
                        div()
                            .text_sm()
                            .text_color(t.foreground)
                            .child(SharedString::from(format!("• {i}")))
                    }))
            })
        };
        div()
            .flex()
            .flex_col()
            .gap_1()
            .p_3()
            .bg(t.secondary)
            .border_1()
            .border_color(t.border)
            .rounded_md()
            .child(div().text_xs().text_color(t.info).child("Summary"))
            .child(
                div()
                    .text_sm()
                    .text_color(t.foreground)
                    .child(SharedString::from(self.summary.summary)),
            )
            .child(list("Action items", self.summary.action_items))
            .child(list("Dates", self.summary.dates))
    }
}

/// Banner offering a suggested sender rule (`shift-y` accept, `shift-n` dismiss).
#[derive(IntoElement)]
pub struct RuleBanner {
    rule: Rule,
}

impl RuleBanner {
    pub fn new(rule: &Rule) -> Self {
        Self { rule: rule.clone() }
    }
}

impl RenderOnce for RuleBanner {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.theme();
        div()
            .flex()
            .items_center()
            .gap_3()
            .px_3()
            .py_1()
            .border_b_1()
            .border_color(t.border)
            .text_sm()
            .text_color(t.foreground)
            .child(div().flex_1().min_w_0().child(
                Alert::info(
                    "rule-banner",
                    format!(
                        "Always move mail from {} to {}?",
                        self.rule.sender,
                        state_name(self.rule.state)
                    ),
                )
                .banner(),
            ))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_2()
                    .child(button("rule-accept", "Accept", "Accept rule", "shift-y", cx).on_click(run(AcceptRule)))
                    .child(button("rule-dismiss", "Dismiss", "Dismiss rule", "shift-n", cx).on_click(run(DismissRule))),
            )
    }
}

/// Row suffix shown in the Later view: `"until Mon 5 Oct 08:00"`.
pub fn snooze_label(until: Timestamp) -> String {
    format!("until {}", format_time(until))
}
