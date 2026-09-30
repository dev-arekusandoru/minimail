//! Root view: rail + message list + reader, hint bar, help overlay, toast,
//! command palette, reply composer, snooze picker, settings/rules panels,
//! screener, search and triage-session modes.

use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

use crate::theme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::app::actions::*;
use crate::app::chrome::{EmptyState, HelpOverlay, HintBar, HintMode, ViewTabs};
use crate::app::icons::{self, Glyph, GlyphInputs};
use crate::app::row::{self, RowVisual};
use crate::app::overlay::overlay;
use crate::app::compose::{ComposeEvent, ComposeReply};
use crate::app::palette::{CommandPalette, PaletteEvent};
use crate::app::panels::{
    RuleBanner, RulesEvent, RulesPanel, ScreenerHeader, SessionCard, SummaryCard,
};
use crate::app::settings::{SettingsEvent, SettingsPanel};
use crate::app::snooze::{SnoozeEvent, SnoozePicker};
use crate::clock::{Clock, DAY, SystemClock, Timestamp};
use crate::judge::{JudgePolicy, Routed, StubJudge, classify};
use crate::model::{Mailbox, Message, MessageId, Triage, TriageState};
use crate::rules::{Rule, RuleBook};
use crate::search::Query;
use crate::summary::{StubSummarizer, Summarizer, ThreadSummary};
use crate::app::ui::{button, run};
use crate::threads::Row;

mod accessors;
mod actions;
mod grouping;
mod help;
mod list;
mod modals;
mod mouse;
mod reader;
mod render;
mod rows;
use mouse::close_on_backdrop;
use reader::format_when;

const TOAST_MS: u64 = 4000;

/// What the message list currently shows.
#[derive(Clone, Debug, PartialEq, Eq)]
enum ListMode {
    /// One of the four triage states (`Triage::view`).
    State,
    Screener,
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
    /// Width of the message list panel in pixels (drives how many row icons fit).
    list_w: f32,
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
    settings: Option<Entity<SettingsPanel>>,
    rules_panel: Option<Entity<RulesPanel>>,
    pending_rule: Option<Rule>,
    session: Option<Session>,
    session_end: Option<(usize, i64)>,
    summary: Option<(u32, ThreadSummary)>,
    toast_gen: u64,
    focus_handle: FocusHandle,
    list_scroll: UniformListScrollHandle,
    help_scroll: ScrollHandle,
    _modal_sub: Option<Subscription>,
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
        // Real-time heartbeat: wakes snoozed mail, resurfaces Waiting, flushes the outbox.
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
            triage: Triage::new(TriageState::Inbox),
            opened: None,
            palette: None,
            compose: None,
            help: false,
            toast: None,
            summaries_enabled: false,
            preview_lines: crate::preview::DEFAULT_LINES,
            read: HashSet::new(),
            list_w: row::list_width(1200.),
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
            settings: None,
            rules_panel: None,
            pending_rule: None,
            session: None,
            session_end: None,
            summary: None,
            toast_gen: 0,
            focus_handle: cx.focus_handle(),
            list_scroll: UniformListScrollHandle::new(),
            help_scroll: ScrollHandle::new(),
            _modal_sub: None,
        };
        app.classify_visible();
        app
    }

    /// Advance time-based mailbox behavior (snooze wake-up, Waiting resurfacing, outbox flush).
    pub fn tick(&mut self, cx: &mut Context<Self>) {
        let now = self.now();
        self.mailbox.tick(now);
        cx.notify();
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus_handle
    }
}

