//! Settings panel: declarative settings rows grouped into searchable sections, drawn with
//! gpui-kit widgets (Switch, dropdown Button, NumberInput, TabBar, GroupBox) and driven by the
//! keyboard row cursor.

use crate::app::mail_app::panes::Orientation;
use crate::app::overlay::FitViewport as _;
use crate::app::ui::button;
use crate::clock::{Timestamp, DAY};
use crate::judge::{JudgePolicy, Mode, QuestionKey};
use crate::theme;
use gpui_kit::{
    component::{
        button::Button,
        group_box::GroupBox,
        input::{Input, InputEvent, InputState, NumberInput, NumberInputEvent, StepAction},
        menu::{DropdownMenu as _, PopupMenuItem},
        scroll::ScrollableElement as _,
        switch::Switch,
        tab::{Tab, TabBar},
        Disableable as _, Sizable as _,
    },
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
    TabAvatars(bool),
    PreviewLines(u8),
    PaneLayout(Orientation),
    /// New global follow-up timeout, in seconds (`mailbox.set_follow_up_timeout`).
    FollowUp(Timestamp),
    /// Unblock the sender (`mailbox.unblock_sender`); one undo step.
    Unblock(String),
    Close,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section { General, Appearance, Inbox, Blocked, Classifier }

impl Section {
    const ALL: [Self; 5] = [Self::General, Self::Appearance, Self::Inbox, Self::Blocked, Self::Classifier];
    fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Appearance => "Appearance / Theme",
            Self::Inbox => "Inbox & Threads",
            Self::Blocked => "Blocked senders",
            Self::Classifier => "Classifier / AI",
        }
    }
}

#[derive(Clone, Copy)]
enum SettingKey {
    Summaries,
    Theme,
    PaneLayout,
    TabAvatars,
    Grouping,
    PreviewLines,
    FollowUp,
    /// Blocked sender at this index in [`SettingsPanel::blocked`].
    Blocked(usize),
    Classifier(QuestionKey),
    Threshold(QuestionKey),
}

#[derive(Clone, Copy)]
enum ControlKind {
    /// A [`Switch`].
    Toggle,
    /// A dropdown button over a fixed or registry-provided list of options.
    Choice,
    /// A percent [`NumberInput`] that steps by `step`.
    Stepper { min: u8, max: u8, step: u8 },
    /// The follow-up [`NumberInput`], whole days within `min..=max`.
    Days { min: u8, max: u8 },
    /// Drawn by [`render_blocked`], not by the schema-driven rows.
    Action,
}

#[derive(Clone, Copy)]
struct SettingSpec {
    section: Section,
    key: SettingKey,
    title: &'static str,
    description: &'static str,
    control: ControlKind,
}

const PREVIEW_OPTIONS: &[&str] = &["Off", "1 line", "2 lines", "3 lines", "4 lines", "5 lines"];
const CLASSIFIER_OPTIONS: &[&str] = &["Auto", "Review"];
const PANE_OPTIONS: &[&str] = &["Side by side", "Stacked"];

