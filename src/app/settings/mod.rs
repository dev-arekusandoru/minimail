//! Settings window panel. Its entity owns transient page state and emits
//! [`SettingsEvent`]s for changes applied live by the mail window.
use gpui_kit::prelude::*;
use gpui_kit::assets::IconName;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Selectable as _;
use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::ButtonVariant;
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

mod accounts;
mod appearance;
mod inbox;
mod ai;
mod senders;

/// Key context of the panel.
pub const SETTINGS_CONTEXT: &str = "SettingsPanel";

pub enum SettingsEvent {
    Changed(JudgePolicy, bool),
    Grouping(bool),
    TabAvatars(bool),
    PreviewLines(u8),
    Theme { mode: theme::ThemeMode, light: String, dark: String },
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
    ResetAll,
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
const ACCOUNTS_PAGE: usize = 0;

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
    theme_mode: theme::ThemeMode,
    light_theme: String,
    dark_theme: String,
    /// Blocked sender addresses, sorted; `Unblock` removes one and emits [`SettingsEvent::Unblock`].
    blocked: Vec<String>,
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
            theme_mode: theme::ThemeMode::System,
            light_theme: String::new(),
            dark_theme: crate::app_settings::DEFAULT_DARK_THEME.to_owned(),
            blocked: Vec::new(),
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
    pub fn theme_preferences(mut self, mode: theme::ThemeMode, light: String, dark: String) -> Self {
        self.theme_mode = mode;
        self.light_theme = light;
        self.dark_theme = dark;
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

    /// Seed the blocked-sender list and follow-up timeout (seconds).
    pub fn mailbox_state(mut self, blocked: Vec<String>, follow_up_timeout: Timestamp) -> Self {
        self.blocked = blocked;
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
    fn set_theme_mode(&mut self, mode: theme::ThemeMode, cx: &mut Context<Self>) {
        if self.theme_mode != mode {
            self.theme_mode = mode;
            self.emit_theme(cx);
        }
    }

    fn set_light_theme(&mut self, name: String, cx: &mut Context<Self>) {
        if self.light_theme != name {
            self.light_theme = name;
            self.emit_theme(cx);
        }
    }

    fn set_dark_theme(&mut self, name: String, cx: &mut Context<Self>) {
        if self.dark_theme != name {
            self.dark_theme = name;
            self.emit_theme(cx);
        }
    }

    fn emit_theme(&self, cx: &mut Context<Self>) {
        cx.emit(SettingsEvent::Theme {
            mode: self.theme_mode,
            light: self.light_theme.clone(),
            dark: self.dark_theme.clone(),
        });
        cx.notify();
    }
    pub(super) fn reset_fields(&mut self, cx: &mut Context<Self>) {
        self.policy = JudgePolicy::default();
        self.summaries = false;
        self.group = false;
        self.tab_avatars = true;
        self.preview_lines = preview::DEFAULT_LINES;
        self.orientation = Orientation::SideBySide;
        self.theme_mode = theme::ThemeMode::System;
        self.light_theme.clear();
        self.dark_theme = crate::app_settings::DEFAULT_DARK_THEME.to_owned();
        self.follow_up_days =
            (crate::model::DEFAULT_FOLLOW_UP_TIMEOUT / DAY).clamp(1, 14) as u8;
        cx.notify();
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
            self.accounts_page(weak),
            self.appearance_page(weak, cx),
            self.inbox_page(weak),
            self.classifier_page(weak),
            self.blocked_page(weak),
        ]
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
        let reset = button("settings-reset-all", "Reset all settings…", "Reset preferences to their defaults", "", cx)
            .on_click(move |_, window, cx| {
                let weak = weak.clone();
                window.open_alert_dialog(cx, move |alert, _, _| {
                    let weak = weak.clone();
                    alert
                        .title("Reset all settings?")
                        .description("This affects preferences only, not accounts or mail.")
                        .confirm()
                        .ok_text("Reset")
                        .ok_variant(ButtonVariant::Danger)
                        .cancel_text("Cancel")
                        .on_ok(move |_, _, cx| {
                            weak.update(cx, |this, cx| {
                                this.reset_fields(cx);
                                cx.emit(SettingsEvent::ResetAll);
                            })
                            .ok();
                            true
                        })
                })
            });
        div()
            .key_context(SETTINGS_CONTEXT)
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .size_full()
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
            .child(
                div()
                    .flex()
                    .justify_end()
                    .border_t_1()
                    .border_color(t.border)
                    .pt_2()
                    .child(reset),
            )
    }
}
