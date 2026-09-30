//! Settings panel: declarative settings rows grouped into searchable sections.

use crate::app::overlay::FitViewport as _;
use crate::app::mail_app::panes::Orientation;
use crate::app::ui::button;
use crate::clock::{Timestamp, DAY};
use crate::judge::{JudgePolicy, Mode, QuestionKey};
use crate::theme;
use gpui_kit::{
    component::input::{Input, InputEvent, InputState},
    component::scroll::ScrollableElement as _,
    prelude::FluentBuilder as _,
    *,
};

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
        SettingsClose,
        SettingsSearch,
        SettingsSectionNext,
        SettingsSectionPrev
    ]
);

pub enum SettingsEvent {
    Changed(JudgePolicy, bool),
    Grouping(bool),
    PreviewLines(u8),
    PaneLayout(Orientation),
    /// Show the action toolbar and banner buttons in the reader.
    ReaderToolbar(bool),
    /// New global follow-up timeout, in seconds (`mailbox.set_follow_up_timeout`).
    FollowUp(Timestamp),
    /// Unblock the sender (`mailbox.unblock_sender`); one undo step.
    Unblock(String),
    Close,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section { General, Appearance, Inbox, Blocked, Classifier, Shortcuts }

impl Section {
    const ALL: [Self; 6] = [Self::General, Self::Appearance, Self::Inbox, Self::Blocked, Self::Classifier, Self::Shortcuts];
    fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Appearance => "Appearance / Theme",
            Self::Inbox => "Inbox & Threads",
            Self::Blocked => "Blocked senders",
            Self::Classifier => "Classifier / AI",
            Self::Shortcuts => "Keyboard / Shortcuts",
        }
    }
}

#[derive(Clone, Copy)]
enum SettingKey {
    Summaries,
    Theme,
    PaneLayout,
    ReaderToolbar,
    Grouping,
    PreviewLines,
    FollowUp,
    /// Blocked sender at this index in [`SettingsPanel::blocked`].
    Blocked(usize),
    Classifier(QuestionKey),
    Threshold(QuestionKey),
    Shortcuts,
}

#[derive(Clone, Copy)]
enum ControlKind { Toggle, Choice(&'static [&'static str]), Stepper { min: u8, max: u8, step: u8 }, Days { min: u8, max: u8 }, Action }

#[derive(Clone, Copy)]
struct SettingSpec {
    section: Section,
    key: SettingKey,
    title: &'static str,
    description: &'static str,
    control: ControlKind,
}

const THEMES_CONTROL: &[&str] = &["theme registry"];
const PREVIEW_OPTIONS: &[&str] = &["Off", "1 line", "2 lines", "3 lines", "4 lines", "5 lines"];
const CLASSIFIER_OPTIONS: &[&str] = &["Auto", "Review"];
const PANE_OPTIONS: &[&str] = &["Side by side", "Stacked"];

fn setting_spec(key: SettingKey) -> SettingSpec {
    match key {
        SettingKey::Summaries => SettingSpec { section: Section::General, key, title: "Thread summaries", description: "Opt in to generated summaries above conversations.", control: ControlKind::Toggle },
        SettingKey::Theme => SettingSpec { section: Section::Appearance, key, title: "Theme", description: "Choose the color scheme used throughout the app.", control: ControlKind::Choice(THEMES_CONTROL) },
        SettingKey::PaneLayout => SettingSpec { section: Section::Appearance, key, title: "Pane layout", description: "Stack the message list and the reader side by side or one above the other.", control: ControlKind::Choice(PANE_OPTIONS) },
        SettingKey::ReaderToolbar => SettingSpec { section: Section::Appearance, key, title: "Reader action toolbar", description: "Show action buttons in the reader. Off: keyboard hints only.", control: ControlKind::Toggle },
        SettingKey::Grouping => SettingSpec { section: Section::Inbox, key, title: "Group by thread", description: "Show one inbox row per conversation instead of per message.", control: ControlKind::Toggle },
        SettingKey::PreviewLines => SettingSpec { section: Section::Inbox, key, title: "Preview lines", description: "Snippet lines shown under each subject in the inbox.", control: ControlKind::Choice(PREVIEW_OPTIONS) },
        SettingKey::FollowUp => SettingSpec { section: Section::Inbox, key, title: "Follow-up after", description: "Wait for a reply before flagging.", control: ControlKind::Days { min: 1, max: 14 } },
        SettingKey::Blocked(_) => SettingSpec { section: Section::Blocked, key, title: "Blocked sender", description: "Mail from this sender is blocked.", control: ControlKind::Action },
        SettingKey::Classifier(question) => SettingSpec { section: Section::Classifier, key, title: question.label(), description: "Choose automatic handling or manual review.", control: ControlKind::Choice(CLASSIFIER_OPTIONS) },
        SettingKey::Threshold(_) => SettingSpec { section: Section::Classifier, key, title: "Confidence threshold", description: "Minimum confidence required for automatic handling.", control: ControlKind::Stepper { min: 0, max: 100, step: 5 } },
        SettingKey::Shortcuts => SettingSpec { section: Section::Shortcuts, key, title: "Keyboard shortcuts", description: "Keyboard navigation and actions for this panel.", control: ControlKind::Action },
    }
}