fn setting_spec(key: SettingKey) -> SettingSpec {
    match key {
        SettingKey::Summaries => SettingSpec { section: Section::General, key, title: "Thread summaries", description: "Opt in to generated summaries above conversations.", control: ControlKind::Toggle },
        SettingKey::Theme => SettingSpec { section: Section::Appearance, key, title: "Theme", description: "Choose the color scheme used throughout the app.", control: ControlKind::Choice },
        SettingKey::PaneLayout => SettingSpec { section: Section::Appearance, key, title: "Pane layout", description: "Stack the message list and the reader side by side or one above the other.", control: ControlKind::Choice },
        SettingKey::TabAvatars => SettingSpec { section: Section::Appearance, key, title: "Show sender avatar in tabs", description: "Show the sender's monogram as each reader tab's icon.", control: ControlKind::Toggle },
        SettingKey::Grouping => SettingSpec { section: Section::Inbox, key, title: "Group by thread", description: "Show one inbox row per conversation instead of per message.", control: ControlKind::Toggle },
        SettingKey::PreviewLines => SettingSpec { section: Section::Inbox, key, title: "Preview lines", description: "Snippet lines shown under each subject in the inbox.", control: ControlKind::Choice },
        SettingKey::FollowUp => SettingSpec { section: Section::Inbox, key, title: "Follow-up after", description: "Wait for a reply before flagging.", control: ControlKind::Days { min: 1, max: 14 } },
        SettingKey::Blocked(_) => SettingSpec { section: Section::Blocked, key, title: "Blocked sender", description: "Mail from this sender is blocked.", control: ControlKind::Action },
        SettingKey::Classifier(question) => SettingSpec { section: Section::Classifier, key, title: question.label(), description: "Choose automatic handling or manual review.", control: ControlKind::Choice },
        SettingKey::Threshold(_) => SettingSpec { section: Section::Classifier, key, title: "Confidence threshold", description: "Minimum confidence required for automatic handling.", control: ControlKind::Stepper { min: 0, max: 100, step: 5 } },
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
        setting_spec(SettingKey::TabAvatars),
        setting_spec(SettingKey::Grouping),
        setting_spec(SettingKey::PreviewLines),
        setting_spec(SettingKey::FollowUp),
    ])
}

enum SettingChange { Toggle, Step(f32) }

