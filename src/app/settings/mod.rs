//! Settings window panel. Its entity owns transient page state and emits
//! [`SettingsEvent`]s for changes applied live by the mail window.
use gpui_kit::prelude::*;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::ButtonVariant;
use std::collections::HashMap;
use crate::app::actions::{SettingsDismiss, SettingsEnterBody, SettingsNextPage, SettingsPrevPage, SettingsSearch};

use crate::app::mail_app::panes::Orientation;
use crate::app::ui::button;
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

/// Binding predicate for the settings sidebar keys: the panel, but not while a text field
/// has focus (a focused kit `Input` sets context `Input`).
pub const SETTINGS_NAV: &str = "SettingsPanel && !Input && !PopupMenu";
/// Same, for the keys that may fire from inside a field: ⌘F and Esc.
pub const SETTINGS_SCOPE: &str = "SettingsPanel && !PopupMenu";

#[derive(Clone, Debug)]
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
    /// The user picked what the account's sync status allows.
    SyncAction { id: String, action: crate::sync_status::SyncAction },
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
    /// Resolved icon key, one of `account_style::ICONS`.
    pub icon: &'static str,
    /// Raw nickname text; blank means unset (see `account_style::normalize_nickname`).
    pub nickname: String,
    /// Linked Gmail account (removable, has a sync status).
    pub gmail: bool,
    /// Sync status line (see [`crate::sync_status::describe`]) and whether it reports a problem.
    pub sync: String,
    pub sync_problem: bool,
    /// The single action this status allows, or `None` while it is healthy.
    pub sync_action: Option<crate::sync_status::SyncAction>,
}

/// Index of the Accounts page in [`SettingsPanel::pages`].
const ACCOUNTS_PAGE: usize = 0;

const CLASSIFIER_MODES: [(&str, &str); 3] = [("auto", "Auto"), ("review", "Review"), ("off", "Off")];
const CONFIDENCES: [(&str, &str); 3] = [("high", "High"), ("medium", "Medium"), ("low", "Low")];
const MAX_PREVIEW_LINES: u8 = 5;
const FOLLOW_UP_DAYS: std::ops::RangeInclusive<u8> = 1..=14;

type Weak = WeakEntity<SettingsPanel>;

/// Per-account widgets of the Accounts page; the subscription reports their changes.
struct AccountControls {
    nickname: Entity<InputState>,
    _sub: Subscription,
}