fn setting_specs() -> impl Iterator<Item = SettingSpec> {
    QuestionKey::ALL.into_iter().flat_map(|key| [
        setting_spec(SettingKey::Classifier(key)),
        setting_spec(SettingKey::Threshold(key)),
    ]).chain([
        setting_spec(SettingKey::Summaries),
        setting_spec(SettingKey::Theme),
        setting_spec(SettingKey::PaneLayout),
        setting_spec(SettingKey::Grouping),
        setting_spec(SettingKey::ReaderToolbar),
        setting_spec(SettingKey::PreviewLines),
        setting_spec(SettingKey::FollowUp),
        setting_spec(SettingKey::Shortcuts),
    ])
}

enum SettingChange { Toggle, Step(f32) }

impl SettingSpec {
    fn get(self, state: &SettingsPanel, active_theme: &str) -> String {
        match self.key {
            SettingKey::Summaries => if state.summaries { "on" } else { "off" }.into(),
            SettingKey::Theme => active_theme.to_owned(),
            SettingKey::Grouping => if state.group { "on" } else { "off" }.into(),
            SettingKey::PaneLayout => state.orientation.label().into(),
            SettingKey::ReaderToolbar => if state.reader_toolbar { "on" } else { "off" }.into(),
            SettingKey::PreviewLines => PREVIEW_OPTIONS[state.preview_lines.min(5) as usize].into(),
            SettingKey::FollowUp => if state.follow_up_days == 1 {
                "1 day".into()
            } else {
                format!("{} days", state.follow_up_days)
            },
            SettingKey::Blocked(_) => String::new(),
            SettingKey::Classifier(question) => match state.policy.mode(question) {
                Mode::Auto { threshold } => format!("auto ≥ {threshold:.2}"),
                Mode::Review => "review".into(),
            },
            SettingKey::Threshold(question) => match state.policy.mode(question) {
                Mode::Auto { threshold } => format!("{:.0}%", threshold * 100.),
                Mode::Review => "—".into(),
            },
            SettingKey::Shortcuts => String::new(),
        }
    }

    fn apply(self, state: &mut SettingsPanel, change: SettingChange, cx: &mut Context<SettingsPanel>) {
        match (self.key, change) {
            (SettingKey::Summaries, SettingChange::Toggle) => {
                state.summaries = !state.summaries;
                state.changed(cx);
            }
            (SettingKey::Theme, SettingChange::Toggle) => state.cycle_theme(1, cx),
            (SettingKey::Theme, SettingChange::Step(delta)) => state.cycle_theme(if delta < 0. { -1 } else { 1 }, cx),
            (SettingKey::PaneLayout, SettingChange::Toggle) => {
                let next = state.orientation.toggled();
                state.orientation = next;
                cx.emit(SettingsEvent::PaneLayout(next));
                cx.notify();
            }
            (SettingKey::ReaderToolbar, SettingChange::Toggle) => {
                state.reader_toolbar = !state.reader_toolbar;
                cx.emit(SettingsEvent::ReaderToolbar(state.reader_toolbar));
                cx.notify();
            }
            (SettingKey::Grouping, SettingChange::Toggle) => {
                state.group = !state.group;
                cx.emit(SettingsEvent::Grouping(state.group));
                cx.notify();
            }
            (SettingKey::PreviewLines, SettingChange::Toggle) => {
                state.preview_lines = (state.preview_lines + 1) % PREVIEW_OPTIONS.len() as u8;
                cx.emit(SettingsEvent::PreviewLines(state.preview_lines));
                cx.notify();
            }
            (SettingKey::FollowUp, SettingChange::Toggle) => state.step_follow_up(1, cx),
            (SettingKey::FollowUp, SettingChange::Step(delta)) => {
                state.step_follow_up(if delta < 0. { -1 } else { 1 }, cx)
            }
            (SettingKey::Blocked(index), SettingChange::Toggle) => state.unblock(index, cx),
            (SettingKey::Classifier(key), SettingChange::Toggle) => {
                state.policy.toggle_mode(key);
                state.changed(cx);
            }
            (SettingKey::Threshold(key), SettingChange::Step(delta))
                if matches!(state.policy.mode(key), Mode::Auto { .. }) =>
            {
                state.policy.nudge_threshold(key, delta);
                state.changed(cx);
            }
            _ => {}
        }
    }
}