impl SettingSpec {
    fn apply(self, state: &mut SettingsPanel, change: SettingChange, cx: &mut Context<SettingsPanel>) {
        // Whatever the source (key, dropdown, stepper), the number inputs re-read the model.
        state.resync = true;
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
            (SettingKey::TabAvatars, SettingChange::Toggle) => {
                state.tab_avatars = !state.tab_avatars;
                cx.emit(SettingsEvent::TabAvatars(state.tab_avatars));
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
    /// Number inputs: follow-up days, and one confidence threshold (percent) per question.
    follow_up_input: Entity<InputState>,
    threshold_inputs: Vec<Entity<InputState>>,
    /// The model changed outside the number inputs' own typing: rewrite their text.
    resync: bool,
    _subscriptions: Vec<Subscription>,
    policy: JudgePolicy,
    summaries: bool,
    group: bool,
    tab_avatars: bool,
    preview_lines: u8,
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
        let mut subscriptions = vec![cx.subscribe_in(&input, window, |this, input, event: &InputEvent, window, cx| match event {
            InputEvent::Change => {
                this.search = input.read(cx).value().to_string();
                if !this.search.is_empty() {
                    this.cursor = this.first_search_match();
                }
                cx.notify();
            }
            // Enter hands the keyboard from the search box to the filtered rows.
            InputEvent::PressEnter { .. } => window.focus(&this.focus, cx),
            _ => {}
        })];
        // No `.step(..)`: the inputs then emit `NumberInputEvent::Step` for +/-, the arrow keys and
        // the stepper buttons, and the panel applies the step to its own model.
        let number_input = |window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| InputState::new(window, cx).placeholder("—"))
        };
        let follow_up_input = number_input(window, cx);
        let threshold_inputs: Vec<_> = QuestionKey::ALL.iter().map(|_| number_input(window, cx)).collect();
        let watched = std::iter::once((SettingKey::FollowUp, follow_up_input.clone())).chain(
            QuestionKey::ALL.iter().zip(&threshold_inputs)
                .map(|(question, input)| (SettingKey::Threshold(*question), input.clone())),
        );
        for (key, number) in watched {
            subscriptions.push(cx.subscribe_in(&number, window, move |this, number, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    let text = number.read(cx).value().to_string();
                    this.typed(key, &text, cx);
                }
                InputEvent::Blur => {
                    this.resync = true;
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => {
                    this.resync = true;
                    window.focus(&this.focus, cx);
                    cx.notify();
                }
                InputEvent::Focus => {}
            }));
            subscriptions.push(cx.subscribe(&number, move |this, _, event: &NumberInputEvent, cx| {
                let NumberInputEvent::Step(action) = event;
                let sign = if *action == StepAction::Increment { 1. } else { -1. };
                let delta = if matches!(key, SettingKey::FollowUp) { sign } else { sign * STEP };
                this.cursor = setting_index(key);
                setting_spec(key).apply(this, SettingChange::Step(delta), cx);
            }));
        }
        Self {
            focus: cx.focus_handle(),
            input,
            follow_up_input,
            threshold_inputs,
            resync: true,
            _subscriptions: subscriptions,
            policy,
            summaries,
            group,
            tab_avatars: true,
            preview_lines,
            orientation,
            blocked: Vec::new(),
            unsubscribed: Vec::new(),
            follow_up_days: (crate::model::DEFAULT_FOLLOW_UP_TIMEOUT / DAY).clamp(1, 14) as u8,
            cursor: 0,
            section: Section::Classifier,
            search: String::new(),
        }
    }

    /// Whether tabs show the sender's avatar (the row's initial value).
    pub fn tab_avatars(mut self, on: bool) -> Self {
        self.tab_avatars = on;
        self
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
        self.resync = true;
        self
    }

    fn classifier_rows() -> usize { QuestionKey::ALL.len() * 2 }
    fn summary_row() -> usize { Self::classifier_rows() }
    fn theme_row() -> usize { Self::summary_row() + 1 }
    fn pane_row() -> usize { Self::summary_row() + 2 }
    fn tab_avatar_row() -> usize { Self::summary_row() + 3 }
    fn group_row() -> usize { Self::summary_row() + 4 }
    fn preview_row() -> usize { Self::summary_row() + 5 }
    fn follow_up_row() -> usize { Self::summary_row() + 6 }
    fn blocked_row(index: usize) -> usize { Self::summary_row() + 7 + index }

    /// Set the follow-up timeout to `days` (clamped to `1..=14`); emits the new timeout on change.
    fn set_follow_up(&mut self, days: i64, cx: &mut Context<Self>) {
        let next = days.clamp(1, 14) as u8;
        if next == self.follow_up_days {
            return;
        }
        self.follow_up_days = next;
        cx.emit(SettingsEvent::FollowUp(i64::from(next) * DAY));
        cx.notify();
    }

    fn step_follow_up(&mut self, delta: isize, cx: &mut Context<Self>) {
        self.set_follow_up(i64::from(self.follow_up_days) + delta as i64, cx);
    }

    /// Threshold of `question` as a whole percent, `None` while the question is in Review.
    fn threshold_percent(&self, question: QuestionKey) -> Option<i64> {
        match self.policy.mode(question) {
            Mode::Auto { threshold } => Some((threshold * 100.).round() as i64),
            Mode::Review => None,
        }
    }

    /// A number input's text changed by typing: write a parsable, in-range value to the model.
    /// Half-typed text (`""`, `"-"`) is left alone; an out-of-range number is clamped and shown.
    fn typed(&mut self, key: SettingKey, text: &str, cx: &mut Context<Self>) {
        let Ok(number) = text.trim().parse::<i64>() else { return };
        match key {
            SettingKey::FollowUp => {
                self.resync |= !(1..=14).contains(&number);
                self.set_follow_up(number, cx);
            }
            SettingKey::Threshold(question) => {
                let Some(current) = self.threshold_percent(question) else { return };
                let percent = number.clamp(0, 100);
                self.resync |= percent != number;
                if percent != current {
                    self.policy.set_threshold(question, percent as f32 / 100.);
                    self.changed(cx);
                }
            }
            _ => {}
        }
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
            value if value == Self::tab_avatar_row() => Some(SettingKey::TabAvatars),
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

    /// Show `section` with the cursor on its first row.
    fn select_section(&mut self, section: Section) {
        self.section = section;
        self.cursor = match section {
            Section::General => Self::summary_row(),
            Section::Appearance => Self::theme_row(),
            Section::Inbox => Self::group_row(),
            Section::Blocked => Self::blocked_row(0),
            Section::Classifier => 0,
        };
    }

    fn move_section(&mut self, delta: isize) {
        let index = Section::ALL.iter().position(|section| *section == self.section).unwrap_or(0) as isize;
        let next = (index + delta).rem_euclid(Section::ALL.len() as isize) as usize;
        self.select_section(Section::ALL[next]);
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
            (Self::tab_avatar_row(), &["show sender avatar in tabs", "avatar", "monogram", "tab icon", "appearance"]),
            (Self::group_row(), &["group by thread", "inbox", "threads"]),
            (Self::preview_row(), &["preview lines", "message snippet", "inbox"]),
            (Self::follow_up_row(), &["follow-up after", "follow up", "wait for a response", "flag"]),
            (Self::blocked_row(0), &["blocked senders", "blocked", "unblock", "unsubscribed", "resubscribe"]),
            (Self::pane_row(), &["pane layout", "side by side", "stacked", "layout", "appearance"]),
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
        };
        if count > 0 {
            let end = start + count - 1;
            self.cursor = (self.cursor as isize + delta).clamp(start as isize, end as isize) as usize;
        }
    }

    /// Current state of a [`ControlKind::Toggle`] row.
    fn is_on(&self, key: SettingKey) -> bool {
        match key {
            SettingKey::Summaries => self.summaries,
            SettingKey::TabAvatars => self.tab_avatars,
            SettingKey::Grouping => self.group,
            _ => false,
        }
    }

    /// Option labels and the current option of a [`ControlKind::Choice`] row.
    fn choices(&self, key: SettingKey, cx: &App) -> (Vec<SharedString>, usize) {
        let fixed = |options: &[&'static str], current: usize| {
            (options.iter().map(|option| SharedString::from(*option)).collect(), current)
        };
        match key {
            SettingKey::Theme => {
                let names = theme::names(cx);
                let current = theme::active(cx).name.clone();
                let at = names.iter().position(|name| *name == current).unwrap_or(0);
                (names.into_iter().map(SharedString::from).collect(), at)
            }
            SettingKey::PaneLayout => fixed(PANE_OPTIONS, usize::from(self.orientation == Orientation::Stacked)),
            SettingKey::PreviewLines => fixed(PREVIEW_OPTIONS, usize::from(self.preview_lines.min(5))),
            SettingKey::Classifier(question) => {
                fixed(CLASSIFIER_OPTIONS, usize::from(matches!(self.policy.mode(question), Mode::Review)))
            }
            _ => (Vec::new(), 0),
        }
    }

    /// Choose option `ix` of a [`ControlKind::Choice`] row (from its dropdown menu).
    fn pick(&mut self, key: SettingKey, ix: usize, cx: &mut Context<Self>) {
        self.cursor = setting_index(key);
        match key {
            SettingKey::Theme => {
                if let Some(name) = theme::names(cx).get(ix) {
                    theme::set_active(cx, name);
                    self.changed(cx);
                }
            }
            SettingKey::PaneLayout => {
                let next = if ix == 1 { Orientation::Stacked } else { Orientation::SideBySide };
                if next != self.orientation {
                    self.orientation = next;
                    cx.emit(SettingsEvent::PaneLayout(next));
                    cx.notify();
                }
            }
            SettingKey::PreviewLines => {
                let lines = ix.min(5) as u8;
                if lines != self.preview_lines {
                    self.preview_lines = lines;
                    cx.emit(SettingsEvent::PreviewLines(lines));
                    cx.notify();
                }
            }
            SettingKey::Classifier(question)
                if matches!(self.policy.mode(question), Mode::Review) != (ix == 1) =>
            {
                self.resync = true;
                self.policy.toggle_mode(question);
                self.changed(cx);
            }
            _ => {}
        }
    }

    /// Rewrite the number inputs' text from the model (after a key, menu or step change).
    fn sync_numbers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !std::mem::take(&mut self.resync) {
            return;
        }
        let days = self.follow_up_days.to_string();
        self.follow_up_input.update(cx, |input, cx| input.set_value(days, window, cx));
        for (question, input) in QuestionKey::ALL.iter().zip(&self.threshold_inputs) {
            let text = self.threshold_percent(*question).map(|percent| percent.to_string()).unwrap_or_default();
            input.update(cx, |input, cx| input.set_value(text, window, cx));
        }
    }

    fn number_focused(&self, window: &Window, cx: &App) -> bool {
        std::iter::once(&self.follow_up_input).chain(&self.threshold_inputs)
            .any(|input| input.focus_handle(cx).is_focused(window))
    }
}

