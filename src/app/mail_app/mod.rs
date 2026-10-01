//! Root view: rail + message list + reader, hint bar, help overlay, toast,
//! command palette, reply composer, snooze picker, settings/rules panels,
//! search and triage-session modes.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use crate::theme;
use gpui_kit::component::{WindowExt as _, notification::Notification};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::app::actions::*;
use crate::app::chrome::{HelpEvent, HelpPanel, HintBar};
use crate::hints::{HintContext, HintMode};
use crate::app::icons::{self, Glyph, GlyphInputs};
use crate::app::row::{self, RowVisual};
use crate::app::compose::{ComposeEvent, ComposeReply};
use crate::draft::DraftKind;
use crate::app::dialog::{ChoiceDialog, DialogEvent, DialogOption};
use crate::app::folder_picker::{FolderOption, FolderPicker, FolderPickerEvent};
use crate::app::palette::{CommandPalette, PaletteEvent};
use crate::app::panels::{RuleBanner, RulesEvent, RulesPanel, SessionCard, SummaryCard};
use crate::app::settings::{AccountRow, SettingsEvent, SettingsPanel};
use crate::app::snooze::{SnoozeEvent, SnoozePicker};
use crate::clock::{Clock, DAY, SystemClock, Timestamp};
use crate::judge::{JudgePolicy, Kind, QuestionKey, Routed, StubJudge, classify};
use crate::model::{AccountId, Folder, FolderId, Location, Mailbox, Message, MessageId, Tag, Triage, TriageState};
use crate::reading::ReaderView;
use crate::rules::{Rule, RuleBook};
use crate::search::Query;
use crate::summary::{StubSummarizer, Summarizer, ThreadSummary};
use crate::tabs::Tabs;
use crate::app::ui::{button, icon_button, run};
use crate::threads::Row;

mod accessors;
mod actions;
mod filters;
mod grouping;
mod help;
mod list;
mod menus;
mod modals;
mod mouse;
pub mod panes;
mod reader;
mod render;
mod reader_find;
mod reader_state;
mod reader_tabs;
mod sidebar;
mod titlebar;
mod sync;
mod rows;
use menus::MenuKind;
use panes::{Orientation as PaneLayout, Panes};
use reader::format_when;

/// Rows after the end of the list that start a load-more request.
const LOAD_MORE_MARGIN: usize = 20;
/// Identity of the toast notification, so each new toast replaces the last.
struct ToastId;
/// Identity of the persistent “Fetching mail…” notification, kept apart from `ToastId` so the
/// result toast doesn't replace it before it is dismissed.
struct FetchToastId;

/// A triage session: one inbox message at a time.
struct Session {
    ids: Vec<MessageId>,
    index: usize,
    handled: usize,
    started: Timestamp,
}