pub struct SettingsPanel {
    focus: FocusHandle,
    input: Entity<InputState>,
    policy: JudgePolicy,
    summaries: bool,
    group: bool,
    preview_lines: u8,
    reader_toolbar: bool,
    orientation: Orientation,
    /// Blocked sender addresses, sorted; `Unblock` removes one and emits [`SettingsEvent::Unblock`].
    blocked: Vec<String>,
    /// Unsubscribed senders, read-only: resubscribing happens at the source.
    unsubscribed: Vec<String>,
    /// Days to wait for a reply before flagging a thread (`1..=14`).
    follow_up_days: u8,
    cursor: usize,
    section: Section,
    search: String,
}

impl SettingsPanel {
    pub fn new(
        policy: JudgePolicy,
        summaries: bool,
        group: bool,
        preview_lines: u8,
        orientation: Orientation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search settings…"));
        cx.subscribe(&input, |this, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.search = input.read(cx).value().to_string();
                if !this.search.is_empty() {
                    this.cursor = this.first_search_match();
                }
                cx.notify();
            }
        }).detach();
        Self {
            focus: cx.focus_handle(),
            input,
            policy,
            summaries,
            group,
            preview_lines,
            orientation,
            reader_toolbar: false,
            blocked: Vec::new(),
            unsubscribed: Vec::new(),
            follow_up_days: (crate::model::DEFAULT_FOLLOW_UP_TIMEOUT / DAY).clamp(1, 14) as u8,
            cursor: 0,
            section: Section::Classifier,
            search: String::new(),
        }
    }

    /// Seed the mailbox-backed rows: the blocked-sender list, the read-only unsubscribed list and
    /// the current follow-up timeout (seconds).
    pub fn mailbox_state(
        mut self,
        blocked: Vec<String>,
        unsubscribed: Vec<String>,
        follow_up_timeout: Timestamp,
    ) -> Self {
        self.blocked = blocked;
        self.unsubscribed = unsubscribed;
        self.follow_up_days = (follow_up_timeout.div_euclid(DAY)).clamp(1, 14) as u8;
        self
    }

    /// Seed the current state of the reader action toolbar toggle.
    pub fn reader_toolbar(mut self, on: bool) -> Self {
        self.reader_toolbar = on;
        self
    }

    fn classifier_rows() -> usize { QuestionKey::ALL.len() * 2 }
    fn summary_row() -> usize { Self::classifier_rows() }
    fn theme_row() -> usize { Self::summary_row() + 1 }
    fn pane_row() -> usize { Self::summary_row() + 2 }
    fn reader_toolbar_row() -> usize { Self::summary_row() + 3 }
    fn group_row() -> usize { Self::summary_row() + 4 }
    fn preview_row() -> usize { Self::summary_row() + 5 }
    fn follow_up_row() -> usize { Self::summary_row() + 6 }
    fn blocked_row(index: usize) -> usize { Self::summary_row() + 7 + index }
    /// Total rows of the schema-driven sections (the blocked list adds one row per sender).
    fn rows() -> usize { Self::summary_row() + 7 }

    /// Step the follow-up timeout by `delta` days, clamped to `1..=14`; emits the new timeout.
    fn step_follow_up(&mut self, delta: isize, cx: &mut Context<Self>) {
        let next = (isize::from(self.follow_up_days) + delta).clamp(1, 14) as u8;
        if next == self.follow_up_days {
            return;
        }
        self.follow_up_days = next;
        cx.emit(SettingsEvent::FollowUp(i64::from(next) * DAY));
        cx.notify();
    }

    /// Remove blocked sender `index` from the local list and ask the app to unblock it.
    fn unblock(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.blocked.len() {
            return;
        }
        let email = self.blocked.remove(index);
        if !self.blocked.is_empty() {
            self.cursor = self.cursor.min(Self::blocked_row(self.blocked.len() - 1));
        }
        cx.emit(SettingsEvent::Unblock(email));
        cx.notify();
    }

    fn changed(&self, cx: &mut Context<Self>) {
        cx.emit(SettingsEvent::Changed(self.policy.clone(), self.summaries));
        cx.notify();
    }

    fn cycle_theme(&mut self, step: isize, cx: &mut Context<Self>) {
        let names = theme::names(cx);
        if names.is_empty() { return; }
        let current = theme::active(cx).name.clone();
        let at = names.iter().position(|n| *n == current).unwrap_or(0) as isize;
        let next = (at + step).rem_euclid(names.len() as isize) as usize;
        theme::set_active(cx, &names[next]);
        self.changed(cx);
    }

    fn row_key(row: usize) -> Option<SettingKey> {
        if row < QuestionKey::ALL.len() * 2 {
            let question = QuestionKey::ALL[row / 2];
            return Some(if row.is_multiple_of(2) { SettingKey::Classifier(question) } else { SettingKey::Threshold(question) });
        }
        match row {
            value if value == Self::summary_row() => Some(SettingKey::Summaries),
            value if value == Self::theme_row() => Some(SettingKey::Theme),
            value if value == Self::pane_row() => Some(SettingKey::PaneLayout),
            value if value == Self::reader_toolbar_row() => Some(SettingKey::ReaderToolbar),
            value if value == Self::group_row() => Some(SettingKey::Grouping),
            value if value == Self::preview_row() => Some(SettingKey::PreviewLines),
            value if value == Self::follow_up_row() => Some(SettingKey::FollowUp),
            value if value >= Self::blocked_row(0) => {
                Some(SettingKey::Blocked(value - Self::blocked_row(0)))
            }
            _ => None,
        }
    }

    fn toggle(&mut self, cx: &mut Context<Self>) {
        if let Some(key) = Self::row_key(self.cursor) {
            setting_spec(key).apply(self, SettingChange::Toggle, cx);
        }
    }

    fn nudge(&mut self, delta: f32, cx: &mut Context<Self>) {
        if let Some(key) = Self::row_key(self.cursor) {
            setting_spec(key).apply(self, SettingChange::Step(delta), cx);
        }
    }
    fn move_section(&mut self, delta: isize) {
        let index = Section::ALL.iter().position(|section| *section == self.section).unwrap_or(0) as isize;
        let next = (index + delta).rem_euclid(Section::ALL.len() as isize) as usize;
        self.section = Section::ALL[next];
        self.cursor = match self.section {
            Section::General => Self::summary_row(),
            Section::Appearance => Self::theme_row(),
            Section::Inbox => Self::group_row(),
            Section::Blocked => Self::blocked_row(0),
            Section::Classifier | Section::Shortcuts => 0,
        };
    }

    fn first_search_match(&self) -> usize {
        let query = self.search.to_lowercase();
        for (index, key) in QuestionKey::ALL.iter().enumerate() {
            if query_matches(&query, &[key.label(), "auto apply or review"]) {
                return index * 2;
            }
            if query_matches(&query, &["classifier confidence threshold", "confidence threshold"]) {
                return index * 2 + 1;
            }
        }
        let candidates: [(usize, &[&str]); 8] = [
            (Self::summary_row(), &["thread summaries", "opt in to generated summaries"]),
            (Self::theme_row(), &["theme", "appearance", "color scheme"]),
            (Self::group_row(), &["group by thread", "inbox", "threads"]),
            (Self::preview_row(), &["preview lines", "message snippet", "inbox"]),
            (Self::follow_up_row(), &["follow-up after", "follow up", "wait for a response", "flag"]),
            (Self::blocked_row(0), &["blocked senders", "blocked", "unblock", "unsubscribed", "resubscribe"]),
            (Self::pane_row(), &["pane layout", "side by side", "stacked", "layout", "appearance"]),
            (Self::reader_toolbar_row(), &["reader action toolbar", "action buttons", "toolbar", "reader", "keyboard hints"]),
        ];
        candidates.into_iter().find(|(_, terms)| query_matches(&query, terms)).map_or(0, |(index, _)| index)
    }
    fn move_row(&mut self, delta: isize) {
        if !self.search.is_empty() {
            let last = Self::blocked_row(self.blocked.len()).saturating_sub(1);
            self.cursor = (self.cursor as isize + delta).clamp(0, last as isize) as usize;
            return;
        }
        let (start, count) = match self.section {
            Section::General => (Self::summary_row(), 1),
            Section::Appearance => (Self::theme_row(), 3),
            Section::Inbox => (Self::group_row(), 3),
            Section::Blocked => (Self::blocked_row(0), self.blocked.len()),
            Section::Classifier => (0, QuestionKey::ALL.len() * 2),
            Section::Shortcuts => (0, 0),
        };
        if count > 0 {
            let end = start + count - 1;
            self.cursor = (self.cursor as isize + delta).clamp(start as isize, end as isize) as usize;
        }
    }

}