impl Focusable for SettingsPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl EventEmitter<SettingsEvent> for SettingsPanel {}

/// One settings row: title and description on the left, the `control` widget on the right.
fn row(
    t: &theme::Theme,
    selected: bool,
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
    control: impl IntoElement,
) -> Div {
    let hover = t.hover;
    div().flex().items_center().justify_between().gap_2().px_2().py_1().rounded_sm().text_sm()
        .text_color(t.text).hover(move |el| el.bg(hover))
        .when(selected, |el| el.bg(t.selection).text_color(t.accent))
        .child(div().flex().flex_col().min_w_0().child(title.into())
            .child(div().text_xs().text_color(t.text_muted).truncate().child(description.into())))
        .child(control)
}

fn setting_index(key: SettingKey) -> usize {
    match key {
        SettingKey::Classifier(question) => QuestionKey::ALL.iter().position(|key| *key == question).unwrap_or(0) * 2,
        SettingKey::Threshold(question) => QuestionKey::ALL.iter().position(|key| *key == question).unwrap_or(0) * 2 + 1,
        SettingKey::Summaries => SettingsPanel::summary_row(),
        SettingKey::Theme => SettingsPanel::theme_row(),
        SettingKey::PaneLayout => SettingsPanel::pane_row(),
        SettingKey::TabAvatars => SettingsPanel::tab_avatar_row(),
        SettingKey::Grouping => SettingsPanel::group_row(),
        SettingKey::PreviewLines => SettingsPanel::preview_row(),
        SettingKey::FollowUp => SettingsPanel::follow_up_row(),
        SettingKey::Blocked(index) => SettingsPanel::blocked_row(index),
    }
}

