//! Root view: rail + message list + reader, hint bar, help overlay, toast,
//! command palette, reply composer, snooze picker, settings/rules panels,
//! search and triage-session modes.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;
use std::sync::Arc;
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
use crate::app::email_webview::EmailWebViews;
use crate::app::folder_picker::{FolderOption, FolderPicker, FolderPickerEvent};
use crate::app::palette::{CommandPalette, PaletteEvent};
use crate::app::panels::{RuleBanner, RulesEvent, RulesPanel, SessionCard, SummaryCard};
use crate::app::settings::{AccountRow, SettingsEvent, SettingsPanel};
use crate::app::snooze::{SnoozeEvent, SnoozePicker};
use crate::clock::{Clock, DAY, SystemClock, Timestamp};
use crate::tz::{MONTHS, WEEKDAYS, Now, SystemZone, Zone};
use crate::judge::{
    Judge, JudgeFactory, JudgePolicy, JudgeStatus, Kind, QuestionKey, Routed, StoredAnswer,
};
use crate::model::{AccountId, Folder, FolderId, Location, Mailbox, Message, MessageId, Tag, Triage, TriageState};
use crate::reading::ReaderView;
use crate::spam_senders::{SpamSenders, SpamVerdict};
use crate::provider::secrets::{MemorySecrets, SecretStore};
use crate::rules::{Rule, RuleBook};
use crate::search::Query;
use crate::summary::{StubSummarizer, Summarizer, ThreadSummary};
use crate::tabs::Tabs;
use crate::app::ui::{button, icon_button, run};
use crate::threads::Row;
use crate::list_tabs::ListTabs;

use self::list_strip::{Pane, Slot};
use self::list_view::ListView;
use self::judging::JudgeJob;

mod accessors;
mod actions;
mod dock;
mod drag;
mod filters;
mod grouping;
mod help;
mod judging;
mod list;
mod menus;
mod modals;
mod list_strip;
mod list_tabbar;
pub mod list_view;
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
    /// The open views of the mailbox, one tab each, and which one the list is showing.
    pub list_tabs: ListTabs,
    /// The state of every open list tab, in `list_tabs`' order (see [`MailApp::view`]): each
    /// view's cursor, selection and scroll, so no tab can be left holding another's.
    list_views: Vec<ListView>,
    /// Which pane the tab keys act on.
    pane_focus: Pane,
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
    /// The sidebar, list and reader, as one dock.
    pub panes: Panes,
    /// Per-thread reader disclosure state (expanded messages, recipients, quoted text, reader mode).
    pub reader: ReaderView,
    /// Messages the next dispatched menu action applies to, instead of the cursor's. Set
    /// by a message menu for exactly one action dispatch (see `menus.rs`).
    menu_target: Option<Vec<MessageId>>,
    pub policy: JudgePolicy,
    pub rules: RuleBook,
    /// The classifier, when one is configured. `None` means no key: classification is off.
    judge: Option<Arc<dyn Judge + Send + Sync>>,
    /// Builds the judge from the Jev key, endpoint and model; swapped for a fake in tests.
    judge_factory: JudgeFactory,
    /// Where the Jev key is kept; `MemorySecrets` by default, the keychain in the real app.
    secrets: Rc<dyn SecretStore>,
    /// The classifier's state, shown in the status bar.
    judge_status: JudgeStatus,
    /// Judge answers cached this session, keyed by [`MessageId`]; preloaded at startup.
    judgements: HashMap<MessageId, Vec<StoredAnswer>>,
    /// Messages and replies waiting to be judged, oldest first.
    judge_queue: VecDeque<JudgeJob>,
    /// Completed requests in the current classifier batch (footer `Classifying X/Y`).
    judge_batch_done: usize,
    /// Total requests in the current classifier batch, grown as work is queued mid-batch.
    judge_batch_total: usize,
    /// A judge request is in flight; the worker is running.
    judging: bool,
    /// Message of the request in flight, so a second pass does not queue it again.
    judge_in_flight: Option<MessageId>,
    /// Bumped when the judge is replaced, so an in-flight result is discarded.
    judge_gen: u64,
    /// Messages whose last classification failed (cleared on success); `ClassifyState::Failed`.
    classify_failed: HashSet<MessageId>,
    /// Automatic classification (`AUTO_CLASSIFY`); false means Jev runs only on demand.
    auto_classify: bool,
    /// Age cutoff for automatic classification in days (`AUTO_CLASSIFY_MAX_AGE_DAYS`); 0 = off.
    auto_max_age_days: u16,
    /// The cold-outreach checkbox (`SPAM_COLD_OUTREACH`); part of the Spam version.
    spam_cold_outreach: bool,
    /// Messages classified before their body downloaded: id → force every enabled question.
    /// The never/always-spam lists as last written to preferences.
    saved_spam_senders: SpamSenders,
    pending_classify: HashMap<MessageId, bool>,
    /// The opened message the frame loop last saw, so the open hook fires once per change.
    last_opened: Option<MessageId>,
    clock: Rc<dyn Clock>,
    /// The zone every displayed date and every relative date is read in: the user's clock.
    zone: Rc<dyn Zone>,
    /// One row per thread instead of per message (State panels).
    pub group_threads: bool,
    /// Threads whose messages are listed under their header.
    expanded: HashSet<u32>,
    snooze: Option<Entity<SnoozePicker>>,
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
    /// The list a session works through: its own cursor, selection and scroll over the queue,
    /// so a session never touches the user's list tabs. Present exactly while `session` or
    /// `session_end` is (see [`MailApp::triage_open`]).
    session_view: Option<ListView>,
    /// Which list and reader the render in flight draws, when it is not the default
    /// ([`MailApp::slot`]). Set only while a panel renders, so the user's own tabs and the
    /// Triage tab can each draw their own state in the same frame.
    drawing: std::cell::Cell<Option<Slot>>,
    summary: Option<(u32, ThreadSummary)>,
    toast_gen: u64,
    focus_handle: FocusHandle,
    help_scroll: ScrollHandle,
    /// Scroll state of each tabbed thread's reader, so a tab switch keeps the position and the
    /// opened message can be scrolled into view.
    reader_panes: RefCell<HashMap<u32, reader::ReaderPane>>,
    /// The native web views that draw HTML message bodies, kept per message while its tab lives.
    webviews: EmailWebViews,
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
    /// Bodies waiting to be downloaded, front first: the opened/visible rows, then the rest of
    /// the Inbox newest first.
    body_queue: VecDeque<MessageId>,
    /// Bodies that failed this session; not retried until reopened or re-classified.
    body_failed: HashSet<MessageId>,
    /// Body prefetch paused after a rate limit; a timer resumes it.
    bodies_paused: bool,
    /// Wakes the sync loop when work is queued.
    sync_wake: Option<futures::channel::mpsc::UnboundedSender<()>>,
    /// Toast text queued from a background task, shown on the next frame.
    pending_toast: Option<String>,
}

