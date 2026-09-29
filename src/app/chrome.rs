//! Stateless chrome: hint bar, help overlay, view tabs, empty state.

use crate::app::actions::commands;
use crate::model::TriageState;
use gpui_kit::{
    prelude::FluentBuilder as _,
    component::{kbd::Kbd, label::Label, separator::Separator},
    *,
};

const BG: u32 = 0x16171a;
const BORDER: u32 = 0x2a2c31;
const ROW_ACTIVE: u32 = 0x25272c;
const MUTED: u32 = 0x80838a;
const TEXT: u32 = 0xd9dadd;
const ACCENT: u32 = 0x7dd3a8;

fn kbd(key: &str) -> Kbd {
    Kbd::new(Keystroke::parse(key).unwrap_or_else(|_| Keystroke::parse("space").unwrap()))
        .appearance(false)
}

fn hint(key: &str, what: &str) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_1()
        .child(kbd(key))
        .child(div().text_xs().text_color(rgb(MUTED)).child(SharedString::from(what.to_owned())))
}

/// Which keys the bottom bar advertises.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HintMode {
    List,
    /// Number of selected messages.
    Selection(usize),
    Reader,
    Compose,
    Palette,
}

#[derive(IntoElement)]
pub struct HintBar {
    mode: HintMode,
}

impl HintBar {
    pub fn new(mode: HintMode) -> Self {
        Self { mode }
    }
}

impl RenderOnce for HintBar {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let hints: &[(&str, &str)] = match self.mode {
            HintMode::List => &[
                ("j", "next"),
                ("k", "prev"),
                ("x", "select"),
                ("e", "done"),
                ("w", "waiting"),
                ("l", "later"),
                ("r", "reply"),
                ("u", "undo"),
                ("cmd-k", "commands"),
                ("?", "help"),
            ],
            HintMode::Selection(_) => &[
                ("x", "toggle"),
                ("shift-j", "extend"),
                ("e", "done"),
                ("w", "waiting"),
                ("l", "later"),
                ("i", "inbox"),
                ("escape", "clear"),
                ("u", "undo"),
            ],
            HintMode::Reader => &[
                ("r", "reply"),
                ("e", "done"),
                ("w", "waiting"),
                ("l", "later"),
                ("j", "next"),
                ("u", "undo"),
            ],
            HintMode::Compose => &[("cmd-enter", "send"), ("escape", "cancel")],
            HintMode::Palette => &[("enter", "run"), ("up", "prev"), ("down", "next"), ("escape", "close")],
        };
        div()
            .flex()
            .items_center()
            .gap_3()
            .px_3()
            .h(px(28.))
            .border_t_1()
            .border_color(rgb(BORDER))
            .when_some(
                match self.mode {
                    HintMode::Selection(n) => Some(n),
                    _ => None,
                },
                |el, n| {
                    el.child(
                        div()
                            .text_xs()
                            .text_color(rgb(ACCENT))
                            .child(format!("{n} selected")),
                    )
                },
            )
            .children(hints.iter().map(|(k, w)| hint(k, w).into_any_element()))
    }
}

#[derive(IntoElement)]
pub struct HelpOverlay;

impl HelpOverlay {
    pub fn new() -> Self {
        Self
    }
}

impl Default for HelpOverlay {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for HelpOverlay {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .w(px(420.))
            .p_3()
            .gap_1()
            .bg(rgb(BG))
            .border_1()
            .border_color(rgb(BORDER))
            .rounded_md()
            .child(div().text_sm().text_color(rgb(ACCENT)).child("Keyboard"))
            .child(Separator::horizontal())
            .children(commands().into_iter().map(|c| {
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_sm()
                    .text_color(rgb(TEXT))
                    .child(Label::new(c.name))
                    .child(kbd(c.key))
            }))
    }
}

#[derive(IntoElement)]
pub struct ViewTabs {
    active: TriageState,
    counts: [(TriageState, usize); 4],
}

impl ViewTabs {
    pub fn new(active: TriageState, counts: [(TriageState, usize); 4]) -> Self {
        Self { active, counts }
    }
}

impl RenderOnce for ViewTabs {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let to_triage = self
            .counts
            .iter()
            .find(|(s, _)| *s == TriageState::Inbox)
            .map_or(0, |(_, n)| *n);
        div()
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .child(
                div()
                    .px_2()
                    .pb_2()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child(format!("{to_triage} to triage")),
            )
            .children(self.counts.iter().enumerate().map(|(i, &(state, n))| {
                let active = state == self.active;
                let name = match state {
                    TriageState::Inbox => "To triage",
                    other => other.label(),
                };
                div()
                    .id(("view-tab", i))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .text_sm()
                    .text_color(rgb(if active { ACCENT } else { TEXT }))
                    .when(active, |el| el.bg(rgb(ROW_ACTIVE)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(kbd(&(i + 1).to_string()))
                            .child(name),
                    )
                    .child(div().text_xs().text_color(rgb(MUTED)).child(n.to_string()))
            }))
    }
}

#[derive(IntoElement)]
pub struct EmptyState {
    view: TriageState,
}

impl EmptyState {
    pub fn new(view: TriageState) -> Self {
        Self { view }
    }
}

impl RenderOnce for EmptyState {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let (title, sub) = match self.view {
            TriageState::Inbox => ("Inbox zero", "Everything is triaged."),
            TriageState::Waiting => ("Nothing waiting", "Replies you send land here."),
            TriageState::Later => ("Nothing for later", "Press l to defer a message."),
            TriageState::Done => ("Nothing done yet", "Press e to finish a message."),
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_1()
            .child(div().text_color(rgb(ACCENT)).child(title))
            .child(div().text_xs().text_color(rgb(MUTED)).child(sub))
    }
}
