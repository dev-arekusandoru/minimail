//! Settings panel: gpui-kit's `Settings` component (sidebar pages with search, grouped setting
//! items) hosted in a modal. The panel entity owns the settings state and emits
//! [`SettingsEvent`]s; the component's field getters/setters reach it through a `WeakEntity`.

use crate::app::mail_app::panes::Orientation;
use crate::app::overlay::{fit_height, fit_width, top_offset};
use crate::app::ui::button;
use crate::clock::{DAY, Timestamp};
use crate::judge::{JudgePolicy, Mode, QuestionKey};
use crate::{preview, theme};
use gpui_kit::{
    component::setting::{
        NumberFieldOptions, SettingField, SettingGroup, SettingItem, SettingPage, Settings,
    },
    *,
};

/// Key context of the panel.
pub const SETTINGS_CONTEXT: &str = "SettingsPanel";

gpui_kit::actions!(settings, [SettingsClose]);

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

const CLASSIFIER_MODES: [(&str, &str); 2] = [("auto", "Auto"), ("review", "Review")];
const PANE_LAYOUTS: [(&str, &str); 2] = [("side", "Side by side"), ("stacked", "Stacked")];
const MAX_PREVIEW_LINES: u8 = 5;
const FOLLOW_UP_DAYS: std::ops::RangeInclusive<u8> = 1..=14;

type Weak = WeakEntity<SettingsPanel>;

pub struct SettingsPanel {
    focus: FocusHandle,
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
    /// Days to wait for a reply before flagging a thread.
    follow_up_days: u8,
}

impl SettingsPanel {
    pub fn new(
        policy: JudgePolicy,
        summaries: bool,
        group: bool,
        preview_lines: u8,
        orientation: Orientation,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            focus: cx.focus_handle(),
            policy,
            summaries,
            group,
            tab_avatars: true,
            preview_lines: preview_lines.min(MAX_PREVIEW_LINES),
            orientation,
            blocked: Vec::new(),
            unsubscribed: Vec::new(),
            follow_up_days: (crate::model::DEFAULT_FOLLOW_UP_TIMEOUT / DAY)
                .clamp((*FOLLOW_UP_DAYS.start()).into(), (*FOLLOW_UP_DAYS.end()).into()) as u8,
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
        self.follow_up_days = follow_up_timeout
            .div_euclid(DAY)
            .clamp((*FOLLOW_UP_DAYS.start()).into(), (*FOLLOW_UP_DAYS.end()).into())
            as u8;
        self
    }

    fn changed(&self, cx: &mut Context<Self>) {
        cx.emit(SettingsEvent::Changed(self.policy.clone(), self.summaries));
        cx.notify();
    }

    fn set_follow_up(&mut self, days: f64, cx: &mut Context<Self>) {
        let days = days.round().clamp(f64::from(*FOLLOW_UP_DAYS.start()), f64::from(*FOLLOW_UP_DAYS.end())) as u8;
        if days != self.follow_up_days {
            self.follow_up_days = days;
            cx.emit(SettingsEvent::FollowUp(i64::from(days) * DAY));
            cx.notify();
        }
    }

    fn set_threshold(&mut self, question: QuestionKey, percent: f64, cx: &mut Context<Self>) {
        let threshold = (percent.clamp(0., 100.) / 100.) as f32;
        if (self.policy.threshold(question) - threshold).abs() > f32::EPSILON {
            self.policy.set_threshold(question, threshold);
            self.changed(cx);
        }
    }

    fn set_classifier_mode(&mut self, question: QuestionKey, review: bool, cx: &mut Context<Self>) {
        if matches!(self.policy.mode(question), Mode::Review) != review {
            self.policy.toggle_mode(question);
            self.changed(cx);
        }
    }

    fn set_orientation(&mut self, orientation: Orientation, cx: &mut Context<Self>) {
        if orientation != self.orientation {
            self.orientation = orientation;
            cx.emit(SettingsEvent::PaneLayout(orientation));
            cx.notify();
        }
    }

