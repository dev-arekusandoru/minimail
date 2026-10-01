//! Settings panel: gpui-kit's `Settings` component (sidebar pages with search, grouped setting
//! items) hosted in a modal. The panel entity owns the settings state and emits
//! [`SettingsEvent`]s; the component's field getters/setters reach it through a `WeakEntity`.
use gpui_kit::prelude::*;
use gpui_kit::assets::IconName;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Selectable as _;
use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::popover::Popover;
use std::collections::HashMap;

use crate::account_style;
use crate::app::icons;
use crate::app::mail_app::panes::Orientation;
use crate::app::ui::{button, icon_button};
use crate::clock::{DAY, Timestamp};
use crate::judge::{Confidence, JudgePolicy, Mode, QuestionKey};
use crate::{preview, theme};
use gpui_kit::{
    component::setting::{
        NumberFieldOptions, SelectIndex, SettingField, SettingGroup, SettingItem, SettingPage, Settings,
    },
    *,
};

/// Key context of the panel.
pub const SETTINGS_CONTEXT: &str = "SettingsPanel";

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
    /// Start the Gmail sign-in flow.
    AddGmail,
    /// Remove a linked account (confirmed in the panel).
    RemoveAccount(String),
    /// An account's icon key and `#rrggbb` color changed (`mailbox.set_account_style`).
    AccountStyle { id: String, icon: &'static str, color: String },
    /// An account's nickname text changed (`mailbox.set_account_nickname`; blank clears it).
    AccountNickname { id: String, nickname: String },
    Close,
}

/// One row of the Accounts page.
/// Shown beside "Add Gmail…" when the OAuth client is not configured.
pub const GMAIL_ENV_HINT: &str =
    "Needs MAIL_CLASSIFIER_GOOGLE_CLIENT_ID and MAIL_CLASSIFIER_GOOGLE_CLIENT_SECRET set at launch.";

#[derive(Clone, Debug)]
pub struct AccountRow {
    pub id: String,
    pub name: String,
    pub email: String,
    pub color: String,
    /// Resolved icon key (see [`account_style::ICONS`]).
    pub icon: &'static str,
    /// Raw nickname text; blank means unset (see [`account_style::normalize_nickname`]).
    pub nickname: String,
    /// Linked Gmail account (removable, has a sync status).
    pub gmail: bool,
    /// Sync status line (see [`crate::sync_status::describe`]) and whether it reports a problem.
    pub sync: String,
    pub sync_problem: bool,
}

/// Index of the Accounts page in [`SettingsPanel::pages`].
const ACCOUNTS_PAGE: usize = 5;

const CLASSIFIER_MODES: [(&str, &str); 3] = [("auto", "Auto"), ("review", "Review"), ("off", "Off")];
const CONFIDENCES: [(&str, &str); 3] = [("high", "High"), ("medium", "Medium"), ("low", "Low")];
const PANE_LAYOUTS: [(&str, &str); 2] = [("side", "Side by side"), ("stacked", "Stacked")];
const MAX_PREVIEW_LINES: u8 = 5;
const FOLLOW_UP_DAYS: std::ops::RangeInclusive<u8> = 1..=14;

type Weak = WeakEntity<SettingsPanel>;