impl Focusable for SettingsPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl EventEmitter<SettingsEvent> for SettingsPanel {}

fn row(
    t: &theme::Theme,
    selected: bool,
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
    value: String,
) -> Div {
    let hover = t.hover;
    div().flex().items_center().justify_between().px_2().py_1().rounded_sm().text_sm()
        .text_color(t.text).hover(move |el| el.bg(hover))
        .when(selected, |el| el.bg(t.selection).text_color(t.accent))
        .child(div().flex().flex_col().min_w_0().child(title.into())
            .child(div().text_xs().text_color(t.text_muted).truncate().child(description.into())))
        .child(div().text_xs().text_color(t.text_muted).child(SharedString::from(value)))
}

fn setting_index(key: SettingKey) -> usize {
    match key {
        SettingKey::Classifier(question) => QuestionKey::ALL.iter().position(|key| *key == question).unwrap_or(0) * 2,
        SettingKey::Threshold(question) => QuestionKey::ALL.iter().position(|key| *key == question).unwrap_or(0) * 2 + 1,
        SettingKey::Summaries => SettingsPanel::summary_row(),
        SettingKey::Theme => SettingsPanel::theme_row(),
        SettingKey::PaneLayout => SettingsPanel::pane_row(),
        SettingKey::ReaderToolbar => SettingsPanel::reader_toolbar_row(),
        SettingKey::Grouping => SettingsPanel::group_row(),
        SettingKey::PreviewLines => SettingsPanel::preview_row(),
        SettingKey::FollowUp => SettingsPanel::follow_up_row(),
        SettingKey::Blocked(index) => SettingsPanel::blocked_row(index),
        SettingKey::Shortcuts => SettingsPanel::rows(),
    }
}