    fn set_preview_lines(&mut self, lines: u8, cx: &mut Context<Self>) {
        let lines = lines.min(MAX_PREVIEW_LINES);
        if lines != self.preview_lines {
            self.preview_lines = lines;
            cx.emit(SettingsEvent::PreviewLines(lines));
            cx.notify();
        }
    }

    fn set_grouping(&mut self, on: bool, cx: &mut Context<Self>) {
        if on != self.group {
            self.group = on;
            cx.emit(SettingsEvent::Grouping(on));
            cx.notify();
        }
    }

    fn set_tab_avatars(&mut self, on: bool, cx: &mut Context<Self>) {
        if on != self.tab_avatars {
            self.tab_avatars = on;
            cx.emit(SettingsEvent::TabAvatars(on));
            cx.notify();
        }
    }

    fn set_summaries(&mut self, on: bool, cx: &mut Context<Self>) {
        if on != self.summaries {
            self.summaries = on;
            self.changed(cx);
        }
    }

    /// Remove `email` from the local list and ask the app to unblock it.
    fn unblock(&mut self, email: &str, cx: &mut Context<Self>) {
        let before = self.blocked.len();
        self.blocked.retain(|blocked| blocked != email);
        if self.blocked.len() != before {
            cx.emit(SettingsEvent::Unblock(email.to_owned()));
            cx.notify();
        }
    }

    fn pages(&self, weak: &Weak, cx: &App) -> Vec<SettingPage> {
        vec![
            self.general_page(weak),
            self.appearance_page(weak, cx),
            self.inbox_page(weak),
            self.blocked_page(weak),
            self.classifier_page(weak),
        ]
    }

    fn general_page(&self, weak: &Weak) -> SettingPage {
        SettingPage::new("General").resettable(false).group(SettingGroup::new().item(
            SettingItem::new(
                "Thread summaries",
                switch(weak, |this| this.summaries, SettingsPanel::set_summaries),
            )
            .description("Opt in to generated summaries above conversations.")
            .keywords(["opt in to generated summaries"]),
        ))
    }

    fn appearance_page(&self, weak: &Weak, cx: &App) -> SettingPage {
        let themes = theme::names(cx)
            .into_iter()
            .map(|name| (SharedString::from(name.clone()), SharedString::from(name)))
            .collect();
        let theme_field = {
            let weak = weak.clone();
            SettingField::dropdown(
                themes,
                |cx| SharedString::from(theme::active(cx).name.clone()),
                move |name: SharedString, cx| {
                    theme::set_active(cx, &name);
                    weak.update(cx, |this, cx| this.changed(cx)).ok();
                },
            )
        };
        let layout_field = {
            let (get, set) = (weak.clone(), weak.clone());
            SettingField::dropdown(
                options(&PANE_LAYOUTS),
                move |cx| {
                    let stacked = get
                        .read_with(cx, |this, _| this.orientation == Orientation::Stacked)
                        .unwrap_or_default();
                    pick(&PANE_LAYOUTS, usize::from(stacked))
                },
                move |value: SharedString, cx| {
                    let orientation =
                        if value == "stacked" { Orientation::Stacked } else { Orientation::SideBySide };
                    set.update(cx, |this, cx| this.set_orientation(orientation, cx)).ok();
                },
            )
        };
        SettingPage::new("Appearance").resettable(false).group(
            SettingGroup::new()
                .item(
                    SettingItem::new("Theme", theme_field)
                        .description("Choose the color scheme used throughout the app.")
                        .keywords(["appearance", "color scheme"]),
                )
                .item(
                    SettingItem::new("Pane layout", layout_field)
                        .description("Stack the message list and the reader side by side or one above the other.")
                        .keywords(["side by side", "stacked", "layout", "appearance"]),
                )
                .item(
                    SettingItem::new(
                        "Show sender avatar in tabs",
                        switch(weak, |this| this.tab_avatars, SettingsPanel::set_tab_avatars),
                    )
                    .description("Show the sender's monogram as each reader tab's icon.")
                    .keywords(["avatar", "monogram", "tab icon", "appearance"]),
                ),
        )
    }

