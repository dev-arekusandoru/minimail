//! Settings panel: declarative settings rows grouped into searchable sections.

use crate::app::overlay::FitViewport as _;
use crate::app::ui::button;
use crate::judge::{JudgePolicy, Mode, QuestionKey};
use crate::theme;
use gpui_kit::{
    component::input::{Input, InputEvent, InputState},
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
    Close,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section { General, Appearance, Inbox, Classifier, Shortcuts }

impl Section {
    const ALL: [Self; 5] = [Self::General, Self::Appearance, Self::Inbox, Self::Classifier, Self::Shortcuts];
    fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Appearance => "Appearance / Theme",
            Self::Inbox => "Inbox & Threads",
            Self::Classifier => "Classifier / AI",
            Self::Shortcuts => "Keyboard / Shortcuts",
        }
    }
}

#[derive(Clone, Copy)]
enum SettingKey { Summaries, Theme, Grouping, PreviewLines, Classifier(QuestionKey) }

#[derive(Clone, Copy)]
enum ControlKind { Toggle, Choice(&'static [&'static str]), Stepper { min: u8, max: u8, step: u8 }, Classifier }

struct SettingSpec {
    section: Section,
    key: SettingKey,
    title: &'static str,
    description: &'static str,
    control: ControlKind,
}

const THEMES_CONTROL: &[&str] = &["theme registry"];

fn setting_spec(key: SettingKey) -> SettingSpec {
    match key {
        SettingKey::Summaries => SettingSpec { section: Section::General, key, title: "Thread summaries", description: "Opt in to generated summaries above conversations.", control: ControlKind::Toggle },
        SettingKey::Theme => SettingSpec { section: Section::Appearance, key, title: "Theme", description: "Choose the color scheme used throughout the app.", control: ControlKind::Choice(THEMES_CONTROL) },
        SettingKey::Grouping => SettingSpec { section: Section::Inbox, key, title: "Group by thread", description: "Show one inbox row per conversation instead of per message.", control: ControlKind::Toggle },
        SettingKey::PreviewLines => SettingSpec { section: Section::Inbox, key, title: "Preview lines", description: "Snippet lines shown under each subject in the inbox.", control: ControlKind::Stepper { min: 0, max: 5, step: 1 } },
        SettingKey::Classifier(question) => SettingSpec { section: Section::Classifier, key, title: question.label(), description: "Choose automatic handling or manual review; adjust the confidence threshold.", control: ControlKind::Classifier },
    }
}