fn render_setting(
    spec: SettingSpec,
    state: &SettingsPanel,
    t: &theme::Theme,
    cx: &mut Context<SettingsPanel>,
) -> AnyElement {
    match spec.control {
        ControlKind::Toggle => render_toggle(spec, state, t, cx),
        ControlKind::Choice(options) => render_choice(spec, options, state, t, cx),
        ControlKind::Stepper { min, max, step } => render_stepper(spec, min, max, step, state, t, cx),
        ControlKind::Days { min, max } => render_days(spec, min, max, state, t, cx),
        ControlKind::Action => render_action(spec, t),
    }
}

fn render_toggle(spec: SettingSpec, state: &SettingsPanel, t: &theme::Theme, cx: &mut Context<SettingsPanel>) -> AnyElement {
    render_clickable(spec, state, t, cx)
}

fn render_choice(
    spec: SettingSpec,
    options: &'static [&'static str],
    state: &SettingsPanel,
    t: &theme::Theme,
    cx: &mut Context<SettingsPanel>,
) -> AnyElement {
    let _declared_options = options;
    let mut element = render_clickable(spec, state, t, cx);
    if matches!(spec.key, SettingKey::Theme) {
        element = div().flex().items_center().gap_2().child(element)
            .child(div().flex().gap_1()
                .child(div().w(px(9.)).h(px(9.)).rounded_full().bg(t.surface))
                .child(div().w(px(9.)).h(px(9.)).rounded_full().bg(t.accent))
                .child(div().w(px(9.)).h(px(9.)).rounded_full().bg(t.selection)))
            .into_any_element();
    }
    element
}