impl MailApp {
    pub fn new(mailbox: Mailbox, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new_with_clock(mailbox, Rc::new(SystemClock), Rc::new(SystemZone), window, cx)
    }

    pub fn new_with_clock(
        mailbox: Mailbox,
        clock: Rc<dyn Clock>,
        zone: Rc<dyn Zone>,
        window: &mut Window,
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
        // A window opens on All Inboxes, and that tab can never be closed.
        let all_inboxes = mailbox.location_query(&Location::AllInboxes);
        let panes = Panes::new(&cx.weak_entity(), &all_inboxes, window, cx);
        let mut app = Self {
            mailbox,
            list_tabs: ListTabs::new(all_inboxes.clone()),
            list_views: vec![ListView::new(all_inboxes, Location::AllInboxes)],
            pane_focus: Pane::default(),
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
            reader: ReaderView::default(),
            menu_target: None,
            policy: JudgePolicy::default(),
            rules: RuleBook::default(),
            judge: None,
            judge_factory: default_judge_factory(),
            secrets: Rc::new(MemorySecrets::default()),
            judge_status: JudgeStatus::NoKey,
            judgements: HashMap::new(),
            judge_queue: VecDeque::new(),
            judge_batch_done: 0,
            judge_batch_total: 0,
            judging: false,
            judge_in_flight: None,
            judge_gen: 0,
            classify_failed: HashSet::new(),
            // Overridden from `AUTO_CLASSIFY` by `load_preferences` at startup; the constructor
            // default keeps headless tests that never load preferences classifying.
            auto_classify: true,
            auto_max_age_days: 0,
            spam_cold_outreach: false,
            pending_classify: HashMap::new(),
            saved_spam_senders: SpamSenders::default(),
            last_opened: None,
            zone,
            clock,
            group_threads: false,
            expanded: HashSet::new(),
            _theme_sub: None,
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
            session_view: None,
            drawing: std::cell::Cell::new(None),
            summary: None,
            toast_gen: 0,
            focus_handle: cx.focus_handle(),
            help_scroll: ScrollHandle::new(),
            reader_panes: RefCell::new(HashMap::new()),
            webviews: EmailWebViews::new(cx),
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
            body_queue: VecDeque::new(),
            body_failed: HashSet::new(),
            bodies_paused: false,
            sync_wake: None,
            pending_toast: None,
        };
        app.classify_visible(cx);
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
        self.policy.set_mode(
            QuestionKey::Folder,
            crate::app_settings::MODE_FOLDER.get(store.as_ref(), &scope),
        );
        self.auto_classify = crate::app_settings::AUTO_CLASSIFY.get(store.as_ref(), &scope);
        self.auto_max_age_days =
            crate::app_settings::AUTO_CLASSIFY_MAX_AGE_DAYS.get(store.as_ref(), &scope);
        self.spam_cold_outreach =
            crate::app_settings::SPAM_COLD_OUTREACH.get(store.as_ref(), &scope);
        self.preferences = Some(store);
        let senders = SpamSenders::from_lists(
            crate::app_settings::NEVER_SPAM_SENDERS.get(store.as_ref(), &scope),
            crate::app_settings::ALWAYS_SPAM_SENDERS.get(store.as_ref(), &scope),
        );
        self.mailbox.load_spam_senders(senders.clone());
        self.saved_spam_senders = senders;
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
        // Map any pre-MIGRATION_4 hash-keyed judgements onto their message ids before the first
        // pass, so their answers are found and not re-billed, then load every cached answer.
        self.carry_over_judgements();
        self.preload_judgements();
        self.classify_visible(cx);
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

    /// Use `secrets` for the Jev key instead of the default in-memory store. `main` installs the
    /// keychain here; tests leave the default so nothing touches the real keychain.
    pub fn set_secrets(&mut self, secrets: Rc<dyn SecretStore>) {
        self.secrets = secrets;
    }

    /// Use `factory` to build the judge from the Jev settings, so tests can inject a fake.
    pub fn set_judge_factory(&mut self, factory: JudgeFactory) {
        self.judge_factory = factory;
    }
}

/// The production judge factory: a [`JevJudge`] over the configured endpoint.
fn default_judge_factory() -> JudgeFactory {
    Rc::new(|base, model, key| Arc::new(crate::judge::JevJudge::new(base, model, key)))
}