pub struct SettingsPanel {
    /// The sidebar itself, focused when the window opens: ↑/↓ walk the pages from here and
    /// `→` steps into the page content.
    focus: FocusHandle,
    /// Stepping stone painted just in front of the kit sidebar. The kit owns its search
    /// field, so ⌘F and `/` focus this and step once onto it.
    search_anchor: FocusHandle,
    /// The field ⌘F and `/` landed on, so Esc can tell a typed query from an empty one.
    search_field: Option<FocusHandle>,
    /// Whether the search field has taken text since it was focused.
    query_typed: bool,
    /// Page on screen. The kit keeps its own copy of the selection, so a keyboard page change
    /// rebuilds it through [`Self::page_gen`].
    page: usize,
    /// Bumped with every page change and part of the kit `Settings` key, so the rebuilt
    /// sidebar opens on [`Self::page`] with an empty query.
    page_gen: u32,
    policy: JudgePolicy,
    summaries: bool,
    group: bool,
    tab_avatars: bool,
    preview_lines: u8,
    orientation: Orientation,
    theme_mode: theme::ThemeMode,
    light_theme: String,
    dark_theme: String,
    /// Blocked sender addresses with the time each was blocked, sorted;
    /// `Unblock` removes one and emits [`SettingsEvent::Unblock`].
    blocked: Vec<(String, Timestamp)>,
    /// Address filter for the Senders page.
    filter: String,
    /// The filter's input (with its subscription).
    filter_input: Entity<InputState>,
    _filter_sub: Subscription,
    /// The app's clock when the window opened, for relative dates.
    now: Timestamp,
    /// Days to wait for a reply before flagging a thread.
    follow_up_days: u8,
    /// Number input behind the follow-up row.
    follow_up_input: Entity<InputState>,
    /// Text last read from [`Self::follow_up_input`].
    follow_up_text: String,
    /// Set when [`Self::follow_up_text`] changed outside the input (a reset or a reseed);
    /// the next render writes it into the field.
    follow_up_write_back: bool,
    /// Keeps the follow-up input and the panel's day count in step.
    _follow_up_sub: Subscription,
    accounts: Vec<AccountRow>,
    /// Nickname input per account (with its subscription).
    controls: HashMap<String, AccountControls>,
    /// Gmail OAuth client credentials are present in the environment.
    gmail_configured: bool,
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
        let focus = cx.focus_handle();
        let search_anchor = cx.focus_handle().tab_stop(false);
        let follow_up_days = default_follow_up_days();
        let follow_up_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(follow_up_days.to_string())
                .step(1.)
                .min(f64::from(*FOLLOW_UP_DAYS.start()))
                .max(f64::from(*FOLLOW_UP_DAYS.end()))
        });
        let follow_up_sub = cx.subscribe(&follow_up_input, |this, input, event: &InputEvent, cx| {
            if !matches!(event, InputEvent::Change) {
                return;
            }
            let text = input.read(cx).value().to_string();
            if text != this.follow_up_text {
                this.follow_up_text = text.clone();
                // Half-typed values ("", "1.") are kept in the field but change nothing.
                if let Ok(days) = text.parse::<f64>() {
                    this.set_follow_up(days, cx);
                }
            }
            cx.notify();
        });
        let filter_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Filter by address"));
        let filter_sub = cx.subscribe(&filter_input, |this, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let text = input.read(cx).value().to_string();
                this.set_filter(text, cx);
            }
        });
        Self {
            focus,
            search_anchor,
            search_field: None,
            query_typed: false,
            page: ACCOUNTS_PAGE,
            page_gen: 0,
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
            filter: String::new(),
            filter_input,
            _filter_sub: filter_sub,
            now: 0,
            accounts: Vec::new(),
            controls: HashMap::new(),
            gmail_configured: false,
            follow_up_days,
            follow_up_input,
            follow_up_text: follow_up_days.to_string(),
            follow_up_write_back: false,
            _follow_up_sub: follow_up_sub,
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

    /// The page on screen, as the sidebar has it.
    pub fn page(&self) -> usize {
        self.page
    }

    /// Open on the Accounts page.
    pub fn on_accounts_page(mut self) -> Self {
        self.page = ACCOUNTS_PAGE;
        self
    }

    /// Move the sidebar selection a page down (`step` = 1) or up (`step` = -1), but only
    /// while the sidebar itself has focus. The kit owns its own selection and exposes no
    /// setter, so the tree is rebuilt on the new page.
    fn step_page(&mut self, step: isize, window: &Window, cx: &mut Context<Self>) {
        if !self.nav_has_focus(window, cx) {
            return;
        }
        let weak = cx.entity().downgrade();
        let count = self.pages(&weak, cx).len() as isize;
        let next = (self.page as isize + step).rem_euclid(count.max(1)) as usize;
        if next != self.page {
            self.page = next;
            self.rebuild(cx);
        }
    }

    /// Rebuild the kit's settings, which starts it on the current page with an empty query.
    fn rebuild(&mut self, cx: &mut Context<Self>) {
        self.page_gen = self.page_gen.wrapping_add(1);
        self.search_field = None;
        self.query_typed = false;
        cx.notify();
    }

    /// Whether the sidebar, and not a field or a control inside the page, has focus.
    fn nav_has_focus(&self, window: &Window, cx: &App) -> bool {
        window.focused(cx).is_some_and(|focused| focused == self.focus)
    }

    /// Step into the page content, the way Tab does.
    fn enter_body(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus_next(cx);
    }

    /// Focus the search field the kit renders on top of its sidebar.
    fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.search_anchor, cx);
        window.focus_next(cx);
        self.search_field = window.focused(cx);
        self.query_typed = false;
    }

    /// Whether the search field has focus, which is what makes Esc clear it rather than close.
    fn search_focused(&self, window: &Window, cx: &App) -> bool {
        self.search_field
            .as_ref()
            .is_some_and(|field| Some(field) == window.focused(cx).as_ref())
    }

    /// Note text going into the search field, which is what makes Esc clear it.
    fn note_typing(&mut self, event: &gpui_kit::KeyDownEvent, window: &Window, cx: &App) {
        if event.keystroke.key_char.is_some() && self.search_focused(window, cx) {
            self.query_typed = true;
        }
    }

    /// Esc: clear a search that has text in it, otherwise close the window.
    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.query_typed && self.search_focused(window, cx) {
            self.rebuild(cx);
            window.focus(&self.focus, cx);
        } else {
            cx.emit(SettingsEvent::Close);
        }
    }

    /// Seed the blocked-sender list, the app clock it dates rows against and the
    /// follow-up timeout (seconds).
    pub fn mailbox_state(
        mut self,
        blocked: Vec<(String, Timestamp)>,
        now: Timestamp,
        follow_up_timeout: Timestamp,
    ) -> Self {
        self.blocked = blocked;
        self.now = now;
        self.follow_up_days = follow_up_timeout
            .div_euclid(DAY)
            .clamp((*FOLLOW_UP_DAYS.start()).into(), (*FOLLOW_UP_DAYS.end()).into())
            as u8;
        self.follow_up_text = self.follow_up_days.to_string();
        self.follow_up_write_back = true;
        self
    }

    /// Re-read the blocked senders and the clock (undo restores a block).
    pub fn set_blocked(&mut self, blocked: Vec<(String, Timestamp)>, now: Timestamp, cx: &mut Context<Self>) {
        if self.blocked != blocked || self.now != now {
            self.blocked = blocked;
            self.now = now;
            cx.notify();
        }
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
        self.light_theme = crate::app_settings::DEFAULT_LIGHT_THEME.to_owned();
        self.dark_theme = crate::app_settings::DEFAULT_DARK_THEME.to_owned();
        self.follow_up_days = default_follow_up_days();
        self.follow_up_text = self.follow_up_days.to_string();
        self.follow_up_write_back = true;
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
        self.set_mode(question, mode, cx);
    }

    /// Route `question` by `mode`, persisting the policy when it changes.
    fn set_mode(&mut self, question: QuestionKey, mode: Mode, cx: &mut Context<Self>) {
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

    /// Narrow the Senders page to addresses containing `text`.
    fn set_filter(&mut self, text: String, cx: &mut Context<Self>) {
        if self.filter != text {
            self.filter = text;
            cx.notify();
        }
    }

    /// Blocked senders whose address contains the filter text.
    fn filtered_blocked(&self) -> Vec<&(String, Timestamp)> {
        let needle = self.filter.trim().to_lowercase();
        self.blocked
            .iter()
            .filter(|(email, _)| needle.is_empty() || email.to_lowercase().contains(&needle))
            .collect()
    }

    /// Drop `email` from the local list and ask the app to unblock it.
    fn unblock(&mut self, email: &str, cx: &mut Context<Self>) {
        let before = self.blocked.len();
        self.blocked.retain(|(blocked, _)| blocked != email);
        if self.blocked.len() != before {
            cx.emit(SettingsEvent::Unblock(email.to_owned()));
            cx.notify();
        }
    }

    /// Replace the account list (after a sign-in, removal or sync change).
    pub fn set_accounts(&mut self, accounts: Vec<AccountRow>, cx: &mut Context<Self>) {
        self.accounts = accounts;
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


    /// Whether the follow-up row differs from its default (the reset button's condition).
    fn follow_up_is_modified(&self) -> bool {
        self.follow_up_days != default_follow_up_days()
    }

    /// Restore the default follow-up timeout.
    fn reset_follow_up(&mut self, cx: &mut Context<Self>) {
        self.follow_up_days = default_follow_up_days();
        self.follow_up_text = self.follow_up_days.to_string();
        self.follow_up_write_back = true;
        cx.emit(SettingsEvent::FollowUp(i64::from(self.follow_up_days) * DAY));
        cx.notify();
    }

    /// The text the follow-up field must show, handed out once per change so later renders
    /// leave the field alone while it is being typed in.
    fn take_follow_up_write_back(&mut self) -> Option<String> {
        std::mem::take(&mut self.follow_up_write_back).then(|| self.follow_up_text.clone())
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

/// The default number of days to wait for a reply, clamped to the row's range.
fn default_follow_up_days() -> u8 {
    (crate::model::DEFAULT_FOLLOW_UP_TIMEOUT / DAY)
        .clamp(i64::from(*FOLLOW_UP_DAYS.start()), i64::from(*FOLLOW_UP_DAYS.end()))
        as u8
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

/// A muted one-line placeholder row.
fn note(id: &'static str, text: &'static str) -> SettingItem {
    SettingItem::render(move |_, _, cx| {
        div().id(id).text_sm().text_color(cx.theme().muted_foreground).child(text)
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
        // The kit keys its own state by element id, so the page and query it starts with
        // follow `page_gen`.
        let settings = format!("settings-{}", self.page_gen);
        let close = button("settings-close", "Close", "Close the settings window", "escape", cx)
            .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::Close)));
        div()
            .key_context(SETTINGS_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &SettingsNextPage, window, cx| this.step_page(1, window, cx)))
            .on_action(cx.listener(|this, _: &SettingsPrevPage, window, cx| this.step_page(-1, window, cx)))
            .on_action(cx.listener(|this, _: &SettingsEnterBody, window, cx| this.enter_body(window, cx)))
            .on_action(cx.listener(|this, _: &SettingsSearch, window, cx| this.focus_search(window, cx)))
            .on_action(cx.listener(|this, _: &SettingsDismiss, window, cx| this.dismiss(window, cx)))
            // Capture phase, so text swallowed by the search field is still seen.
            .capture_key_down(cx.listener(|this, event, window, cx| this.note_typing(event, window, cx)))
            .flex()
            .flex_col()
            .size_full()
            .p_3()
            .gap_2()
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    // Stepping stone in front of the kit sidebar, so ⌘F and `/` reach its
                    // search field without counting the stops above it.
                    .child(div().size(px(0.)).flex_none().track_focus(&self.search_anchor))
                    .child(
                        div().flex_1().min_h_0().child(
                            Settings::new(settings)
                                .sidebar_width(px(190.))
                                .default_selected_index(SelectIndex { page_ix: self.page, group_ix: None })
                                .pages(pages),
                        ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .justify_between()
                    .border_t_1()
                    .border_color(t.border)
                    .pt_2()
                    .child(reset)
                    .child(close),
            )
    }
}