fn render_clickable(
    spec: SettingSpec,
    state: &SettingsPanel,
    t: &theme::Theme,
    cx: &mut Context<SettingsPanel>,
) -> AnyElement {
    let index = setting_index(spec.key);
    let value = spec.get(state, &t.name);
    let selected = state.cursor == index;
    let click = cx.listener(move |this, _, _, cx| {
        this.cursor = index;
        setting_spec(spec.key).apply(this, SettingChange::Toggle, cx);
    });
    let content = row(t, selected, spec.title, spec.description, value);
    match spec.key {
        SettingKey::Classifier(question) => {
            let ix = QuestionKey::ALL.iter().position(|key| *key == question).unwrap_or(0);
            div().id(("question-row", ix)).cursor_pointer().on_click(click).child(content).test_support().into_any_element()
        }
        SettingKey::Summaries => div().id("summaries-row").cursor_pointer().on_click(click).child(content).test_support().into_any_element(),
        SettingKey::Theme => div().id("theme-row").cursor_pointer().on_click(click).child(content).test_support().into_any_element(),
        SettingKey::PaneLayout => div().id("pane-layout-row").cursor_pointer().on_click(click).child(content).test_support().into_any_element(),
        SettingKey::ReaderToolbar => div().id("reader-toolbar-row").cursor_pointer().on_click(click).child(content).test_support().into_any_element(),
        SettingKey::Grouping => div().id("group-row").cursor_pointer().on_click(click).child(content).test_support().into_any_element(),
        SettingKey::PreviewLines => div().id("preview-lines-row").cursor_pointer().on_click(click).child(content).test_support().into_any_element(),
        SettingKey::FollowUp | SettingKey::Blocked(_) | SettingKey::Threshold(_) | SettingKey::Shortcuts => {
            content.into_any_element()
        }
    }
}

fn render_stepper(
    spec: SettingSpec,
    min: u8,
    max: u8,
    step: u8,
    state: &SettingsPanel,
    t: &theme::Theme,
    cx: &mut Context<SettingsPanel>,
) -> AnyElement {
    let SettingKey::Threshold(question) = spec.key else { return render_action(spec, t); };
    let ix = QuestionKey::ALL.iter().position(|key| *key == question).unwrap_or(0);
    let index = setting_index(spec.key);
    let can_step = matches!(state.policy.mode(question), Mode::Auto { .. });
    let description: SharedString = format!("{} · range {min}–{max}%, step {step}%", spec.description).into();
    div().flex().items_center().gap_1()
        .child(div().id(("threshold-row", ix)).test_support().flex_1()
            .child(row(t, state.cursor == index, spec.title, description, spec.get(state, &t.name))))
        .when(can_step, |el| el
            .child(button(("threshold-down", ix), "−", "Lower threshold", "-", cx).on_click(cx.listener(move |this, _, _, cx| {
                this.cursor = index;
                setting_spec(spec.key).apply(this, SettingChange::Step(-(step as f32 / 100.)), cx);
            })))
            .child(button(("threshold-up", ix), "+", "Raise threshold", "=", cx).on_click(cx.listener(move |this, _, _, cx| {
                this.cursor = index;
                setting_spec(spec.key).apply(this, SettingChange::Step(step as f32 / 100.), cx);
            }))))
        .into_any_element()
}