pub struct SettingsPanel {
    focus: FocusHandle,
    input: Entity<InputState>,
    policy: JudgePolicy,
    summaries: bool,
    group: bool,
    preview_lines: u8,
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
            cursor: 0,
            section: Section::Classifier,
            search: String::new(),
        }
    }

    fn rows() -> usize {
        QuestionKey::ALL.len() + 4
    }

    fn theme_row() -> usize { QuestionKey::ALL.len() + 1 }
    fn group_row() -> usize { QuestionKey::ALL.len() + 2 }
    fn preview_row() -> usize { QuestionKey::ALL.len() + 3 }

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
        if let Some(&question) = QuestionKey::ALL.get(row) {
            Some(SettingKey::Classifier(question))
        } else if row == QuestionKey::ALL.len() {
            Some(SettingKey::Summaries)
        } else if row == Self::theme_row() {
            Some(SettingKey::Theme)
        } else if row == Self::group_row() {
            Some(SettingKey::Grouping)
        } else if row == Self::preview_row() {
            Some(SettingKey::PreviewLines)
        } else {
            None
        }
    }
    fn setting_value(&self, key: SettingKey, current_theme: &str) -> String {
        match key {
            SettingKey::Summaries => if self.summaries { "on" } else { "off" }.into(),
            SettingKey::Theme => current_theme.to_owned(),
            SettingKey::Grouping => if self.group { "on" } else { "off" }.into(),
            SettingKey::PreviewLines => if self.preview_lines == 0 { "Off".into() } else { format!("{} lines", self.preview_lines) },
            SettingKey::Classifier(question) => match self.policy.mode(question) {
                Mode::Auto { threshold } => format!("auto ≥ {threshold:.2}"),
                Mode::Review => "review".into(),
            },
        }
    }

    fn toggle(&mut self, cx: &mut Context<Self>) {
        match Self::row_key(self.cursor) {
            Some(SettingKey::Theme) => self.cycle_theme(1, cx),
            Some(SettingKey::Grouping) => {
                self.group = !self.group;
                cx.emit(SettingsEvent::Grouping(self.group));
                cx.notify();
            }
            Some(SettingKey::PreviewLines) => {
                self.preview_lines = (self.preview_lines + 1) % 6;
                cx.emit(SettingsEvent::PreviewLines(self.preview_lines));
                cx.notify();
            }
            Some(SettingKey::Summaries) => {
                self.summaries = !self.summaries;
                self.changed(cx);
            }
            Some(SettingKey::Classifier(key)) => {
                self.policy.toggle_mode(key);
                self.changed(cx);
            }
            None => {}
        }
    }

    fn nudge(&mut self, delta: f32, cx: &mut Context<Self>) {
        match Self::row_key(self.cursor) {
            Some(SettingKey::Theme) => self.cycle_theme(if delta > 0.0 { 1 } else { -1 }, cx),
            Some(SettingKey::PreviewLines) => {
                self.preview_lines = (self.preview_lines as i8 + if delta > 0.0 { 1 } else { -1 }).clamp(0, 5) as u8;
                cx.emit(SettingsEvent::PreviewLines(self.preview_lines));
                cx.notify();
            }
            Some(SettingKey::Classifier(key)) if matches!(self.policy.mode(key), Mode::Auto { .. }) => {
                self.policy.nudge_threshold(key, delta);
                self.changed(cx);
            }
            _ => {}
        }
    }
    fn move_section(&mut self, delta: isize) {
        let index = Section::ALL.iter().position(|section| *section == self.section).unwrap_or(0) as isize;
        let next = (index + delta).rem_euclid(Section::ALL.len() as isize) as usize;
        self.section = Section::ALL[next];
        self.cursor = match self.section {
            Section::General => QuestionKey::ALL.len(),
            Section::Appearance => Self::theme_row(),
            Section::Inbox => Self::group_row(),
            Section::Classifier | Section::Shortcuts => 0,
        };
    }

    fn first_search_match(&self) -> usize {
        let query = self.search.to_lowercase();
        for (index, key) in QuestionKey::ALL.iter().enumerate() {
            if query_matches(&query, &[key.label(), "classifier confidence threshold", "auto apply or review"]) {
                return index;
            }
        }
        [
            (QuestionKey::ALL.len(), ["thread summaries", "opt in to generated summaries"]),
            (Self::theme_row(), ["theme", "appearance", "color scheme"]),
            (Self::group_row(), ["group by thread", "inbox", "threads"]),
            (Self::preview_row(), ["preview lines", "message snippet", "inbox"]),
        ].into_iter().find(|(_, terms)| query_matches(&query, terms)).map_or(0, |(index, _)| index)
    }
    fn move_row(&mut self, delta: isize) {
        if !self.search.is_empty() {
            self.cursor = (self.cursor as isize + delta).clamp(0, Self::rows() as isize - 1) as usize;
            return;
        }
        let (start, count) = match self.section {
            Section::General => (QuestionKey::ALL.len(), 1),
            Section::Appearance => (Self::theme_row(), 1),
            Section::Inbox => (Self::group_row(), 2),
            Section::Classifier => (0, QuestionKey::ALL.len()),
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
        .child(div().flex().flex_col()
            .child(label)
            .child(div().text_xs().text_color(t.text_muted).child(setting_description(label))))
        .child(div().text_xs().text_color(t.text_muted).child(SharedString::from(value)))
}

fn setting_description(title: &str) -> &'static str {
    let key = match title {
        "Thread summaries" => SettingKey::Summaries,
        "Theme" => SettingKey::Theme,
        "Group by thread" => SettingKey::Grouping,
        "Preview lines" => SettingKey::PreviewLines,
        _ => match QuestionKey::ALL.iter().find(|key| key.label() == title) {
            Some(&question) => SettingKey::Classifier(question),
            None => return "Choose automatic handling or manual review; adjust the confidence threshold.",
        },
    };
    let spec = setting_spec(key);
    let _schema = (spec.section, spec.key, spec.title, match spec.control {
        ControlKind::Toggle => 0,
        ControlKind::Choice(options) => options.len(),
        ControlKind::Stepper { min, max, step } => (max - min) as usize / step as usize,
        ControlKind::Classifier => 0,
    });
    spec.description
}

