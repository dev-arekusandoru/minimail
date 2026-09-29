//! Small presentational panels: rules list, session card, summary card, rule banner, screener header.

use crate::app::chrome::format_time;
use crate::clock::Timestamp;
use crate::model::TriageState;
use crate::rules::Rule;
use crate::summary::ThreadSummary;
use gpui_kit::{
    component::{kbd::Kbd, separator::Separator},
    prelude::FluentBuilder as _,
    *,
};

/// Key context of the rules panel.
pub const RULES_CONTEXT: &str = "RulesPanel";

const BG: u32 = 0x16171a;
const BORDER: u32 = 0x2a2c31;
const ROW_SELECTED: u32 = 0x25272c;
const MUTED: u32 = 0x80838a;
const TEXT: u32 = 0xd9dadd;
const ACCENT: u32 = 0x7dd3a8;

gpui_kit::actions!(rules_panel, [RulesNext, RulesPrev, RulesRevoke, RulesClose]);

fn kbd(key: &str) -> Kbd {
    Kbd::new(Keystroke::parse(key).unwrap_or_else(|_| Keystroke::parse("space").unwrap()))
        .appearance(false)
}

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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
            .on_action(cx.listener(|_, _: &RulesClose, _, cx| cx.emit(RulesEvent::Close)))
            .flex()
            .flex_col()
            .w(px(420.))
            .p_3()
            .gap_1()
            .bg(rgb(BG))
            .border_1()
            .border_color(rgb(BORDER))
            .rounded_md()
            .child(div().text_sm().text_color(rgb(ACCENT)).child("Rules"))
            .child(Separator::horizontal())
            .when(self.rules.is_empty(), |el| {
                el.child(
                    div()
                        .px_2()
                        .text_xs()
                        .text_color(rgb(MUTED))
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
                    .text_color(rgb(TEXT))
                    .when(i == cursor, |el| el.bg(rgb(ROW_SELECTED)).text_color(rgb(ACCENT)))
                    .child(SharedString::from(rule.sender.clone()))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child(SharedString::from(format!("→ {}", state_name(rule.state)))),
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
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        match (self.finished, self.progress) {
            (Some((handled, secs)), _) => div()
                .flex()
                .flex_col()
                .items_center()
                .gap_1()
                .p_6()
                .child(div().text_color(rgb(ACCENT)).child("Session complete"))
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(TEXT))
                        .child(SharedString::from(format!(
                            "{handled} handled · {}m {}s",
                            secs / 60,
                            secs % 60
                        ))),
                )
                .child(div().text_xs().text_color(rgb(MUTED)).child("Press escape to close")),
            (None, progress) => {
                let (pos, total) = progress.unwrap_or((0, 0));
                div()
                    .flex()
                    .items_center()
                    .px_3()
                    .py_1()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child(SharedString::from(format!("{pos}/{total}")))
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
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let list = |title: &'static str, items: Vec<String>| {
            div().when(!items.is_empty(), |el| {
                el.flex()
                    .flex_col()
                    .child(div().text_xs().text_color(rgb(MUTED)).child(title))
                    .children(items.into_iter().map(|i| {
                        div()
                            .text_sm()
                            .text_color(rgb(TEXT))
                            .child(SharedString::from(format!("• {i}")))
                    }))
            })
        };
        div()
            .flex()
            .flex_col()
            .gap_1()
            .p_3()
            .bg(rgb(BG))
            .border_1()
            .border_color(rgb(BORDER))
            .rounded_md()
            .child(div().text_xs().text_color(rgb(ACCENT)).child("Summary"))
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(TEXT))
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
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .px_3()
            .py_1()
            .border_b_1()
            .border_color(rgb(BORDER))
            .text_sm()
            .text_color(rgb(TEXT))
            .child(SharedString::from(format!(
                "Always move mail from {} to {}?",
                self.rule.sender,
                state_name(self.rule.state)
            )))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child(kbd("shift-y"))
                    .child("accept")
                    .child(kbd("shift-n"))
                    .child("dismiss"),
            )
    }
}

/// Header of the screener view.
#[derive(IntoElement)]
pub struct ScreenerHeader {
    count: usize,
}

impl ScreenerHeader {
    pub fn new(count: usize) -> Self {
        Self { count }
    }
}

impl RenderOnce for ScreenerHeader {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .justify_between()
            .px_3()
            .py_1()
            .border_b_1()
            .border_color(rgb(BORDER))
            .text_xs()
            .text_color(rgb(MUTED))
            .child(SharedString::from(format!(
                "Screener · {} from new senders",
                self.count
            )))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(kbd("a"))
                    .child("allow")
                    .child(kbd("b"))
                    .child("block"),
            )
    }
}

/// Row suffix shown in the Later view: `"until Mon 5 Oct 08:00"`.
pub fn snooze_label(until: Timestamp) -> String {
    format!("until {}", format_time(until))
}