pub struct MailApp {
    pub mailbox: Mailbox,
    pub triage: Triage,
    /// The folder the sidebar last showed. Its `in:` (and `account:`) values are the query's
    /// implicit part: hidden as pills until something else is applied.
    folder: Location,
    /// Reader tabs, one per thread. The active tab's message is the one shown in the reader
    /// (see [`MailApp::opened`]).
    pub tabs: Tabs,
    /// Show the sender's monogram as each tab's icon (settings).
    pub tab_avatars: bool,
    theme_mode: crate::theme::ThemeMode,
    light_theme: String,
    dark_theme: String,
    pub palette: Option<Entity<CommandPalette>>,
    pub compose: Option<Entity<ComposeReply>>,
    help: Option<Entity<HelpPanel>>,
    /// Toast text, if visible.
    pub toast: Option<SharedString>,
    /// Opt-in thread summaries (settings panel).
    pub summaries_enabled: bool,
    /// Block remote images in the reader (settings panel; opt in, images load by default).
    pub block_remote_images: bool,
    /// Preview snippet lines under each subject (`crate::preview::OPTIONS`); 0 is Off.
    pub preview_lines: u8,
    /// Width of the message list panel in pixels (drives how many row icons fit):
    /// the pane size side by side, the whole pane region when stacked.
    list_w: f32,
    /// The sidebar, list and reader layout and its resizable group states.
    pub panes: Panes,
    _pane_subs: Vec<Subscription>,
    /// Per-thread reader disclosure state (expanded messages, recipients, quoted text, reader mode).
    pub reader: ReaderView,
    /// Messages the next dispatched menu action applies to, instead of the cursor's. Set
    /// by a message menu for exactly one action dispatch (see `menus.rs`).
    menu_target: Option<Vec<MessageId>>,
    pub policy: JudgePolicy,
    pub rules: RuleBook,
    clock: Rc<dyn Clock>,
    /// One row per thread instead of per message (State panels).
    pub group_threads: bool,
    /// Threads whose messages are listed under their header.
    expanded: HashSet<u32>,
    /// Cursor over grouped rows.
    row_cursor: usize,
    row_anchor: Option<usize>,
    snooze: Option<Entity<SnoozePicker>>,
    /// Generic choice dialog (post-send, block, spam, sender-wide confirms).
    dialog: Option<Entity<ChoiceDialog>>,
    /// Folder picker (`f` and the dialogs' `File…`).
    folder_picker: Option<Entity<FolderPicker>>,
    settings: Option<Entity<SettingsPanel>>,
    _theme_sub: Option<Subscription>,
    /// Separate native Settings window, if currently open.
    settings_window: Option<AnyWindowHandle>,
    preferences: Option<Rc<crate::contacts::ContactStore>>,
    rules_panel: Option<Entity<RulesPanel>>,
    pending_rule: Option<Rule>,
    session: Option<Session>,
    session_end: Option<(usize, i64)>,
    summary: Option<(u32, ThreadSummary)>,
    toast_gen: u64,
    focus_handle: FocusHandle,
    /// Variable-height message list: rows are measured, so thread headers can size to
    /// their content.
    list_state: ListState,
    /// What `list_state` was last measured against, so a stale row height is re-measured.
    list_shape: Option<list::ListShape>,
    /// End of the list's visible range, from its scroll handler.
    list_visible_end: std::cell::Cell<usize>,
    help_scroll: ScrollHandle,
    /// Scroll state of each tabbed thread's reader, so a tab switch keeps the position and the
    /// opened message can be scrolled into view.
    reader_panes: RefCell<HashMap<u32, reader::ReaderPane>>,
    /// Open find bars, by the thread of their tab.
    finds: HashMap<u32, reader_find::FindTab>,
    /// Layout of the text holding the current find match, from the frame numbered by the last
    /// field, and that frame counter (see `place_find_match`).
    find_layout: RefCell<Option<reader_find::FindLayout>>,
    find_gen: std::cell::Cell<u64>,
    /// Frames until a reopened find bar selects its text (see `place_find_match`).
    find_select: std::cell::Cell<u8>,
    _modal_sub: Option<Subscription>,
    _settings_sub: Option<Subscription>,
    /// Which popup menu is open, if any (the kit owns the popup itself).
    open_menu: Option<MenuKind>,
    /// The `+ Filter` picker or a pill's value editor, when one is open.
    filter_popover: Option<filters::FilterPopoverState>,
    /// Local mail cache; `None` when it could not be opened.
    cache: Option<Rc<crate::sync::cache::Cache>>,
    providers: HashMap<AccountId, crate::sync::SharedProvider>,
    /// Last sync error shown, so each distinct error toasts once.
    sync_error: Option<String>,
    /// Latest round error per account, cleared by its next error-free round (Accounts settings).
    account_errors: HashMap<AccountId, String>,
    /// Accounts the user asked to retry (Accounts settings); each runs a check on the next round.
    retry_accounts: HashSet<AccountId>,
    /// Run a server change check on the next round (Fetch mail, sign-in).
    force_check: bool,
    /// Time of the next scheduled check.
    check_at: Timestamp,
    /// Rounds pause until this time after a rate limit.
    throttled_until: Option<Timestamp>,
    /// Message count when a fetch was requested; toast the difference when it lands.
    fetch_baseline: Option<usize>,
    /// The persistent “Fetching mail…” notification is showing.
    fetch_toast: bool,
    /// Scopes whose older mail the next round should extend.
    older_queue: HashMap<AccountId, Vec<crate::provider::Scope>>,
    /// Accounts with a load-more page in flight.
    older_in_flight: HashSet<AccountId>,
    /// Messages whose body is being downloaded.
    bodies_in_flight: HashSet<MessageId>,
    /// Wakes the sync loop when work is queued.
    sync_wake: Option<futures::channel::mpsc::UnboundedSender<()>>,
    /// Toast text queued from a background task, shown on the next frame.
    pending_toast: Option<String>,
}

impl MailApp {
    pub fn new(mailbox: Mailbox, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new_with_clock(mailbox, Rc::new(SystemClock), window, cx)
    }