/// Per-account widgets of the Accounts page; the subscription reports their changes.
struct AccountControls {
    nickname: Entity<InputState>,
    _sub: Subscription,
}

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
    accounts: Vec<AccountRow>,
    /// Nickname input per account (with its subscription).
    controls: HashMap<String, AccountControls>,
    /// Account whose Remove button has been clicked once (awaiting confirm).
    confirm_remove: Option<String>,
    /// Gmail OAuth client credentials are present in the environment.
    gmail_configured: bool,
    /// Page selected when the panel opens.
    start_page: usize,
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
            accounts: Vec::new(),
            controls: HashMap::new(),
            confirm_remove: None,
            gmail_configured: false,
            start_page: 0,
            follow_up_days: (crate::model::DEFAULT_FOLLOW_UP_TIMEOUT / DAY)
                .clamp((*FOLLOW_UP_DAYS.start()).into(), (*FOLLOW_UP_DAYS.end()).into()) as u8,
        }
    }

    /// Whether tabs show the sender's avatar (the row's initial value).
    pub fn tab_avatars(mut self, on: bool) -> Self {
        self.tab_avatars = on;
        self
    }

    /// Seed the Accounts page: the account list and whether Gmail sign-in is configured.
    pub fn accounts(mut self, accounts: Vec<AccountRow>, gmail_configured: bool) -> Self {
        self.accounts = accounts;
        self.gmail_configured = gmail_configured;
        self
    }

    /// Open on the Accounts page instead of General.
    pub fn on_accounts_page(mut self) -> Self {
        self.start_page = ACCOUNTS_PAGE;
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

    fn set_confidence(&mut self, question: QuestionKey, confidence: Confidence, cx: &mut Context<Self>) {
        if matches!(self.policy.mode(question), Mode::Auto(c) if c != confidence) {
            self.policy.set_mode(question, Mode::Auto(confidence));
            self.changed(cx);
        }
    }

    fn set_classifier_mode(&mut self, question: QuestionKey, value: &str, cx: &mut Context<Self>) {
        let mode = match (value, self.policy.mode(question)) {
            ("auto", mode @ Mode::Auto(_)) => mode,
            ("auto", _) => Mode::Auto(Confidence::Medium),
            ("off", _) => Mode::Off,
            _ => Mode::Review,
        };
        if self.policy.mode(question) != mode {
            self.policy.set_mode(question, mode);
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

    /// Replace the account list (after a sign-in, removal or sync change); a pending remove
    /// confirm survives only while its account is still listed.
    pub fn set_accounts(&mut self, accounts: Vec<AccountRow>, cx: &mut Context<Self>) {
        self.accounts = accounts;
        if let Some(id) = &self.confirm_remove
            && !self.accounts.iter().any(|a| &a.id == id)
        {
            self.confirm_remove = None;
        }
        cx.notify();
    }

    /// Apply a picked icon and/or color to the account row and tell the app.
    fn set_account_style(&mut self, id: &str, icon: Option<&'static str>, color: Option<String>, cx: &mut Context<Self>) {
        let Some(row) = self.accounts.iter_mut().find(|a| a.id == id) else {
            return;
        };
        let new_icon = icon.unwrap_or(row.icon);
        let new_color = color.unwrap_or_else(|| row.color.clone());
        if new_icon == row.icon && new_color.eq_ignore_ascii_case(&row.color) {
            return;
        }
        row.icon = new_icon;
        row.color = new_color.clone();
        cx.emit(SettingsEvent::AccountStyle { id: id.to_owned(), icon: new_icon, color: new_color });
        cx.notify();
    }

    /// Give every account a nickname input; drop those of removed accounts.
    fn sync_controls(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.controls.retain(|id, _| self.accounts.iter().any(|a| &a.id == id));
        let missing: Vec<(String, String, String)> = self
            .accounts
            .iter()
            .filter(|a| !self.controls.contains_key(&a.id))
            .map(|a| (a.id.clone(), a.name.clone(), a.nickname.clone()))
            .collect();
        for (id, name, nickname) in missing {
            let input = cx.new(|cx| {
                let mut input = InputState::new(window, cx).placeholder(name);
                input.set_value(nickname, window, cx);
                input
            });
            let account = id.clone();
            let sub = cx.subscribe(&input, move |this, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let text = input.read(cx).value().to_string();
                    this.set_account_nickname(&account, &text, cx);
                }
            });
            self.controls.insert(id, AccountControls { nickname: input, _sub: sub });
        }
    }

    /// Apply a typed nickname to the account row and tell the app (blank clears it).
    fn set_account_nickname(&mut self, id: &str, text: &str, cx: &mut Context<Self>) {
        let Some(row) = self.accounts.iter_mut().find(|a| a.id == id) else {
            return;
        };
        if row.nickname == text {
            return;
        }
        row.nickname = text.to_owned();
        cx.emit(SettingsEvent::AccountNickname { id: id.to_owned(), nickname: text.to_owned() });
        cx.notify();
    }

    /// First click on Remove arms the confirm; the second emits the removal.
    fn remove_account(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.confirm_remove.as_deref() == Some(id) {
            self.confirm_remove = None;
            cx.emit(SettingsEvent::RemoveAccount(id.to_owned()));
        } else {
            self.confirm_remove = Some(id.to_owned());
        }
        cx.notify();
    }

    fn pages(&self, weak: &Weak, cx: &App) -> Vec<SettingPage> {
        vec![
            self.general_page(weak),
            self.appearance_page(weak, cx),
            self.inbox_page(weak),
            self.blocked_page(weak),
            self.classifier_page(weak),
            self.accounts_page(weak),
        ]
    }

    fn accounts_page(&self, weak: &Weak) -> SettingPage {
        let keywords = ["accounts", "gmail", "add account", "remove account", "sign in", "email"];
        let mut linked = SettingGroup::new();
        if self.accounts.is_empty() {
            linked = linked.item(note("accounts-empty", "No accounts.").keywords(keywords));
        }
        for (index, account) in self.accounts.iter().cloned().enumerate() {
            let nickname_input = self.controls.get(&account.id).map(|c| c.nickname.clone());
            let weak = weak.clone();
            let label = account.email.clone();
            let confirming = self.confirm_remove.as_deref() == Some(account.id.as_str());
            linked = linked.item(
                SettingItem::render(move |_, _, cx| account_card(index, &account, nickname_input.as_ref(), confirming, &weak, cx))
                    .keywords(keywords.into_iter().chain([label.as_str()])),
            );
        }
        let configured = self.gmail_configured;
        let weak = weak.clone();
        let add = SettingItem::render(move |_, _, cx| {
            let weak = weak.clone();
            let hint = if configured {
                "Sign in with your browser; the token is kept in the system keychain."
            } else {
                GMAIL_ENV_HINT
            };
            div()
                .id("account-add")
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .text_sm()
                .child(
                    div().flex().flex_col().flex_1().min_w_0().child("Add Gmail account").child(
                        div().text_xs().text_color(cx.theme().muted_foreground).child(hint),
                    ),
                )
                .child(
                    div().flex_none().child(
                        button("account-add-gmail", "Add Gmail…", "Sign in to a Gmail account", "", cx)
                            .on_click(move |_, _, cx| {
                                weak.update(cx, |_, cx| cx.emit(SettingsEvent::AddGmail)).ok();
                            }),
                    ),
                )
        })
        .keywords(keywords);
        SettingPage::new("Accounts")
            .resettable(false)
            .groups([linked, SettingGroup::new().title("Add account").item(add)])
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
        let themes = theme::names(cx).into_iter().map(|name| (name.clone(), name)).collect();
        let theme_field = {
            let weak = weak.clone();
            SettingField::dropdown(
                themes,
                |cx| cx.theme().theme_name().clone(),
                move |name: SharedString, cx| {
                    theme::apply(cx, &name);
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
                    let t = cx.theme();
                    div()
                        .id(("blocked-row", index))
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .text_sm()
                        .child(div().flex().flex_col().child(email.clone()).child(
                            div().text_xs().text_color(t.muted_foreground).child("Blocked sender"),
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
                        let index = get
                            .read_with(cx, |this, _| match this.policy.mode(question) {
                                Mode::Auto(_) => 0,
                                Mode::Review => 1,
                                Mode::Off => 2,
                            })
                            .unwrap_or_default();
                        pick(&CLASSIFIER_MODES, index)
                    },
                    move |value: SharedString, cx| {
                        set.update(cx, |this, cx| this.set_classifier_mode(question, &value, cx)).ok();
                    },
                )
            };
            let confidence_field = {
                let (get, set) = (weak.clone(), weak.clone());
                SettingField::dropdown(
                    options(&CONFIDENCES),
                    move |cx| {
                        let index = get
                            .read_with(cx, |this, _| match this.policy.mode(question) {
                                Mode::Auto(Confidence::High) => 0,
                                Mode::Auto(Confidence::Low) => 2,
                                _ => 1,
                            })
                            .unwrap_or_default();
                        pick(&CONFIDENCES, index)
                    },
                    move |value: SharedString, cx| {
                        let confidence = match &*value {
                            "high" => Confidence::High,
                            "low" => Confidence::Low,
                            _ => Confidence::Medium,
                        };
                        set.update(cx, |this, cx| this.set_confidence(question, confidence, cx)).ok();
                    },
                )
            };
            let auto = matches!(self.policy.mode(question), Mode::Auto(_));
            SettingGroup::new().title(question.label()).items([
                SettingItem::new("Handling", mode_field)
                    .description("Choose automatic handling, manual review, or off.")
                    .keywords([question.label(), "auto apply or review", "classifier"]),
                SettingItem::new("Confidence", confidence_field)
                    .description("How sure the classifier must be for automatic handling.")
                    .keywords([question.label(), "classifier confidence"])
                    .disabled(!auto),
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
        div().id(id).text_sm().text_color(cx.theme().muted_foreground).child(text)
    })
}

/// One account as a card: a style tile (opens the icon and color picker), the nickname as an
/// inline-editable title, the address, and a footer with the sync status and Remove.
fn account_card(
    index: usize,
    account: &AccountRow,
    nickname: Option<&Entity<InputState>>,
    confirming: bool,
    weak: &Weak,
    cx: &App,
) -> Stateful<Div> {
    let t = cx.theme();
    let color = theme::parse_color(&account.color).unwrap_or(t.primary);
    let tile = Button::new(("account-style", index))
        .custom(
            ButtonCustomVariant::new(cx)
                .color(color.opacity(0.16))
                .hover(color.opacity(0.28))
                .active(color.opacity(0.36))
                .foreground(color),
        )
        .icon(icons::account_icon(account.icon, &account.color, t, 26.))
        .w(px(52.))
        .h(px(52.))
        .rounded(px(12.))
        .accessibility_label("Account icon and color");
    let picker = {
        let (weak, id, icon, selected_color) = (weak.clone(), account.id.clone(), account.icon, account.color.clone());
        Popover::new(("account-picker", index)).trigger(tile).content(move |_, _, cx| {
            let t = cx.theme();
            let icon_buttons = account_style::ICONS.iter().enumerate().map(|(i, (key, label))| {
                let (weak, id, key) = (weak.clone(), id.clone(), *key);
                let name = icons::account_icon_name(key).unwrap_or(IconName::Mail);
                icon_button(("account-icon", index * account_style::ICONS.len() + i), name, label, "", cx)
                    .selected(key == icon)
                    .on_click(move |_, _, cx| {
                        weak.update(cx, |this, cx| this.set_account_style(&id, Some(key), None, cx)).ok();
                    })
            });
            let swatches = account_style::COLORS.iter().enumerate().filter_map(|(i, hex)| {
                let fill = theme::parse_color(hex)?;
                let (weak, id, hex) = (weak.clone(), id.clone(), *hex);
                let chosen = hex.eq_ignore_ascii_case(&selected_color);
                Some(
                    Button::new(("account-color", index * account_style::COLORS.len() + i))
                        .custom(ButtonCustomVariant::new(cx).color(fill).hover(fill).active(fill).foreground(t.background))
                        .when(chosen, |b| b.icon(IconName::Check))
                        .w(px(24.))
                        .h(px(24.))
                        .rounded(px(12.))
                        .border_2()
                        .border_color(if chosen { t.foreground } else { t.transparent })
                        .accessibility_label(format!("Color {hex}"))
                        .on_click(move |_, _, cx| {
                            weak.update(cx, |this, cx| this.set_account_style(&id, None, Some(hex.to_owned()), cx)).ok();
                        }),
                )
            });
            div()
                .flex()
                .flex_col()
                .gap_3()
                .w(px(232.))
                .child(div().flex().flex_wrap().gap_1().children(icon_buttons))
                .child(div().h(px(1.)).bg(t.border))
                .child(div().flex().flex_wrap().gap_1().children(swatches))
        })
    };
    let title = div().flex_1().min_w_0().text_base().font_weight(FontWeight::SEMIBOLD).children(
        nickname.map(|input| Input::new(input).id(("account-nickname", index)).bordered(false).focus_bordered(true)),
    );
    let header = div()
        .flex()
        .items_center()
        .gap_4()
        .p_4()
        .child(picker)
        .child(
            div().flex().flex_col().flex_1().min_w_0().gap_0p5().child(title).child(
                div().px_3().text_xs().text_color(t.muted_foreground).child(account.email.clone()),
            ),
        );
    let status_color = if account.sync_problem { t.danger } else { t.muted_foreground };
    let remove = account.gmail.then(|| {
        let (weak, id) = (weak.clone(), account.id.clone());
        let (text, tip) = if confirming {
            ("Confirm remove", "Removes the account and its cached mail from this app; Gmail is untouched")
        } else {
            ("Remove", "Remove this account from the app")
        };
        button(("account-remove", index), text, tip, "", cx)
            .ghost()
            .text_color(t.danger)
            .on_click(move |_, _, cx| {
                weak.update(cx, |this, cx| this.remove_account(&id, cx)).ok();
            })
    });
    let footer = div()
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .px_4()
        .py_2()
        .border_t_1()
        .border_color(t.border)
        .child(
            div()
                .id(("account-sync", index))
                .flex_1()
                .min_w_0()
                .truncate()
                .text_xs()
                .text_color(status_color)
                .child(account.sync.clone()),
        )
        .children(remove.map(|b| div().flex_none().child(b)));
    div()
        .id(("account-card", index))
        .flex()
        .flex_col()
        .rounded_lg()
        .border_1()
        .border_color(t.border)
        .bg(t.secondary)
        .child(header)
        .child(footer)
}

impl Focusable for SettingsPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl EventEmitter<SettingsEvent> for SettingsPanel {}

impl Render for SettingsPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_controls(window, cx);
        let t = cx.theme();
        let weak = cx.entity().downgrade();
        let pages = self.pages(&weak, cx);
        let height = f32::from(window.viewport_size().height) * 0.8;
        div()
            .key_context(SETTINGS_CONTEXT)
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .w_full()
            .h(px(height))
            .p_3()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_sm().text_color(t.primary).child("Settings"))
                    .child(
                        button("settings-close", "Close", "Close", "escape", cx)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::Close))),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(
                        Settings::new("settings")
                            .sidebar_width(px(190.))
                            .default_selected_index(SelectIndex { page_ix: self.start_page, group_ix: None })
                            .pages(pages),
                    ),
            )
    }
}