/// `Follow-up after` stepper: `−`/`+` move one day within `min..=max`, clamped.
fn render_days(
    spec: SettingSpec,
    min: u8,
    max: u8,
    state: &SettingsPanel,
    t: &theme::Theme,
    cx: &mut Context<SettingsPanel>,
) -> AnyElement {
    let index = setting_index(spec.key);
    let description: SharedString = format!("{} · {min}–{max} days", spec.description).into();
    div().flex().items_center().gap_1()
        .child(div().id("follow-up-row").test_support().flex_1()
            .child(row(t, state.cursor == index, spec.title, description, spec.get(state, &t.name))))
        .child(button("follow-up-down", "−", "One day less", "-", cx)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.cursor = index;
                setting_spec(spec.key).apply(this, SettingChange::Step(-1.), cx);
            })))
        .child(button("follow-up-up", "+", "One day more", "=", cx)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.cursor = index;
                setting_spec(spec.key).apply(this, SettingChange::Step(1.), cx);
            })))
        .into_any_element()
}

fn render_action(spec: SettingSpec, t: &theme::Theme) -> AnyElement {
    div().id("settings-shortcuts").test_support().flex().flex_col().gap_1()
        .child(row(t, false, spec.title, spec.description, String::new()))
        .child(div().text_sm().text_color(t.text_muted).child("j / ↓ — next · k / ↑ — previous"))
        .child(div().text_sm().text_color(t.text_muted).child("Space / Enter — change value"))
        .child(div().text_sm().text_color(t.text_muted).child("Ctrl-Tab / Ctrl-Shift-Tab — switch section"))
        .child(div().text_sm().text_color(t.text_muted).child("/ — focus search · Escape — clear, then close"))
        .into_any_element()
}

impl Render for SettingsPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::active(cx);
        let query = self.search.trim().to_lowercase();
        let searching = !query.is_empty();
        let matches = |spec: &SettingSpec| !searching
            || spec.title.to_lowercase().contains(&query)
            || spec.description.to_lowercase().contains(&query);
        let blocked_match = query.is_empty()
            || query_matches(&query, &["blocked", "unblock", "unsubscribed", "resubscribe", "senders"]);
        let visible: Vec<Section> = Section::ALL.into_iter().filter(|section| match section {
            Section::Blocked => blocked_match,
            _ => setting_specs().any(|spec| spec.section == *section && matches(&spec)),
        }).collect();
        let sections: Vec<_> = visible.iter()
            .map(|section| section_nav(*section, *section == self.section, cx).into_any_element()).collect();
        let narrow_sections: Vec<_> = visible.iter()
            .map(|section| section_nav(*section, *section == self.section, cx).into_any_element()).collect();
        let filtered: Vec<_> = setting_specs().filter(|spec| {
            matches(spec) && (searching || spec.section == self.section)
        }).map(|spec| render_setting(spec, self, &t, cx)).collect();
        let blocked = (if searching { blocked_match } else { self.section == Section::Blocked })
            .then(|| render_blocked(self, &t, cx));
        let no_results = searching
            && !blocked_match
            && !setting_specs().any(|spec| matches(&spec));
        let narrow = f32::from(window.viewport_size().width) < 620.;
        div().key_context(SETTINGS_CONTEXT).track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &SettingsNext, _, cx| { this.move_row(1); cx.notify(); }))
            .on_action(cx.listener(|this, _: &SettingsPrev, _, cx| { this.move_row(-1); cx.notify(); }))
            .on_action(cx.listener(|this, _: &SettingsToggleMode, _, cx| this.toggle(cx)))
            .on_action(cx.listener(|this, _: &SettingsThresholdUp, _, cx| this.nudge(STEP, cx)))
            .on_action(cx.listener(|this, _: &SettingsThresholdDown, _, cx| this.nudge(-STEP, cx)))
            .on_action(cx.listener(|this, _: &SettingsSearch, window, cx| window.focus(&this.input.focus_handle(cx), cx)))
            .on_action(cx.listener(|this, _: &SettingsSectionNext, _, cx| { this.move_section(1); cx.notify(); }))
            .on_action(cx.listener(|this, _: &SettingsSectionPrev, _, cx| { this.move_section(-1); cx.notify(); }))
            .on_action(cx.listener(|this, _: &SettingsClose, window, cx| {
                if !this.search.is_empty() {
                    this.search.clear();
                    this.input.update(cx, |input, cx| input.set_value("", window, cx));
                    cx.notify();
                } else { cx.emit(SettingsEvent::Close); }
            }))
            .flex().flex_col().fit_viewport(window, 760.).p_3().gap_2()
            .bg(t.surface).border_1().border_color(t.border).rounded_md().id("settings-panel").overflow_y_scrollbar()
            .child(div().flex().items_center().justify_between().child(div().text_sm().text_color(t.accent).child("Settings"))
                .child(button("settings-close", "Close", "Close", "escape", cx).on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::Close)))))
            .child(div().id("settings-search").test_support().child(Input::new(&self.input).appearance(false)))
            .when(narrow, |el| el.child(div().flex().flex_row().gap_1().children(narrow_sections)))
            .child(div().flex().gap_3().flex_1()
                .when(!narrow, |el| el.child(div().w(px(190.)).flex().flex_col().gap_1().children(sections)))
                .child(div().flex_1().flex().flex_col().gap_1().overflow_y_scrollbar()
                    .child(div().text_sm().text_color(t.accent).child(if searching { "Search results" } else { self.section.label() }))
                    .child(div().h(px(1.)).bg(t.border))
                    .children(filtered)
                    .when_some(blocked, |el, block| el.child(block))
                    .when(no_results, |el| el.child(div().text_sm().text_color(t.text_muted).child("No matching settings"))))
            )
            .child(div().h(px(1.)).bg(t.border))
            .child(div().text_xs().text_color(t.text_muted).child("↑/↓ or j/k move · Space change · Ctrl-Tab sections · / search · Esc clear / close"))
    }
}