/// Element id of a setting's control (what headless tests click).
fn setting_id(key: SettingKey) -> ElementId {
    match key {
        SettingKey::Classifier(question) => ("question-row", QuestionKey::ALL.iter().position(|key| *key == question).unwrap_or(0)).into(),
        SettingKey::Threshold(question) => ("threshold-row", QuestionKey::ALL.iter().position(|key| *key == question).unwrap_or(0)).into(),
        SettingKey::Summaries => "summaries-row".into(),
        SettingKey::Theme => "theme-row".into(),
        SettingKey::PaneLayout => "pane-layout-row".into(),
        SettingKey::TabAvatars => "tab-avatars-row".into(),
        SettingKey::Grouping => "group-row".into(),
        SettingKey::PreviewLines => "preview-lines-row".into(),
        SettingKey::FollowUp => "follow-up-row".into(),
        SettingKey::Blocked(index) => ("blocked-row", index).into(),
    }
}

fn render_setting(
    spec: SettingSpec,
    state: &SettingsPanel,
    t: &theme::Theme,
    cx: &mut Context<SettingsPanel>,
) -> AnyElement {
    let key = spec.key;
    let index = setting_index(key);
    let selected = state.cursor == index;
    let weak = cx.entity().downgrade();
    let id = setting_id(key);
    match spec.control {
        ControlKind::Toggle => {
            // The Switch never takes focus: the row cursor keeps driving it from the keyboard.
            let control = Switch::new(("settings-switch", index)).checked(state.is_on(key)).tab_stop(false)
                .on_change(move |_, _, cx| {
                    weak.update(cx, |this, cx| {
                        this.cursor = index;
                        setting_spec(key).apply(this, SettingChange::Toggle, cx);
                    }).ok();
                });
            row(t, selected, spec.title, spec.description, div().id(id).test_support().child(control)).into_any_element()
        }
        ControlKind::Choice => {
            let (options, current) = state.choices(key, cx);
            let label = options.get(current).cloned().unwrap_or_default();
            let menu = Button::new(("settings-choice", index)).label(label).xsmall().outline()
                .dropdown_caret(true).border_color(t.border).text_color(t.text)
                .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                    options.iter().enumerate().fold(menu, |menu, (ix, option)| {
                        let weak = weak.clone();
                        menu.item(PopupMenuItem::new(option.clone()).checked(ix == current).on_click(move |_, _, cx| {
                            weak.update(cx, |this, cx| this.pick(key, ix, cx)).ok();
                        }))
                    })
                });
            let control = div().flex().items_center().gap_2()
                .when(matches!(key, SettingKey::Theme), |el| el.child(div().flex().gap_1()
                    .child(div().w(px(9.)).h(px(9.)).rounded_full().bg(t.surface))
                    .child(div().w(px(9.)).h(px(9.)).rounded_full().bg(t.accent))
                    .child(div().w(px(9.)).h(px(9.)).rounded_full().bg(t.selection))))
                .child(div().id(id).test_support().child(menu));
            row(t, selected, spec.title, spec.description, control).into_any_element()
        }
        ControlKind::Stepper { min, max, step } => {
            let SettingKey::Threshold(question) = key else { return row(t, selected, spec.title, spec.description, div()).into_any_element() };
            let at = QuestionKey::ALL.iter().position(|key| *key == question).unwrap_or(0);
            let auto = matches!(state.policy.mode(question), Mode::Auto { .. });
            let description: SharedString = format!("{} · range {min}–{max}%, step {step}%", spec.description).into();
            let control = div().id(id).test_support().w(px(112.))
                .child(NumberInput::new(&state.threshold_inputs[at]).xsmall().disabled(!auto));
            row(t, selected, spec.title, description, control).into_any_element()
        }
        ControlKind::Days { min, max } => {
            let description: SharedString = format!("{} · {min}–{max} days", spec.description).into();
            let control = div().id(id).test_support().w(px(112.))
                .child(NumberInput::new(&state.follow_up_input).xsmall());
            row(t, selected, spec.title, description, control).into_any_element()
        }
        ControlKind::Action => row(t, selected, spec.title, spec.description, div()).into_any_element(),
    }
}

