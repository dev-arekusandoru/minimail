//! Settings window panel. Its entity owns transient page state and emits
//! [`SettingsEvent`]s for changes applied live by the mail window.
use gpui_kit::prelude::*;
use gpui_kit::assets::IconName;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Selectable as _;
use gpui_kit::component::input::{Input, InputEvent, InputState, NumberInput};
use gpui_kit::component::menu::DropdownMenu as _;
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::component::label::Label;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::TitleBar;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariant, ButtonVariants as _};
use std::collections::HashMap;
use std::rc::Rc;
use gpui_kit::base::AxisExt as _;
use gpui_kit::base::{StyledExt as _, TestSupportExt as _, h_flex, v_flex};
use crate::app::actions::{SettingsDismiss, SettingsEnterBody, SettingsNextPage, SettingsPrevPage, SettingsSearch};

use crate::app::mail_app::panes::Orientation;
use crate::app::ui::icon_button;
use crate::clock::{DAY, Timestamp};
use crate::judge::{Confidence, JudgePolicy, Mode, QuestionKey};
use crate::{preview, theme};
use gpui_kit::{
    component::setting::{
        RenderOptions, SelectIndex, SettingGroup, SettingItem, SettingPage, Settings,
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

/// The range the preview-lines row accepts.
const PREVIEW_LINES: std::ops::RangeInclusive<u8> = 0..=MAX_PREVIEW_LINES;

type Weak = WeakEntity<SettingsPanel>;

/// Per-account widgets of the Accounts page; the subscription reports their changes.
struct AccountControls {
    nickname: Entity<InputState>,
    _sub: Subscription,
}

/// A row whose control is a number field: the input the user types in, the text last
/// read from it, and the flag that pushes the panel's value back after a reset.
struct NumberRow {
    input: Entity<InputState>,
    /// Text last read from `input`, so a render only rewrites what a reset changed.
    text: String,
    /// Set when `text` changed outside the input (a reset or a reseed).
    write_back: bool,
    /// Keeps the input and the panel's value in step.
    _sub: Subscription,
}

impl NumberRow {
    /// A number input seeded with `value` and bounded by `range`; every value it
    /// reports goes to `set`, which stores it on the panel.
    fn new(
        window: &mut Window,
        cx: &mut Context<SettingsPanel>,
        row: fn(&mut SettingsPanel) -> &mut NumberRow,
        value: u8,
        range: &std::ops::RangeInclusive<u8>,
        set: fn(&mut SettingsPanel, u8, &mut Context<SettingsPanel>),
    ) -> Self {
        let text = value.to_string();
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(text.clone())
                .step(1.)
                .min(f64::from(*range.start()))
                .max(f64::from(*range.end()))
        });
        let sub = cx.subscribe(&input, move |this, input, event: &InputEvent, cx| {
            if !matches!(event, InputEvent::Change) {
                return;
            }
            let typed = input.read(cx).value().to_string();
            let changed = {
                let field = row(this);
                let changed = typed != field.text;
                field.text = typed.clone();
                changed
            };
            if changed {
                // Half-typed values ("", "1.") are kept in the field but change nothing.
                if let Ok(days) = typed.parse::<f64>() {
                    set(this, days.round() as u8, cx);
                }
            }
            cx.notify();
        });
        Self { input, text, write_back: false, _sub: sub }
    }

    /// Put `value` in the field at the next render, as a reset or a reseed does.
    fn set(&mut self, value: u8) {
        self.text = value.to_string();
        self.write_back = true;
    }

    /// The text the field must show, handed out once per change so later renders
    /// leave the field alone while it is being typed in.
    fn take_write_back(&mut self) -> Option<SharedString> {
        std::mem::take(&mut self.write_back).then(|| self.text.clone().into())
    }
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
    /// The number field of the follow-up row.
    follow_up: NumberRow,
    /// The number field of the preview-lines row.
    preview: NumberRow,
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
        let follow_up = NumberRow::new(
            window,
            cx,
            SettingsPanel::follow_up_row,
            follow_up_days,
            &FOLLOW_UP_DAYS,
            SettingsPanel::set_follow_up,
        );
        let preview = NumberRow::new(
            window,
            cx,
            SettingsPanel::preview_row,
            preview_lines,
            &PREVIEW_LINES,
            SettingsPanel::set_preview_lines,
        );
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
            follow_up,
            preview,
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

    /// Step into the page content. The body starts at the kit's search field, which is
    /// what Tab reaches from the sidebar too; stepping by `focus_next` would instead
    /// land on whatever the window put before it, like the titlebar.
    fn enter_body(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_search(window, cx);
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
        self.follow_up.set(self.follow_up_days);
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

    fn set_follow_up(&mut self, days: u8, cx: &mut Context<Self>) {
        let days = days.clamp(*FOLLOW_UP_DAYS.start(), *FOLLOW_UP_DAYS.end());
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
        self.follow_up.set(self.follow_up_days);
        self.preview.set(self.preview_lines);
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

    /// Restore the per-thread inbox grouping.
    fn reset_grouping(&mut self, cx: &mut Context<Self>) {
        self.set_grouping(false, cx);
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

    /// The number field of the follow-up row.
    fn follow_up_row(&mut self) -> &mut NumberRow {
        &mut self.follow_up
    }

    /// The number field of the preview-lines row.
    fn preview_row(&mut self) -> &mut NumberRow {
        &mut self.preview
    }

    /// Whether the follow-up row differs from its default (its reset button's condition).
    fn follow_up_is_modified(&self) -> bool {
        self.follow_up_days != default_follow_up_days()
    }

    /// Restore the default follow-up timeout.
    fn reset_follow_up(&mut self, cx: &mut Context<Self>) {
        self.set_follow_up(default_follow_up_days(), cx);
        self.follow_up.set(self.follow_up_days);
    }

    /// Restore the default number of preview lines.
    fn reset_preview_lines(&mut self, cx: &mut Context<Self>) {
        self.set_preview_lines(preview::DEFAULT_LINES, cx);
        self.preview.set(self.preview_lines);
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

/// Reports whether one row differs from its default.
type IsDirty = Rc<dyn Fn(&App) -> bool>;
/// Puts one row's default back.
type Restore = Rc<dyn Fn(&mut SettingsPanel, &mut Context<SettingsPanel>)>;
/// Reads the value a select row shows.
type Read = Rc<dyn Fn(&App) -> SharedString>;
/// Stores what a select row picked.
type Write = Rc<dyn Fn(SharedString, &mut App)>;
/// Runs one row's restore from the app side, as the kit's reset handlers do.
type DoRestore = Rc<dyn Fn(&mut Window, &mut App)>;
/// Stores which segment of a segmented row was clicked.
type Apply = Rc<dyn Fn(&mut SettingsPanel, usize, &mut Context<SettingsPanel>)>;
/// Builds a row's control for one render pass.
type Control = Rc<dyn Fn(&RenderOptions, &mut Window, &mut App) -> AnyElement>;

/// What a row needs to be undone: `dirty` reports whether it differs from its default
/// and `restore` puts the default back. The same pair drives the row's own ↺ and the
/// page-level one.
#[derive(Clone)]
struct Undo {
    dirty: IsDirty,
    restore: DoRestore,
}

impl Undo {
    fn new(is_dirty: impl Fn(&App) -> bool + 'static, reset: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self { dirty: Rc::new(is_dirty), restore: Rc::new(reset) }
    }

    /// The undo of a row whose dirty check and reset are methods of the panel, so both
    /// may close over a question or a default.
    fn of_panel(
        weak: &Weak,
        is_dirty: impl Fn(&SettingsPanel) -> bool + 'static,
        reset: impl Fn(&mut SettingsPanel, &mut Context<SettingsPanel>) + 'static,
    ) -> Self {
        let (read, write) = (weak.clone(), weak.clone());
        let reset: Restore = Rc::new(reset);
        Self::new(
            move |cx| read.read_with(cx, |this, _| is_dirty(this)).unwrap_or_default(),
            move |_window, cx| {
                let reset = reset.clone();
                write.update(cx, move |this, cx| reset(this, cx)).ok();
            },
        )
    }

    /// The undo of a row that stores a value: a reset puts `default` back and emits the
    /// row's event through `set`.
    fn of_value<T: Clone + PartialEq + Into<SharedString> + 'static>(
        weak: &Weak,
        get: fn(&SettingsPanel) -> T,
        set: fn(&mut SettingsPanel, T, &mut Context<SettingsPanel>),
        default: T,
    ) -> Self {
        let (differs, restores) = (default.clone(), default);
        Self::of_panel(
            weak,
            move |this| get(this) != differs,
            move |this, cx| set(this, restores.clone(), cx),
        )
    }

    /// The quiet ↺ that follows the row's title while the row differs from its default.
    /// It restores this row alone, the way the page header's ↺ restores the page.
    fn button(self, options: &RenderOptions, cx: &App) -> Option<AnyElement> {
        if !(self.dirty)(cx) {
            return None;
        }
        let restore = self.restore.clone();
        Some(
            Button::new(format!(
                "row-undo-{}-{}-{}",
                options.page_ix(),
                options.group_ix(),
                options.item_ix()
            ))
            .icon(IconName::Undo2)
            .ghost()
            .xsmall()
            .tooltip("Reset to default")
            .accessibility_label("Reset to default")
            .on_click(move |_, window, cx| restore(window, cx))
            .into_any_element(),
        )
    }
}

/// A panel row: the title with its own ↺, the description under it, and `control` on the
/// right. The row's undo is registered with `on_reset`, so the page header's ↺ resets
/// this row too, and its title and description are searchable keywords.
fn row(
    title: impl Into<SharedString>,
    description: &'static str,
    keywords: &[&'static str],
    undo: Undo,
    control: Control,
) -> SettingItem {
    let title = title.into();
    let search: Vec<SharedString> = std::iter::once(title.clone())
        .chain(std::iter::once(SharedString::from(description)))
        .chain(keywords.iter().map(|keyword| SharedString::from(*keyword)))
        .collect();
    let page_reset = undo.clone();
    SettingItem::render(move |options, window, cx| {
        let control = (control)(options, window, cx);
        let undo_button = undo.clone().button(options, cx);
        let muted = cx.theme().muted_foreground;
        // The kit's own row: the text (with the row's ↺) on the left, the control on
        // the right, which is the layout every settings panel is read by.
        div()
            .w_full()
            .h_flex()
            .justify_between()
            .items_center()
            .gap_3()
            .child(
                v_flex()
                    .flex_1()
                    .max_w_3_5()
                    .child(
                        h_flex()
                            .gap_1()
                            .child(Label::new(title.clone()).text_sm())
                            .children(undo_button),
                    )
                    .child(div().text_sm().text_color(muted).child(description)),
            )
            .child(div().id("field").child(control))
    })
    .keywords(search)
    .on_reset(move |cx| (page_reset.dirty)(cx), move |window, cx| (page_reset.restore)(window, cx))
}

/// A switch control: the panel bool and the setter a click uses.
fn switch(
    weak: &Weak,
    get: fn(&SettingsPanel) -> bool,
    set: fn(&mut SettingsPanel, bool, &mut Context<SettingsPanel>),
) -> Control {
    let (read, write) = (weak.clone(), weak.clone());
    Rc::new(move |_, _, cx| {
        let on = read.read_with(cx, |this, _| get(this)).unwrap_or_default();
        let write = write.clone();
        Switch::new("check")
            .checked(on)
            .small()
            .on_click(move |on: &bool, _, cx| {
                write.update(cx, |this, cx| set(this, *on, cx)).ok();
            })
            .into_any_element()
    })
}

/// The undo of a switch row, which a reset turns back to `default`.
fn switch_undo(
    weak: &Weak,
    get: fn(&SettingsPanel) -> bool,
    default: bool,
    set: fn(&mut SettingsPanel, bool, &mut Context<SettingsPanel>),
) -> Undo {
    Undo::of_panel(
        weak,
        move |this| get(this) != default,
        move |this, cx| set(this, default, cx),
    )
}

/// The buttons of a segmented row, in order.
fn segments(entries: &[(&'static str, &'static str)]) -> Vec<(String, String)> {
    entries.iter().map(|(id, label)| ((*id).to_owned(), (*label).to_owned())).collect()
}

/// A single-selection segmented control (one kit `ButtonGroup`): `entries` are the
/// buttons in order, `selected` reports which one is active, and `apply` stores a click.
fn segmented(
    group: impl Into<ElementId> + Clone + 'static,
    entries: Vec<(String, String)>,
    weak: &Weak,
    selected: impl Fn(&SettingsPanel) -> usize + 'static,
    apply: impl Fn(&mut SettingsPanel, usize, &mut Context<SettingsPanel>) + 'static,
) -> Control {
    let (read, write) = (weak.clone(), weak.clone());
    let apply: Apply = Rc::new(apply);
    Rc::new(move |_, _, cx| {
        let active = read.read_with(cx, |this, _| selected(this)).unwrap_or_default();
        let mut control = ButtonGroup::new(group.clone()).compact().small();
        for (index, (id, label)) in entries.iter().enumerate() {
            control = control.child(Button::new(id.clone()).label(label.clone()).selected(index == active));
        }
        let write = write.clone();
        let apply = apply.clone();
        control
            .on_click(move |clicks: &Vec<usize>, _, cx| {
                if let Some(index) = clicks.first() {
                    let apply = apply.clone();
                    write.update(cx, move |this, cx| apply(this, *index, cx)).ok();
                }
            })
            .into_any_element()
    })
}

/// A number control: a Small kit `NumberInput` over the panel's `input`. Its width
/// follows the kit's own number field; without one the digits collapse and the row
/// shows nothing at all.
fn number_field(
    weak: &Weak,
    row: fn(&mut SettingsPanel) -> &mut NumberRow,
    input: Entity<InputState>,
    suffix: Option<&'static str>,
) -> Control {
    let write_back = weak.clone();
    Rc::new(move |options, window, cx| {
        // Only a change made outside the field (a reset or a reseed) is written back;
        // pushing the panel's text on every render would overwrite what is being typed.
        let text = write_back.update(cx, |this, _| row(this).take_write_back()).ok().flatten();
        if let Some(text) = text {
            input.update(cx, |input, cx| input.set_value(text, window, cx));
        }
        // `NumberInput` carries no id of its own, so the frame around it names the row's
        // field and nothing else may reuse that name.
        div()
            .id(format!("number-{}-{}-{}", options.page_ix(), options.group_ix(), options.item_ix()))
            .test_support()
            .child(
                NumberInput::new(&input)
                    .small()
                    .when_some(suffix, |this, suffix| {
                        let faded = cx.theme().muted_foreground;
                        this.suffix(div().text_sm().text_color(faded).child(suffix))
                    })
                    .map(|this| {
                        if options.layout().is_horizontal() {
                            this.w_32()
                        } else {
                            this.w_full()
                        }
                    }),
            )
            .into_any_element()
    })
}

/// A select control: a Small outline kit button showing the current label and opening
/// the kit's dropdown menu. This is the control the kit's own dropdown field uses, so
/// our dropdown rows look like the kit's.
fn select(
    entries: Vec<(SharedString, SharedString)>,
    read: Read,
    write: Write,
) -> Control {
    Rc::new(move |options, _, cx| {
        let value = read(cx);
        let label = entries
            .iter()
            .find(|(entry, _)| *entry == value)
            .map_or_else(|| value.clone(), |(_, label)| label.clone());
        let menu_entries = entries.clone();
        let write = write.clone();
        Button::new("btn")
            .when(options.layout().is_vertical(), |this| this.w_full())
            .label(label)
            .dropdown_caret(true)
            .outline()
            .small()
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                menu_entries.iter().fold(menu, |menu, (entry, label)| {
                    let entry = entry.clone();
                    menu.item(
                        PopupMenuItem::new(label.clone())
                            .checked(entry == value)
                            .on_click({
                                let write = write.clone();
                                move |_, _, cx| write(entry.clone(), cx)
                            }),
                    )
                })
            })
            .into_any_element()
    })
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
        // The kit keys its own state by element id, so the page and query it starts with
        // follow `page_gen`.
        let settings = format!("settings-{}", self.page_gen);
        // The window draws the kit `TitleBar` itself (the window options come from
        // `TitleBar::window_options`), laid out like the main window's bar: the kit's own
        // left inset is dropped (`pl_0`) so the title keeps the traffic-light clearance, and
        // the reset action is an icon button on the right.
        let title_fg = if window.is_window_active() { t.foreground } else { t.muted_foreground };
        let left = div()
            .flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .items_center()
            .pl(px(if cfg!(target_os = "macos") && !window.is_fullscreen() { 80. } else { 12. }))
            .pr_3()
            .text_size(px(12.))
            .text_color(title_fg)
            .child(div().min_w_0().overflow_hidden().text_ellipsis().whitespace_nowrap().child("Settings"));
        // The reset action is a real control, so presses on it must not start a drag.
        let reset_all = div().flex_none().on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).child(
            icon_button("settings-reset-all", IconName::Undo2, "Reset all settings…", "", cx)
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
                    }),
            );
        let titlebar = TitleBar::new()
            .pl_0()
            .child(left)
            .child(div().flex().flex_none().items_center().h_full().pr_3().child(reset_all));
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
            .child(titlebar)
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
                                .small()
                                .sidebar_width(px(190.))
                                .default_selected_index(SelectIndex { page_ix: self.page, group_ix: None })
                                .pages(pages),
                        ),
                    ),
            )
    }
}
