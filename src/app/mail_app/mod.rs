//! Root view: rail + message list + reader, hint bar, help overlay, toast,
//! command palette, reply composer, snooze picker, settings/rules panels,
//! search and triage-session modes.

use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

use crate::theme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::app::actions::*;
use crate::app::chrome::{HelpOverlay, HintBar, HintMode};
use crate::app::icons::{self, Glyph, GlyphInputs};
use crate::app::row::{self, RowVisual};
use crate::app::overlay::overlay;
use crate::app::compose::{ComposeEvent, ComposeReply};
use crate::app::dialog::{ChoiceDialog, DialogEvent, DialogOption};
use crate::app::folder_picker::{FolderOption, FolderPicker, FolderPickerEvent};
use crate::app::palette::{CommandPalette, PaletteEvent};
use crate::app::menu::{MENU_W, MenuPanel};
use crate::app::panels::{RuleBanner, RulesEvent, RulesPanel, SessionCard, SummaryCard};
use crate::app::settings::{SettingsEvent, SettingsPanel};
use crate::app::snooze::{SnoozeEvent, SnoozePicker};
use crate::clock::{Clock, DAY, SystemClock, Timestamp};
use crate::judge::{JudgePolicy, Kind, Routed, StubJudge, classify};
use crate::model::{AccountId, Chip, Filter, Folder, FolderId, Location, Mailbox, Message, MessageId, Tag, TagFilter, Triage, TriageState, View};
use crate::rules::{Rule, RuleBook};
use crate::search::Query;
use crate::summary::{StubSummarizer, Summarizer, ThreadSummary};
use crate::app::ui::{button, run};
use crate::threads::Row;

mod accessors;
mod actions;
mod chips;
mod filter_menu;
mod grouping;
mod help;
mod list;
mod menus;
mod modals;
mod mouse;
pub mod panes;
mod reader;
mod render;
mod sidebar;
mod titlebar;
mod rows;
use menus::{MenuKind, OpenMenu};
use panes::{Orientation as PaneLayout, Panes};
use mouse::close_on_backdrop;
use reader::format_when;

/// Height of the app-owned titlebar (also anchors the global menu below it).
const HEADER_H: f32 = 36.;
/// Height of the contextual action bar.
const BAR_H: f32 = 30.;
/// Height of the list header above the rows (title, selection count, Filter ▾).
const LIST_HEADER_H: f32 = 28.;
const TOAST_MS: u64 = 4000;

/// What the message list currently shows.
#[derive(Clone, Debug, PartialEq, Eq)]
enum ListMode {
    /// One of the four triage states (`Triage::view`).
    State,
    /// Search results for the (already stripped) query text.
    Search(String),
}

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
    /// Message shown in the reader pane.
    pub opened: Option<MessageId>,
    pub palette: Option<Entity<CommandPalette>>,
    pub compose: Option<Entity<ComposeReply>>,
    pub help: bool,
    /// Toast text, if visible.
    pub toast: Option<SharedString>,
    /// Opt-in thread summaries (settings panel).
    pub summaries_enabled: bool,
    /// Preview snippet lines under each subject (`crate::preview::OPTIONS`); 0 is Off.
    pub preview_lines: u8,
    /// Messages that have been shown in the reader (everything else is unread).
    read: HashSet<MessageId>,
    /// Width of the message list panel in pixels (drives how many row icons fit):
    /// the pane size side by side, the whole pane region when stacked.
    list_w: f32,
    /// Height of the message list panel in pixels when the panes are stacked.
    list_h: f32,
    /// How much room the list and the reader share, and how they are stacked.
    pub panes: Panes,
    pub policy: JudgePolicy,
    pub rules: RuleBook,
    clock: Rc<dyn Clock>,
    mode: ListMode,
    /// Cursor for the non-`State` list modes.
    alt_cursor: usize,
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
    help_scroll: ScrollHandle,
    _modal_sub: Option<Subscription>,
    /// The open popup menu, if any.
    menu: Option<OpenMenu>,
    _menu_sub: Option<Subscription>,
    /// Folders whose children are folded away in the sidebar.
    collapsed_folders: HashSet<FolderId>,
    /// Accounts whose sections are folded away in the sidebar.
    collapsed_accounts: HashSet<AccountId>,
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
        let mut app = Self {
            mailbox,
            triage: Triage::new(View::default()),
            opened: None,
            palette: None,
            compose: None,
            help: false,
            toast: None,
            summaries_enabled: false,
            preview_lines: crate::preview::DEFAULT_LINES,
            read: HashSet::new(),
            list_w: 0.,
            list_h: 0.,
            panes: Panes::default(),
            policy: JudgePolicy::default(),
            rules: RuleBook::default(),
            clock,
            mode: ListMode::State,
            alt_cursor: 0,
            group_threads: false,
            expanded: HashSet::new(),
            row_cursor: 0,
            row_anchor: None,
            snooze: None,
            dialog: None,
            folder_picker: None,
            settings: None,
            rules_panel: None,
            pending_rule: None,
            session: None,
            session_end: None,
            summary: None,
            toast_gen: 0,
            focus_handle: cx.focus_handle(),
            list_state: ListState::new(0, ListAlignment::Top, px(200.)),
            list_shape: None,
            help_scroll: ScrollHandle::new(),
            _modal_sub: None,
            menu: None,
            _menu_sub: None,
            collapsed_folders: HashSet::new(),
            collapsed_accounts: HashSet::new(),
        };
        app.classify_visible();
        app
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