    fn inbox_page(&self, weak: &Weak) -> SettingPage {
        let preview_options = (0..=MAX_PREVIEW_LINES)
            .map(|lines| (SharedString::from(lines.to_string()), SharedString::from(preview::lines_label(lines))))
            .collect();
        let preview_field = {
            let (get, set) = (weak.clone(), weak.clone());
            SettingField::dropdown(
                preview_options,
                move |cx| {
                    let lines = get.read_with(cx, |this, _| this.preview_lines).unwrap_or_default();
                    SharedString::from(lines.to_string())
                },
                move |value: SharedString, cx| {
                    if let Ok(lines) = value.parse::<u8>() {
                        set.update(cx, |this, cx| this.set_preview_lines(lines, cx)).ok();
                    }
                },
            )
        };
        let follow_up_field = {
            let (get, set) = (weak.clone(), weak.clone());
            SettingField::number_input(
                NumberFieldOptions {
                    min: f64::from(*FOLLOW_UP_DAYS.start()),
                    max: f64::from(*FOLLOW_UP_DAYS.end()),
                    step: 1.,
                },
                move |cx| get.read_with(cx, |this, _| f64::from(this.follow_up_days)).unwrap_or_default(),
                move |days, cx| {
                    set.update(cx, |this, cx| this.set_follow_up(days, cx)).ok();
                },
            )
        };
        SettingPage::new("Inbox & Threads").resettable(false).group(
            SettingGroup::new()
                .item(
                    SettingItem::new(
                        "Group by thread",
                        switch(weak, |this| this.group, SettingsPanel::set_grouping),
                    )
                    .description("Show one inbox row per conversation instead of per message.")
                    .keywords(["inbox", "threads"]),
                )
                .item(
                    SettingItem::new("Preview lines", preview_field)
                        .description("Snippet lines shown under each subject in the inbox.")
                        .keywords(["message snippet", "inbox"]),
                )
                .item(
                    SettingItem::new("Follow-up after", follow_up_field)
                        .description("Days to wait for a reply before flagging (1–14).")
                        .keywords(["follow up", "wait for a response", "flag", "days"]),
                ),
        )
    }

    fn blocked_page(&self, weak: &Weak) -> SettingPage {
        let keywords = ["blocked senders", "blocked", "unblock", "unsubscribed", "resubscribe", "senders"];
        let mut blocked = SettingGroup::new().title("Blocked senders");
        if self.blocked.is_empty() {
            blocked = blocked.item(note("blocked-empty", "No blocked senders.").keywords(keywords));
        }
        for (index, email) in self.blocked.iter().cloned().enumerate() {
            let weak = weak.clone();
            let label = email.clone();
            blocked = blocked.item(
                SettingItem::render(move |_, _, cx| {
                    let (weak, email) = (weak.clone(), email.clone());
                    let t = theme::active(cx);
                    div()
                        .id(("blocked-row", index))
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .text_sm()
                        .child(div().flex().flex_col().child(email.clone()).child(
                            div().text_xs().text_color(t.text_muted).child("Blocked sender"),
                        ))
                        .child(
                            button(("blocked-unblock", index), "Unblock", "Allow mail from this sender again", "", cx)
                                .on_click(move |_, _, cx| {
                                    weak.update(cx, |this, cx| this.unblock(&email, cx)).ok();
                                }),
                        )
                })
                .keywords(keywords.into_iter().chain([label.as_str()])),
            );
        }
        let mut unsubscribed = SettingGroup::new()
            .title("Unsubscribed")
            .description("Read-only: resubscribing happens at the original subscription.");
        if self.unsubscribed.is_empty() {
            unsubscribed = unsubscribed.item(note("unsubscribed-empty", "None.").keywords(keywords));
        }
        for (index, email) in self.unsubscribed.iter().cloned().enumerate() {
            let label = email.clone();
            unsubscribed = unsubscribed.item(
                SettingItem::render(move |_, _, _| {
                    div().id(("unsubscribed-row", index)).text_sm().child(email.clone())
                })
                .keywords(keywords.into_iter().chain([label.as_str()])),
            );
        }
        SettingPage::new("Blocked senders").resettable(false).groups([blocked, unsubscribed])
    }