impl Render for SettingsPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_numbers(window, cx);
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
        let selected_tab = visible.iter().position(|section| *section == self.section).unwrap_or(0);
        let tabs = TabBar::new("settings-sections").underline().selected_index(selected_tab)
            .children(visible.iter().map(|section| {
                let section = *section;
                let ix = Section::ALL.iter().position(|item| *item == section).unwrap_or(0);
                Tab::new()
                    .child(div().id(("settings-section", ix)).test_support().child(section.label()))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.select_section(section);
                        cx.notify();
                    }))
            }));
        let filtered: Vec<_> = setting_specs().filter(|spec| {
            matches(spec) && (searching || spec.section == self.section)
        }).map(|spec| render_setting(spec, self, &t, cx)).collect();
        let blocked = (if searching { blocked_match } else { self.section == Section::Blocked })
            .then(|| render_blocked(self, &t, cx));
        let no_results = searching
            && !blocked_match
            && !setting_specs().any(|spec| matches(&spec));
        let title = if searching { "Search results" } else { self.section.label() };
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
                } else if this.number_focused(window, cx) {
                    // Leave the number input for the row cursor; the next escape closes.
                    window.focus(&this.focus, cx);
                } else { cx.emit(SettingsEvent::Close); }
            }))
            .flex().flex_col().fit_viewport(window, 760.).p_3().gap_2()
            .bg(t.surface).border_1().border_color(t.border).rounded_md().id("settings-panel").overflow_y_scrollbar()
            .child(div().flex().items_center().justify_between().child(div().text_sm().text_color(t.accent).child("Settings"))
                .child(button("settings-close", "Close", "Close", "escape", cx).on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::Close)))))
            .child(div().id("settings-search").test_support().child(Input::new(&self.input).appearance(false)))
            .child(tabs)
            .child(div().flex_1().flex().flex_col().gap_2().overflow_y_scrollbar()
                .when(!filtered.is_empty(), |el| el.child(GroupBox::new().title(title).children(filtered)))
                .when_some(blocked, |el, block| el.child(GroupBox::new().title(Section::Blocked.label()).child(block)))
                .when(no_results, |el| el.child(div().text_sm().text_color(t.text_muted).child("No matching settings"))))
            .child(div().h(px(1.)).bg(t.border))
            .child(div().text_xs().text_color(t.text_muted).child("↑/↓ or j/k move · Space change · Ctrl-Tab sections · / search (Enter: results) · Esc clear / close"))
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
                .child(div().flex_1().child(row(t, selected, email.clone(), "Blocked sender", div())))
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