    pub fn new_with_clock(
        mailbox: Mailbox,
        clock: Rc<dyn Clock>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // Real-time heartbeat wakes snoozed mail and flushes the outbox.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                if this.update(cx, |this, cx| this.tick(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        let panes = Panes::new(cx);
        // Rows size themselves from the list pane, so a drag repaints the app as it moves.
        let pane_subs = panes.states().into_iter().map(|state| cx.observe(state, |_, _, cx| cx.notify())).collect();
        let mut app = Self {
            mailbox,
            triage: Triage::new(Query::parse("in:inbox")),
            folder: Location::AllInboxes,
            tabs: Tabs::default(),
            tab_avatars: true,
            theme_mode: crate::theme::ThemeMode::System,
            light_theme: crate::app_settings::DEFAULT_LIGHT_THEME.to_owned(),
            dark_theme: crate::app_settings::DEFAULT_DARK_THEME.to_owned(),
            palette: None,
            compose: None,
            help: None,
            toast: None,
            summaries_enabled: false,
            block_remote_images: false,
            preview_lines: crate::preview::DEFAULT_LINES,
            list_w: 0.,
            panes,
            _pane_subs: pane_subs,
            reader: ReaderView::default(),
            menu_target: None,
            policy: JudgePolicy::default(),
            rules: RuleBook::default(),
            clock,
            group_threads: false,
            expanded: HashSet::new(),
            row_cursor: 0,
            _theme_sub: None,
            row_anchor: None,
            snooze: None,
            dialog: None,
            folder_picker: None,
            settings: None,
            settings_window: None,
            preferences: None,
            rules_panel: None,
            pending_rule: None,
            session: None,
            session_end: None,
            summary: None,
            toast_gen: 0,
            focus_handle: cx.focus_handle(),
            list_state: ListState::new(0, ListAlignment::Top, px(200.)),
            list_shape: None,
            list_visible_end: std::cell::Cell::new(0),
            help_scroll: ScrollHandle::new(),
            reader_panes: RefCell::new(HashMap::new()),
            finds: HashMap::new(),
            find_layout: RefCell::new(None),
            find_gen: std::cell::Cell::new(0),
            find_select: std::cell::Cell::new(0),
            _modal_sub: None,
            _settings_sub: None,
            open_menu: None,
            filter_popover: None,
            cache: None,
            providers: HashMap::new(),
            sync_error: None,
            account_errors: HashMap::new(),
            retry_accounts: HashSet::new(),
            force_check: true,
            check_at: 0,
            throttled_until: None,
            fetch_baseline: None,
            fetch_toast: false,
            older_queue: HashMap::new(),
            older_in_flight: HashSet::new(),
            bodies_in_flight: HashSet::new(),
            sync_wake: None,
            pending_toast: None,
        };
        app.classify_visible();
        app
    }
    pub fn load_preferences(
        &mut self,
        store: Rc<crate::contacts::ContactStore>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::prefs::Scope;
        let scope = Scope::Global;
        self.tab_avatars = crate::app_settings::TAB_AVATARS.get(store.as_ref(), &scope);
        self.block_remote_images = crate::app_settings::BLOCK_REMOTE_IMAGES.get(store.as_ref(), &scope);
        self.group_threads = crate::app_settings::GROUP_THREADS.get(store.as_ref(), &scope);
        self.preview_lines = crate::app_settings::PREVIEW_LINES.get(store.as_ref(), &scope).min(5);
        self.summaries_enabled = crate::app_settings::SUMMARIES.get(store.as_ref(), &scope);
        let days = crate::app_settings::FOLLOW_UP_DAYS.get(store.as_ref(), &scope).clamp(1, 14);
        self.mailbox.set_follow_up_timeout(i64::from(days) * DAY);
        self.theme_mode = crate::app_settings::THEME_MODE.get(store.as_ref(), &scope);
        self.light_theme = crate::app_settings::LIGHT_THEME.get(store.as_ref(), &scope);
        if self.light_theme.is_empty() {
            self.light_theme = crate::theme::names_for(cx, true)
                .first()
                .map(ToString::to_string)
                .unwrap_or_default();
        }
        self.dark_theme = crate::app_settings::DARK_THEME.get(store.as_ref(), &scope);
        if self.dark_theme.is_empty() {
            self.dark_theme = crate::app_settings::DEFAULT_DARK_THEME.to_owned();
        }
        let layout = crate::app_settings::PANE_LAYOUT.get(store.as_ref(), &scope);
        self.panes.set_orientation(if layout == "stacked" { PaneLayout::Stacked } else { PaneLayout::SideBySide });
        self.policy.set_mode(QuestionKey::Spam, crate::app_settings::MODE_SPAM.get(store.as_ref(), &scope));
        self.policy.set_mode(
            QuestionKey::NeedsReply,
            crate::app_settings::MODE_NEEDS_REPLY.get(store.as_ref(), &scope),
        );
        self.policy.set_mode(QuestionKey::Urgency, crate::app_settings::MODE_URGENCY.get(store.as_ref(), &scope));
        self.policy.set_mode(QuestionKey::Kind, crate::app_settings::MODE_KIND.get(store.as_ref(), &scope));
        self.policy.set_mode(
            QuestionKey::ExpectsReply,
            crate::app_settings::MODE_EXPECTS_REPLY.get(store.as_ref(), &scope),
        );
        self.preferences = Some(store);
        cx.set_window_appearance(match self.theme_mode {
            crate::theme::ThemeMode::Light => Some(WindowAppearance::Light),
            crate::theme::ThemeMode::Dark => Some(WindowAppearance::Dark),
            crate::theme::ThemeMode::System => None,
        });
        self.refresh_theme(cx);
        let weak = cx.weak_entity();
        self._theme_sub = Some(window.observe_window_appearance(move |_, cx| {
            weak.update(cx, |this, cx| this.refresh_theme(cx)).ok();
        }));
        self.classify_visible();
        cx.notify();
    }
    fn refresh_theme(&self, cx: &mut App) {
        let dark = matches!(cx.window_appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark);
        let name = crate::theme::active_theme_name(self.theme_mode, &self.light_theme, &self.dark_theme, dark);
        crate::theme::apply(cx, name);
    }

    /// Advance time-based mailbox behavior, including snooze wake-up and outbox flush.
    pub fn tick(&mut self, cx: &mut Context<Self>) {
        let now = self.now();
        self.mailbox.tick(now);
        cx.notify();
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus_handle
    }
}