    fn classifier_page(&self, weak: &Weak) -> SettingPage {
        let groups = QuestionKey::ALL.into_iter().map(|question| {
            let mode_field = {
                let (get, set) = (weak.clone(), weak.clone());
                SettingField::dropdown(
                    options(&CLASSIFIER_MODES),
                    move |cx| {
                        let review = get
                            .read_with(cx, |this, _| matches!(this.policy.mode(question), Mode::Review))
                            .unwrap_or_default();
                        pick(&CLASSIFIER_MODES, usize::from(review))
                    },
                    move |value: SharedString, cx| {
                        set.update(cx, |this, cx| this.set_classifier_mode(question, value == "review", cx)).ok();
                    },
                )
            };
            let threshold_field = {
                let (get, set) = (weak.clone(), weak.clone());
                SettingField::number_input(
                    NumberFieldOptions { min: 0., max: 100., step: 5. },
                    move |cx| {
                        get.read_with(cx, |this, _| f64::from((this.policy.threshold(question) * 100.).round()))
                            .unwrap_or_default()
                    },
                    move |percent, cx| {
                        set.update(cx, |this, cx| this.set_threshold(question, percent, cx)).ok();
                    },
                )
            };
            let review = matches!(self.policy.mode(question), Mode::Review);
            SettingGroup::new().title(question.label()).items([
                SettingItem::new("Handling", mode_field)
                    .description("Choose automatic handling or manual review.")
                    .keywords([question.label(), "auto apply or review", "classifier"]),
                SettingItem::new("Confidence threshold", threshold_field)
                    .description("Minimum confidence (%) required for automatic handling.")
                    .keywords([question.label(), "classifier confidence threshold"])
                    .disabled(review),
            ])
        });
        SettingPage::new("Classifier").resettable(false).groups(groups)
    }
}

/// A switch field backed by a panel bool and its setter.
fn switch(
    weak: &Weak,
    get: fn(&SettingsPanel) -> bool,
    set: fn(&mut SettingsPanel, bool, &mut Context<SettingsPanel>),
) -> SettingField<bool> {
    let (reader, writer) = (weak.clone(), weak.clone());
    SettingField::switch(
        move |cx| reader.read_with(cx, |this, _| get(this)).unwrap_or_default(),
        move |on, cx| {
            writer.update(cx, |this, cx| set(this, on, cx)).ok();
        },
    )
}

fn options(pairs: &[(&'static str, &'static str)]) -> Vec<(SharedString, SharedString)> {
    pairs.iter().map(|(value, label)| ((*value).into(), (*label).into())).collect()
}

fn pick(pairs: &[(&'static str, &'static str)], index: usize) -> SharedString {
    pairs[index].0.into()
}

/// A muted one-line placeholder row.
fn note(id: &'static str, text: &'static str) -> SettingItem {
    SettingItem::render(move |_, _, cx| {
        div().id(id).text_sm().text_color(theme::active(cx).text_muted).child(text)
    })
}

impl Focusable for SettingsPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl EventEmitter<SettingsEvent> for SettingsPanel {}

impl Render for SettingsPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::active(cx);
        let weak = cx.entity().downgrade();
        let pages = self.pages(&weak, cx);
        let height = fit_height(window, top_offset(window));
        div()
            .key_context(SETTINGS_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|_, _: &SettingsClose, _, cx| cx.emit(SettingsEvent::Close)))
            .flex()
            .flex_col()
            .w(px(fit_width(window, 760.)))
            .h(px(height))
            .p_3()
            .gap_2()
            .bg(t.surface)
            .border_1()
            .border_color(t.border)
            .rounded_md()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_sm().text_color(t.accent).child("Settings"))
                    .child(
                        button("settings-close", "Close", "Close", "escape", cx)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::Close))),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(Settings::new("settings").sidebar_width(px(190.)).pages(pages)),
            )
    }
}