/// Blocked senders with an `Unblock` control each, then the read-only unsubscribed list.
fn render_blocked(state: &SettingsPanel, t: &theme::Theme, cx: &mut Context<SettingsPanel>) -> AnyElement {
    let mut list = div().flex().flex_col().gap_1();
    if state.blocked.is_empty() {
        list = list.child(
            div().id("blocked-empty").test_support().text_sm().text_color(t.text_muted)
                .child("No blocked senders."),
        );
    }
    for (index, email) in state.blocked.iter().enumerate() {
        let row_index = SettingsPanel::blocked_row(index);
        let selected = state.cursor == row_index;
        let unblock = cx.listener(move |this, _, _, cx| {
            this.cursor = row_index;
            setting_spec(SettingKey::Blocked(index)).apply(this, SettingChange::Toggle, cx);
        });
        list = list.child(
            div().id(("blocked-row", index)).test_support().flex().items_center().gap_2()
                .child(div().flex_1().child(row(t, selected, email.clone(), "Blocked sender", String::new())))
                .child(button(("blocked-unblock", index), "Unblock", "Allow mail from this sender again", "", cx).on_click(unblock)),
        );
    }
    list = list.child(
        div().text_sm().text_color(t.accent).child("Unsubscribed"),
    );
    list = list.child(
        div().text_xs().text_color(t.text_muted)
            .child("Read-only: resubscribing happens at the original subscription."),
    );
    if state.unsubscribed.is_empty() {
        list = list.child(div().id("unsubscribed-empty").test_support().text_sm().text_color(t.text_muted).child("None."));
    }
    for (index, email) in state.unsubscribed.iter().enumerate() {
        list = list.child(
            div().id(("unsubscribed-row", index)).test_support().text_sm().text_color(t.text).child(email.clone()),
        );
    }
    list.into_any_element()
}

fn query_matches(query: &str, terms: &[&str]) -> bool {
    terms.iter().any(|term| term.to_lowercase().contains(query))
}

fn section_nav(section: Section, selected: bool, cx: &mut Context<SettingsPanel>) -> impl IntoElement {
    let ix = Section::ALL.iter().position(|item| *item == section).unwrap_or(0);
    div().id(("settings-section", ix)).px_2().py_2().rounded_sm().cursor_pointer().text_xs()
        .when(selected, |el| el.bg(theme::active(cx).selection).text_color(theme::active(cx).accent))
        .child(section.label())
        .on_click(cx.listener(move |this, _, _, cx| {
            this.section = section;
            this.cursor = match section {
                Section::General => SettingsPanel::summary_row(),
                Section::Appearance => SettingsPanel::theme_row(),
                Section::Inbox => SettingsPanel::group_row(),
                Section::Blocked => SettingsPanel::blocked_row(0),
                Section::Classifier | Section::Shortcuts => 0,
            };
            cx.notify();
        }))
        .test_support()
}