impl Render for SettingsPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::active(cx);
        let query = self.search.trim().to_lowercase();
        let searching = !query.is_empty();
        let cursor = self.cursor;
        let sections = Section::ALL.into_iter().filter(|section| !searching || match section {
            Section::General => query_matches(&query, &["general", "thread summaries", "opt in to generated summaries"]),
            Section::Appearance => query_matches(&query, &["appearance", "theme", "color scheme"]),
            Section::Inbox => query_matches(&query, &["inbox", "threads", "group by thread", "preview lines", "message snippet"]),
            Section::Classifier => QuestionKey::ALL.iter().any(|key| query_matches(&query, &[key.label(), "classifier confidence threshold", "auto apply or review"])),
            Section::Shortcuts => query_matches(&query, &["keyboard", "shortcuts", "escape", "space", "j/k", "settings"]),
        }).map(|section| section_nav(section, section == self.section, cx));
        let questions = QuestionKey::ALL.iter().enumerate().filter(|(_, key)| {
            !searching || query_matches(&query, &[key.label(), "classifier confidence threshold", "auto apply or review"])
        }).map(|(i, &key)| {
            let mode = self.policy.mode(key);
            let spec = setting_spec(SettingKey::Classifier(key));
            let value = self.setting_value(spec.key, &t.name);
            let auto = matches!(mode, Mode::Auto { .. });
            div().flex().items_center().gap_1().child(
                div().id(("question-row", i)).flex_1().cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| { this.cursor = i; this.toggle(cx); }))
                    .child(row(&t, i == cursor, spec.title, value))
            )
            .when(auto, |d| d
                .child(button(("threshold-down", i), "−", "Lower threshold", "-", cx)
                    .on_click(cx.listener(move |this, _, _, cx| { this.cursor = i; this.nudge(-STEP, cx); })))
                .child(button(("threshold-up", i), "+", "Raise threshold", "=", cx)
                    .on_click(cx.listener(move |this, _, _, cx| { this.cursor = i; this.nudge(STEP, cx); }))))
        });
        let show_general = !searching || query_matches(&query, &["thread summaries", "opt in to generated summaries"]);
        let show_theme = !searching || query_matches(&query, &["theme", "appearance", "color scheme"]);
        let show_group = !searching || query_matches(&query, &["group by thread", "inbox", "threads"]);
        let show_preview = !searching || query_matches(&query, &["preview lines", "message snippet", "inbox"]);
        let show_shortcuts = !searching || query_matches(&query, &["keyboard", "shortcuts", "escape", "space", "j/k", "settings"]);
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
                .child(button("settings-close", "Close", "Close", "escape", cx).on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::Close)))))
            .child(Input::new(&self.input).appearance(false).id("settings-search"))
            .when(narrow, |el| el.child(div().flex().flex_row().gap_1().children(
                Section::ALL.into_iter().filter(|section| !searching || *section == self.section || match section {
                    Section::General => show_general,
                    Section::Appearance => show_theme,
                    Section::Inbox => show_group || show_preview,
                    Section::Classifier => true,
                    Section::Shortcuts => show_shortcuts,
                }).map(|section| section_nav(section, section == self.section, cx))
            )))
            .child(div().flex().gap_3().flex_1()
                .when(!narrow, |el| el.child(div().w(px(190.)).flex().flex_col().gap_1().children(sections)))
                .child(div().flex_1().flex().flex_col().gap_1().overflow_y_scroll()
                    .child(div().text_sm().text_color(t.accent).child(if searching { "Search results" } else { self.section.label() }))
                    .child(div().h(px(1.)).bg(t.border))
                    .when(searching || self.section == Section::Classifier, |el| el.children(questions))
                    .when(!searching && self.section == Section::General, |el| el.child(
                        div().id("summaries-row").cursor_pointer().on_click(cx.listener(|this, _, _, cx| { this.cursor = QuestionKey::ALL.len(); this.toggle(cx); }))
                            .child(row(&t, cursor == QuestionKey::ALL.len(), "Thread summaries", if self.summaries { "on" } else { "off" }.into()))))
                    .when(!searching && self.section == Section::Appearance, |el| el.child(
                        div().id("theme-row").cursor_pointer().on_click(cx.listener(|this, _, _, cx| { this.cursor = Self::theme_row(); this.cycle_theme(1, cx); }))
                            .child(row(&t, cursor == Self::theme_row(), "Theme", t.name.clone()))))
                    .when(!searching && self.section == Section::Inbox, |el| el
                        .child(div().id("group-row").cursor_pointer().on_click(cx.listener(|this, _, _, cx| { this.cursor = Self::group_row(); this.toggle(cx); }))
                            .child(row(&t, cursor == Self::group_row(), "Group by thread", if self.group { "on" } else { "off" }.into())))
                        .child(div().id("preview-lines-row").cursor_pointer().on_click(cx.listener(|this, _, _, cx| { this.cursor = Self::preview_row(); this.toggle(cx); }))
                            .child(row(&t, cursor == Self::preview_row(), "Preview lines", if self.preview_lines == 0 { "Off".into() } else { format!("{} lines", self.preview_lines) })))))
                    .when(searching && show_general, |el| el.child(div().text_xs().text_color(t.text_muted).child("General"))
                        .child(div().id("summaries-row").cursor_pointer().on_click(cx.listener(|this, _, _, cx| { this.cursor = QuestionKey::ALL.len(); this.toggle(cx); }))
                            .child(row(&t, cursor == QuestionKey::ALL.len(), "Thread summaries", if self.summaries { "on" } else { "off" }.into()))))
                    .when(searching && show_theme, |el| el.child(div().text_xs().text_color(t.text_muted).child("Appearance / Theme"))
                        .child(div().id("theme-row").cursor_pointer().on_click(cx.listener(|this, _, _, cx| { this.cursor = Self::theme_row(); this.cycle_theme(1, cx); }))
                            .child(row(&t, cursor == Self::theme_row(), "Theme", t.name.clone()))))
                    .when(searching && show_group, |el| el.child(div().text_xs().text_color(t.text_muted).child("Inbox & Threads"))
                        .child(div().id("group-row").cursor_pointer().on_click(cx.listener(|this, _, _, cx| { this.cursor = Self::group_row(); this.toggle(cx); }))
                            .child(row(&t, cursor == Self::group_row(), "Group by thread", if self.group { "on" } else { "off" }.into()))))
                    .when(searching && show_preview, |el| el.child(
                        div().id("preview-lines-row").cursor_pointer().on_click(cx.listener(|this, _, _, cx| { this.cursor = Self::preview_row(); this.toggle(cx); }))
                            .child(row(&t, cursor == Self::preview_row(), "Preview lines", if self.preview_lines == 0 { "Off".into() } else { format!("{} lines", self.preview_lines) }))))
                    .when(!searching && self.section == Section::Shortcuts, |el| el
                        .child(div().text_sm().text_color(t.text).child("j / ↓ — next setting"))
                        .child(div().text_sm().text_color(t.text).child("k / ↑ — previous setting"))
                        .child(div().text_sm().text_color(t.text).child("Space / Enter — change selected value"))
                        .child(div().text_sm().text_color(t.text).child("Ctrl-Tab / Ctrl-Shift-Tab — switch section"))
                        .child(div().text_sm().text_color(t.text).child("/ — focus settings search"))
                        .child(div().text_sm().text_color(t.text).child("Escape — clear search, then close settings")))
                    .when(searching && show_shortcuts, |el| el.child(div().text_xs().text_color(t.text_muted).child("Keyboard / Shortcuts"))
                        .child(div().text_sm().text_color(t.text).child("j/k — move · Space — change · Ctrl-Tab — switch section")))
                    .when(searching && !show_general && !show_theme && !show_group && !show_preview && !show_shortcuts && QuestionKey::ALL.iter().all(|key| !query_matches(&query, &[key.label(), "classifier confidence threshold", "auto apply or review"])), |el| el.child(div().text_sm().text_color(t.text_muted).child("No matching settings")))
            )

            .child(div().h(px(1.)).bg(t.border))
            .child(div().text_xs().text_color(t.text_muted).child("↑/↓ or j/k move · Space change · Ctrl-Tab sections · / search · Esc clear / close"))
    }
}

fn query_matches(query: &str, terms: &[&str]) -> bool {
    terms.iter().any(|term| term.to_lowercase().contains(query))
}

fn section_nav(section: Section, selected: bool, cx: &mut Context<SettingsPanel>) -> impl IntoElement {
    div().id(("settings-section", section.label())).px_2().py_2().rounded_sm().cursor_pointer().text_xs()
        .when(selected, |el| el.bg(theme::active(cx).selection).text_color(theme::active(cx).accent))
        .child(section.label())
        .on_click(cx.listener(move |this, _, _, cx| {
            this.section = section;
            this.cursor = match section {
                Section::General => QuestionKey::ALL.len(),
                Section::Appearance => Self::theme_row(),
                Section::Inbox => Self::group_row(),
                Section::Classifier | Section::Shortcuts => 0,
            };
            cx.notify();
        }))
}
