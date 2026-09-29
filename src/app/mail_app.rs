//! Root view: rail + message list + reader, hint bar, help overlay, toast,
//! command palette, reply composer, snooze picker, settings/rules panels,
//! screener, search and triage-session modes.

use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::tag::Tag as UiTag;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::app::actions::*;
use crate::app::chrome::{EmptyState, HelpOverlay, HintBar, HintMode, ViewTabs, badges};
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

const ROW_H: f32 = 26.0;
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
    pub policy: JudgePolicy,
    pub rules: RuleBook,
    clock: Rc<dyn Clock>,
    mode: ListMode,
    /// Cursor for the non-`State` list modes.
    alt_cursor: usize,
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
            policy: JudgePolicy::default(),
            rules: RuleBook::default(),
            clock,
            mode: ListMode::State,
            alt_cursor: 0,
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

    // ---- read-only accessors ----

    pub fn palette_open(&self) -> bool {
        self.palette.is_some()
    }

    pub fn compose_open(&self) -> bool {
        self.compose.is_some()
    }

    pub fn help_open(&self) -> bool {
        self.help
    }

    pub fn snooze_open(&self) -> bool {
        self.snooze.is_some()
    }

    pub fn settings_open(&self) -> bool {
        self.settings.is_some()
    }

    pub fn rules_open(&self) -> bool {
        self.rules_panel.is_some()
    }

    pub fn screener_open(&self) -> bool {
        self.mode == ListMode::Screener
    }

    pub fn opened(&self) -> Option<MessageId> {
        self.opened
    }

    /// `Some("search: …")` while a search is active.
    pub fn search_header(&self) -> Option<String> {
        match &self.mode {
            ListMode::Search(q) => Some(format!("search: {q}")),
            _ => None,
        }
    }

    /// Rule suggestion currently shown in the banner.
    pub fn pending_rule(&self) -> Option<Rule> {
        self.pending_rule.clone()
    }

    /// `(current 1-based, total)` while a session is active.
    pub fn session_progress(&self) -> Option<(usize, usize)> {
        self.session.as_ref().map(|s| (s.index + 1, s.ids.len()))
    }

    /// `(handled, elapsed secs)` once the session end card is shown.
    pub fn session_end(&self) -> Option<(usize, i64)> {
        self.session_end
    }

    /// Summary displayed for the opened thread, if any.
    pub fn summary_shown(&self) -> Option<ThreadSummary> {
        let thread = self.opened.and_then(|id| self.mailbox.get(id))?.thread_id;
        match &self.summary {
            Some((t, s)) if *t == thread => Some(s.clone()),
            _ => None,
        }
    }

    /// Command names currently listed in the open palette.
    pub fn palette_rows(&self, cx: &App) -> Vec<String> {
        self.palette
            .as_ref()
            .map(|p| p.read(cx).rows())
            .unwrap_or_default()
    }

    /// Command names listed in the help overlay.
    pub fn help_lines(&self) -> Vec<String> {
        commands().iter().map(|c| c.name.to_string()).collect()
    }

    /// Exactly the message ids the list shows, top to bottom.
    pub fn visible_ids(&self) -> Vec<MessageId> {
        match &self.mode {
            ListMode::State => self.mailbox.ids_in(self.triage.view),
            ListMode::Screener => self.mailbox.screener_ids(),
            ListMode::Search(q) => self.search_ids(q),
        }
    }

    // ---- internals ----

    fn now(&self) -> Timestamp {
        self.clock.now()
    }

    fn search_ids(&self, q: &str) -> Vec<MessageId> {
        let query = Query::parse(q);
        let mut found: Vec<&Message> = self
            .mailbox
            .messages()
            .iter()
            .filter(|m| !self.mailbox.is_hidden(m.id))
            .filter(|m| {
                let state = self.mailbox.state_of(m.id).unwrap_or_default();
                query.matches(m, state, !self.mailbox.is_screened(m.id))
            })
            .collect();
        found.sort_by(|a, b| b.received.cmp(&a.received).then(b.id.cmp(&a.id)));
        found.into_iter().map(|m| m.id).collect()
    }

    fn in_session(&self) -> bool {
        self.session.is_some()
    }

    fn session_current(&self) -> Option<MessageId> {
        self.session.as_ref().and_then(|s| s.ids.get(s.index).copied())
    }

    /// Message under the cursor (or the session's current message).
    fn cursor_id(&self) -> Option<MessageId> {
        if let Some(id) = self.session_current() {
            return Some(id);
        }
        match self.mode {
            ListMode::State => self.triage.cursor(&self.mailbox),
            _ => {
                let ids = self.visible_ids();
                ids.get(self.alt_cursor.min(ids.len().saturating_sub(1))).copied()
            }
        }
    }

    fn cursor_ix(&self) -> usize {
        match self.mode {
            ListMode::State => self.triage.cursor_index(),
            _ => self.alt_cursor,
        }
    }

    /// Ids an action applies to: session message, selection/cursor, or cursor.
    fn target_ids(&self) -> Vec<MessageId> {
        if self.in_session() || self.mode != ListMode::State {
            return self.cursor_id().into_iter().collect();
        }
        self.triage.targets(&self.mailbox)
    }

    fn move_cursor(&mut self, delta: isize) {
        if self.mode == ListMode::State {
            self.triage.move_cursor(&self.mailbox, delta);
        } else {
            let len = self.visible_ids().len();
            let max = len.saturating_sub(1) as isize;
            self.alt_cursor = (self.alt_cursor as isize + delta).clamp(0, max) as usize;
        }
    }

    fn modal_open(&self) -> bool {
        self.palette.is_some()
            || self.compose.is_some()
            || self.snooze.is_some()
            || self.settings.is_some()
            || self.rules_panel.is_some()
    }

    fn scroll_to_cursor(&self) {
        self.list_scroll
            .scroll_to_item(self.cursor_ix(), ScrollStrategy::Nearest);
    }

    fn show_toast(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.toast_gen += 1;
        let generation = self.toast_gen;
        self.toast = Some(text.into());
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(TOAST_MS))
                .await;
            this.update(cx, |this, cx| {
                if this.toast_gen == generation {
                    this.toast = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn close_modals(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette = None;
        self.compose = None;
        self.snooze = None;
        self.settings = None;
        self.rules_panel = None;
        self._modal_sub = None;
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    /// After a state change in a session: count it and move to the next message.
    fn session_advance(&mut self) {
        let Some(s) = self.session.as_mut() else { return };
        s.handled += 1;
        s.index += 1;
        if s.index >= s.ids.len() {
            let elapsed = self.clock.now() - s.started;
            let handled = s.handled;
            self.session = None;
            self.session_end = Some((handled, elapsed));
            self.opened = None;
        } else {
            self.opened = self.session_current();
        }
    }

    fn mark(&mut self, state: TriageState, window: &mut Window, cx: &mut Context<Self>) {
        let ids = self.target_ids();
        if ids.is_empty() {
            return;
        }
        let n = self.mailbox.set_state_at(&ids, state, self.now());
        self.triage.clear_selection();
        if n > 0 {
            let msg = match state {
                TriageState::Inbox => format!("Moved {n} to inbox · u to undo"),
                s => format!("Marked {n} {} · u to undo", s.label().to_lowercase()),
            };
            self.show_toast(msg, window, cx);
        }
        self.session_advance();
        self.scroll_to_cursor();
        cx.notify();
    }

    fn mark_sender(&mut self, state: TriageState, window: &mut Window, cx: &mut Context<Self>) {
        let Some((name, email)) = self
            .cursor_id()
            .and_then(|id| self.mailbox.get(id))
            .map(|m| (m.from_name.clone(), m.from_email.clone()))
        else {
            return;
        };
        let ids: Vec<MessageId> = self
            .mailbox
            .messages()
            .iter()
            .filter(|m| m.from_email == email)
            .map(|m| m.id)
            .collect();
        let n = self.mailbox.set_state_at(&ids, state, self.now());
        self.triage.clear_selection();
        if n > 0 {
            let msg = format!(
                "Marked {n} from {name} {} · u to undo",
                state.label().to_lowercase()
            );
            self.show_toast(msg, window, cx);
        }
        if let Some(rule) = self.rules.record(&email, state) {
            self.pending_rule = Some(rule);
        }
        self.session_advance();
        self.scroll_to_cursor();
        cx.notify();
    }

    fn accept_rule(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rule) = self.pending_rule.take() else { return };
        self.rules.accept(rule.clone());
        let ids: Vec<MessageId> = self
            .mailbox
            .ids_in(TriageState::Inbox)
            .into_iter()
            .filter(|id| {
                self.mailbox
                    .get(*id)
                    .is_some_and(|m| m.from_email == rule.sender)
            })
            .collect();
        let n = self.mailbox.set_state_at(&ids, rule.state, self.now());
        self.show_toast(
            format!(
                "Rule saved: {} → {} ({n} moved)",
                rule.sender,
                rule.state.label().to_lowercase()
            ),
            window,
            cx,
        );
        self.scroll_to_cursor();
        cx.notify();
    }

    fn dismiss_rule(&mut self, cx: &mut Context<Self>) {
        if let Some(rule) = self.pending_rule.take() {
            self.rules.dismiss(rule);
            cx.notify();
        }
    }

    fn show_view(&mut self, view: TriageState, cx: &mut Context<Self>) {
        self.end_session();
        self.mode = ListMode::State;
        self.triage.switch_view(view);
        self.opened = None;
        self.scroll_to_cursor();
        cx.notify();
    }

    fn show_screener(&mut self, cx: &mut Context<Self>) {
        self.end_session();
        self.mode = ListMode::Screener;
        self.alt_cursor = 0;
        self.opened = None;
        cx.notify();
    }

    fn end_session(&mut self) {
        self.session = None;
        self.session_end = None;
    }

    fn start_session(&mut self, cx: &mut Context<Self>) {
        let ids = self.mailbox.ids_in(TriageState::Inbox);
        if ids.is_empty() {
            return;
        }
        self.mode = ListMode::State;
        self.triage.switch_view(TriageState::Inbox);
        self.opened = ids.first().copied();
        self.session_end = None;
        self.session = Some(Session {
            ids,
            index: 0,
            handled: 0,
            started: self.now(),
        });
        cx.notify();
    }

    /// Run the stub judge over the visible inbox. Returns `(auto-applied, queued for review)`.
    fn classify_visible(&mut self) -> (usize, usize) {
        let ids: Vec<MessageId> = self
            .mailbox
            .ids_in(TriageState::Inbox)
            .into_iter()
            .filter(|id| self.mailbox.pending(*id).is_empty())
            .collect();
        let routed = {
            let msgs: Vec<&Message> = ids.iter().filter_map(|id| self.mailbox.get(*id)).collect();
            classify(&StubJudge, &self.policy, &msgs)
        };
        let now = self.now();
        let (mut auto, mut review) = (0, Vec::new());
        for r in routed {
            match r {
                Routed::Auto(s) => {
                    self.mailbox.apply_auto(s, now);
                    auto += 1;
                }
                Routed::Review(s) => review.push(s),
                Routed::Drop => {}
            }
        }
        let queued = review.len();
        self.mailbox.add_suggestions(review);
        (auto, queued)
    }

    fn summarize(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.summaries_enabled {
            self.show_toast("Enable summaries in settings (cmd-,)".into(), window, cx);
            return;
        }
        let Some(msg) = self.cursor_id().and_then(|id| self.mailbox.get(id)).cloned() else {
            return;
        };
        let mut thread: Vec<&Message> = self
            .mailbox
            .messages()
            .iter()
            .filter(|m| m.thread_id == msg.thread_id)
            .collect();
        thread.sort_by(|a, b| a.received.cmp(&b.received));
        match StubSummarizer.summarize(&thread) {
            Ok(s) => {
                self.summary = Some((msg.thread_id, s));
                self.opened = Some(msg.id);
            }
            Err(e) => self.show_toast(format!("Summary failed: {e:?}"), window, cx),
        }
        cx.notify();
    }

    fn undo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(reply) = self.mailbox.recall_last(self.now()) {
            if let Some(msg) = self.mailbox.get(reply.in_reply_to).cloned() {
                self.close_modals(window, cx);
                self.open_compose_for(&msg, Some(reply.body), window, cx);
            }
            self.show_toast("Send recalled · editing draft".into(), window, cx);
        } else if self.mailbox.undo() {
            self.show_toast("Undone".into(), window, cx);
        }
        self.scroll_to_cursor();
        cx.notify();
    }

    fn open_palette(&mut self, prefill: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        let palette = cx.new(|cx| CommandPalette::new(window, cx));
        self._modal_sub = Some(cx.subscribe_in(
            &palette,
            window,
            |this, _, event: &PaletteEvent, window, cx| {
                this.close_modals(window, cx);
                match event {
                    PaletteEvent::Run(action) => window.dispatch_action(action.boxed_clone(), cx),
                    PaletteEvent::Search(q) => this.start_search(q, cx),
                    PaletteEvent::Dismiss => {}
                }
            },
        ));
        if let Some(q) = prefill {
            palette.update(cx, |p, cx| p.set_query(q, window, cx));
        }
        window.focus(&palette.focus_handle(cx), cx);
        self.palette = Some(palette);
        cx.notify();
    }

    fn start_search(&mut self, raw: &str, cx: &mut Context<Self>) {
        let q = raw.trim().trim_start_matches('/').trim().to_string();
        self.end_session();
        self.mode = ListMode::Search(q);
        self.alt_cursor = 0;
        self.opened = None;
        cx.notify();
    }

    fn open_compose(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(msg) = self.cursor_id().and_then(|id| self.mailbox.get(id)).cloned() else {
            return;
        };
        self.open_compose_for(&msg, None, window, cx);
    }

    fn open_compose_for(
        &mut self,
        msg: &Message,
        body: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.opened = Some(msg.id);
        let compose = cx.new(|cx| {
            let mut c = ComposeReply::new(msg, window, cx);
            if let Some(body) = body {
                c.set_body(&body, window, cx);
            }
            c
        });
        self._modal_sub = Some(cx.subscribe_in(
            &compose,
            window,
            |this, _, event: &ComposeEvent, window, cx| {
                match event {
                    ComposeEvent::Send { in_reply_to, body } => {
                        let now = this.now();
                        this.mailbox.send_reply_at(*in_reply_to, body.clone(), now);
                        this.close_modals(window, cx);
                        this.show_toast(
                            "Reply queued · moved to waiting · u to undo send".into(),
                            window,
                            cx,
                        );
                    }
                    ComposeEvent::Cancel => this.close_modals(window, cx),
                }
                cx.notify();
            },
        ));
        window.focus(&compose.focus_handle(cx), cx);
        self.compose = Some(compose);
        cx.notify();
    }

    fn open_snooze(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.in_session() {
            self.mark(TriageState::Later, window, cx);
            return;
        }
        if self.target_ids().is_empty() {
            return;
        }
        let now = self.now();
        let presets: Vec<(String, Timestamp)> = self
            .mailbox
            .snooze_presets(now)
            .into_iter()
            .map(|(l, t)| (l.to_string(), t))
            .collect();
        let picker = cx.new(|cx| {
            SnoozePicker::new(presets, now, crate::model::parse_snooze, window, cx)
        });
        self._modal_sub = Some(cx.subscribe_in(
            &picker,
            window,
            |this, _, event: &SnoozeEvent, window, cx| {
                let pick = match event {
                    SnoozeEvent::Pick(ts) => Some(*ts),
                    SnoozeEvent::Cancel => None,
                };
                this.close_modals(window, cx);
                if let Some(until) = pick {
                    let ids = this.target_ids();
                    let now = this.now();
                    let n = this.mailbox.snooze(&ids, until, now);
                    this.triage.clear_selection();
                    this.show_toast(
                        format!("Snoozed {n} until {} · u to undo", format_when(until)),
                        window,
                        cx,
                    );
                    this.scroll_to_cursor();
                }
            },
        ));
        window.focus(&picker.focus_handle(cx), cx);
        self.snooze = Some(picker);
        cx.notify();
    }

    fn toggle_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings.is_some() {
            self.close_modals(window, cx);
            return;
        }
        if self.modal_open() {
            return;
        }
        let panel = cx.new(|cx| SettingsPanel::new(self.policy.clone(), self.summaries_enabled, cx));
        self._modal_sub = Some(cx.subscribe_in(
            &panel,
            window,
            |this, _, event: &SettingsEvent, window, cx| match event {
                SettingsEvent::Changed(policy, summaries) => {
                    this.policy = policy.clone();
                    this.summaries_enabled = *summaries;
                    cx.notify();
                }
                SettingsEvent::Close => this.close_modals(window, cx),
            },
        ));
        window.focus(&panel.focus_handle(cx), cx);
        self.settings = Some(panel);
        cx.notify();
    }

    fn toggle_rules(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.rules_panel.is_some() {
            self.close_modals(window, cx);
            return;
        }
        if self.modal_open() {
            return;
        }
        let panel = cx.new(|cx| RulesPanel::new(self.rules.rules().to_vec(), cx));
        self._modal_sub = Some(cx.subscribe_in(
            &panel,
            window,
            |this, panel, event: &RulesEvent, window, cx| match event {
                RulesEvent::Revoke(ix) => {
                    this.rules.revoke(*ix);
                    let rules = this.rules.rules().to_vec();
                    panel.update(cx, |p, cx| {
                        p.set_rules(rules, cx);
                    });
                    this.show_toast("Rule revoked".into(), window, cx);
                }
                RulesEvent::Close => this.close_modals(window, cx),
            },
        ));
        window.focus(&panel.focus_handle(cx), cx);
        self.rules_panel = Some(panel);
        cx.notify();
    }

    fn mute_thread(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(thread) = self.cursor_id().and_then(|id| self.mailbox.get(id)).map(|m| m.thread_id)
        else {
            return;
        };
        self.mailbox.mute_thread(thread);
        self.show_toast("Thread muted · u to undo".into(), window, cx);
        self.session_advance();
        cx.notify();
    }

    fn unsubscribe(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(email) = self
            .cursor_id()
            .and_then(|id| self.mailbox.get(id))
            .map(|m| m.from_email.clone())
        else {
            return;
        };
        self.mailbox.unsubscribe(&email);
        self.show_toast(format!("Unsubscribed from {email} · u to undo"), window, cx);
        self.session_advance();
        cx.notify();
    }

    fn screen_sender(&mut self, allow: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode != ListMode::Screener {
            return;
        }
        let Some(email) = self
            .cursor_id()
            .and_then(|id| self.mailbox.get(id))
            .map(|m| m.from_email.clone())
        else {
            return;
        };
        let text = if allow {
            self.mailbox.allow_sender(&email);
            format!("Allowed {email} · u to undo")
        } else {
            self.mailbox.block_sender(&email);
            format!("Blocked {email} · u to undo")
        };
        self.show_toast(text, window, cx);
        self.move_cursor(0);
        cx.notify();
    }

    fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.help {
            self.help = false;
        } else if self.modal_open() {
            self.close_modals(window, cx);
        } else if matches!(self.mode, ListMode::Search(_)) {
            self.mode = ListMode::State;
        } else if let Some(s) = self.session.take() {
            if s.handled > 0 {
                self.session_end = Some((s.handled, self.now() - s.started));
            }
            self.opened = None;
        } else if self.session_end.is_some() {
            self.end_session();
            self.opened = None;
        } else {
            self.triage.clear_selection();
        }
        cx.notify();
    }

    // ---- rendering helpers ----

    fn hint_mode(&self) -> HintMode {
        if self.compose.is_some() {
            HintMode::Compose
        } else if self.snooze.is_some() {
            HintMode::Snooze
        } else if self.settings.is_some() {
            HintMode::Settings
        } else if self.rules_panel.is_some() {
            HintMode::Rules
        } else if self.in_session() || self.session_end.is_some() {
            HintMode::Session(self.session_end.is_some())
        } else if self.mode == ListMode::Screener {
            HintMode::Screener
        } else {
            match self.triage.selected().len() {
                0 => HintMode::List,
                n => HintMode::Selection(n),
            }
        }
    }

    fn clock_label(received: &str, newest: &str) -> String {
        const MONTHS: [&str; 12] = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];
        let date = received.get(..10).unwrap_or(received);
        if date == newest.get(..10).unwrap_or(newest) {
            return received.get(11..16).unwrap_or("").to_string();
        }
        let month = date
            .get(5..7)
            .and_then(|m| m.parse::<usize>().ok())
            .and_then(|m| MONTHS.get(m.wrapping_sub(1)))
            .unwrap_or(&"?");
        let day = date.get(8..10).and_then(|d| d.parse::<u32>().ok()).unwrap_or(0);
        format!("{month} {day}")
    }

    fn newest(&self) -> String {
        self.mailbox
            .messages()
            .iter()
            .map(|m| m.received.as_str())
            .max()
            .unwrap_or("")
            .to_string()
    }

    fn render_row(&self, msg: &Message, ix: usize, newest: &str, cx: &App) -> Stateful<Div> {
        let t = cx.theme();
        let is_cursor = ix == self.cursor_ix();
        let selected = self.mode == ListMode::State && self.triage.is_selected(msg.id);
        let date = match self.mailbox.snoozed_until(msg.id) {
            Some(until) if self.triage.view == TriageState::Later && self.mode == ListMode::State => {
                format!("↩ {}", format_when(until))
            }
            _ => Self::clock_label(&msg.received, newest),
        };
        let pending = self.mailbox.pending(msg.id);
        div()
            .id(("row", msg.id as usize))
            .h(px(ROW_H))
            .w_full()
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .text_size(px(13.))
            .border_l_2()
            .border_color(if is_cursor { t.primary } else { t.transparent })
            .when(selected, |d| d.bg(t.primary.opacity(0.16)))
            .when(is_cursor && !selected, |d| d.bg(t.list_active))
            .child(
                div()
                    .w(px(10.))
                    .text_color(t.primary)
                    .child(if selected { "●" } else { "" }),
            )
            .child(
                div()
                    .w(px(120.))
                    .flex_none()
                    .truncate()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(msg.from_name.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .truncate()
                    .text_color(t.muted_foreground)
                    .child(msg.subject.clone()),
            )
            .child(badges(self.mailbox.tags(msg.id), &pending))
            .child(
                div()
                    .flex_none()
                    .text_size(px(11.))
                    .text_color(t.muted_foreground)
                    .child(date),
            )
    }

    fn render_list(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme();
        let count = self.visible_ids().len();
        let selected = self.triage.selected().len();
        let title = match &self.mode {
            ListMode::State => format!("{} · {count}", self.triage.view.label().to_uppercase()),
            ListMode::Screener => format!("SCREENER · {count}"),
            ListMode::Search(q) => format!("search: {q} · {count}"),
        };
        let header = div()
            .h(px(28.))
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .px_3()
            .text_size(px(11.))
            .text_color(t.muted_foreground)
            .child(title)
            .when(selected > 0, |d| {
                d.child(
                    div()
                        .text_color(t.primary)
                        .child(format!("{selected} selected")),
                )
            });
        let body = if count == 0 {
            let view = self.triage.view;
            let empty = match &self.mode {
                ListMode::State => EmptyState::new(view).into_any_element(),
                ListMode::Screener => div().child("No new senders").into_any_element(),
                ListMode::Search(_) => div().child("No matches").into_any_element(),
            };
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(t.muted_foreground)
                .child(empty)
                .into_any_element()
        } else {
            uniform_list(
                "messages",
                count,
                cx.processor(|this, range: std::ops::Range<usize>, _window, cx| {
                    let ids = this.visible_ids();
                    let newest = this.newest();
                    let mut rows = Vec::with_capacity(range.len());
                    for ix in range {
                        if let Some(msg) = ids.get(ix).and_then(|id| this.mailbox.get(*id)) {
                            rows.push(this.render_row(msg, ix, &newest, cx));
                        }
                    }
                    rows
                }),
            )
            .track_scroll(&self.list_scroll)
            .flex_1()
            .into_any_element()
        };
        div()
            .w(px(460.))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .border_r_1()
            .border_color(t.border)
            .child(header)
            .when(self.mode == ListMode::Screener, |d| {
                d.child(ScreenerHeader::new(count))
            })
            .child(body)
            .into_any_element()
    }

    fn render_reader(&self, cx: &App) -> AnyElement {
        let t = cx.theme();
        let pane = div().flex_1().h_full().min_w_0().flex().flex_col().px_5().py_4();
        if let Some((handled, secs)) = self.session_end {
            return pane
                .items_center()
                .justify_center()
                .child(SessionCard::finished(handled, secs))
                .into_any_element();
        }
        let Some(msg) = self.opened.and_then(|id| self.mailbox.get(id)) else {
            return pane
                .items_center()
                .justify_center()
                .text_size(px(13.))
                .text_color(t.muted_foreground)
                .child("enter to open")
                .into_any_element();
        };
        let state = self.mailbox.state_of(msg.id).unwrap_or_default();
        let newest = self.newest();
        let mut thread: Vec<&Message> = self
            .mailbox
            .messages()
            .iter()
            .filter(|m| m.thread_id == msg.thread_id)
            .collect();
        thread.sort_by(|a, b| a.received.cmp(&b.received));
        let thread_len = thread.len();
        let summary = self.summary_shown();
        pane.gap_3()
            .when_some(self.session.as_ref(), |d, s| {
                d.child(SessionCard::new(s.index + 1, s.ids.len()))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .text_size(px(16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(msg.subject.clone()),
                    )
                    .child(UiTag::secondary().outline().child(state.label())),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(t.muted_foreground)
                    .child(format!(
                        "{} <{}> → {} · {}",
                        msg.from_name,
                        msg.from_email,
                        msg.to,
                        Self::clock_label(&msg.received, &newest)
                    )),
            )
            .child(
                div()
                    .id("reader-body")
                    .flex_1()
                    .overflow_y_scroll()
                    .text_size(px(13.))
                    .line_height(relative(1.5))
                    .child(msg.body.clone()),
            )
            .when_some(summary, |d, s| d.child(SummaryCard::new(&s)))
            .when(thread_len > 1, |d| {
                d.child(
                    div()
                        .flex_none()
                        .pt_2()
                        .border_t_1()
                        .border_color(t.border)
                        .flex()
                        .flex_col()
                        .text_size(px(12.))
                        .child(
                            div()
                                .pb_1()
                                .text_size(px(11.))
                                .text_color(t.muted_foreground)
                                .child(format!("THREAD · {thread_len}")),
                        )
                        .children(thread.into_iter().map(|m| {
                            let here = m.id == msg.id;
                            div()
                                .h(px(22.))
                                .flex()
                                .items_center()
                                .gap_2()
                                .text_color(if here { t.foreground } else { t.muted_foreground })
                                .child(
                                    div()
                                        .w(px(110.))
                                        .flex_none()
                                        .truncate()
                                        .child(m.from_name.clone()),
                                )
                                .child(div().flex_1().truncate().child(m.body.replace('\n', " ")))
                                .child(
                                    div()
                                        .flex_none()
                                        .text_size(px(11.))
                                        .child(Self::clock_label(&m.received, &newest)),
                                )
                        })),
                )
            })
            .into_any_element()
    }
}

/// `Wed Oct 7 08:00` (UTC) for snooze return times.
fn format_when(ts: Timestamp) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    let days = ts.div_euclid(DAY);
    let secs = ts.rem_euclid(DAY);
    let weekday = WEEKDAYS[(days + 4).rem_euclid(7) as usize];
    // Civil-from-days (Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    format!(
        "{weekday} {} {day} {:02}:{:02}",
        MONTHS[(month - 1) as usize],
        secs / 3600,
        secs % 3600 / 60
    )
}

impl Focusable for MailApp {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for MailApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let counts = TriageState::ALL.map(|s| (s, self.mailbox.count(s)));
        let screener_count = self.mailbox.screener_ids().len();
        let screener_active = self.mode == ListMode::Screener;
        let hint = self.hint_mode();
        let in_session = self.in_session() || self.session_end.is_some();
        let list = (!in_session).then(|| self.render_list(cx));
        let reader = match &self.compose {
            Some(compose) => div()
                .flex_1()
                .h_full()
                .min_w_0()
                .child(compose.clone())
                .into_any_element(),
            None => self.render_reader(cx),
        };
        let banner = self.pending_rule.clone();
        let t = cx.theme();
        let (bg, fg, border, muted, primary, sidebar) = (
            t.background,
            t.foreground,
            t.border,
            t.muted_foreground,
            t.primary,
            t.sidebar,
        );
        let active_view = self.triage.view;
        div()
            .id("mail-app")
            .track_focus(&self.focus_handle)
            .when(!self.modal_open(), |d| d.key_context(MAIL_CONTEXT))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(bg)
            .text_color(fg)
            .text_size(px(13.))
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| {
                this.move_cursor(1);
                this.scroll_to_cursor();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &SelectPrev, _, cx| {
                this.move_cursor(-1);
                this.scroll_to_cursor();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ExtendNext, _, cx| {
                if this.mode == ListMode::State && !this.in_session() {
                    this.triage.extend(&this.mailbox, 1);
                }
                this.scroll_to_cursor();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ExtendPrev, _, cx| {
                if this.mode == ListMode::State && !this.in_session() {
                    this.triage.extend(&this.mailbox, -1);
                }
                this.scroll_to_cursor();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleSelect, _, cx| {
                if this.mode == ListMode::State && !this.in_session() {
                    this.triage.toggle_select(&this.mailbox);
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ClearSelection, window, cx| this.escape(window, cx)))
            .on_action(cx.listener(|this, _: &OpenMessage, _, cx| {
                if let Some(id) = this.cursor_id() {
                    this.opened = Some(id);
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &MarkDone, w, cx| this.mark(TriageState::Done, w, cx)))
            .on_action(cx.listener(|this, _: &MarkWaiting, w, cx| this.mark(TriageState::Waiting, w, cx)))
            .on_action(cx.listener(|this, _: &MarkLater, w, cx| this.mark(TriageState::Later, w, cx)))
            .on_action(cx.listener(|this, _: &MoveToInbox, w, cx| this.mark(TriageState::Inbox, w, cx)))
            .on_action(cx.listener(|this, _: &SenderDone, w, cx| this.mark_sender(TriageState::Done, w, cx)))
            .on_action(cx.listener(|this, _: &SenderWaiting, w, cx| this.mark_sender(TriageState::Waiting, w, cx)))
            .on_action(cx.listener(|this, _: &SenderLater, w, cx| this.mark_sender(TriageState::Later, w, cx)))
            .on_action(cx.listener(|this, _: &SenderInbox, w, cx| this.mark_sender(TriageState::Inbox, w, cx)))
            .on_action(cx.listener(|this, _: &Undo, window, cx| this.undo(window, cx)))
            .on_action(cx.listener(|this, _: &ToggleCommandPalette, window, cx| {
                if this.palette.is_some() {
                    this.close_modals(window, cx);
                } else if !this.modal_open() {
                    this.open_palette(None, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &OpenSearch, window, cx| {
                if !this.modal_open() {
                    this.open_palette(Some("/"), window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &Reply, window, cx| {
                if !this.modal_open() {
                    this.open_compose(window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &CancelCompose, window, cx| {
                if this.compose.is_some() {
                    this.close_modals(window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ShowInbox, _, cx| this.show_view(TriageState::Inbox, cx)))
            .on_action(cx.listener(|this, _: &ShowWaiting, _, cx| this.show_view(TriageState::Waiting, cx)))
            .on_action(cx.listener(|this, _: &ShowLater, _, cx| this.show_view(TriageState::Later, cx)))
            .on_action(cx.listener(|this, _: &ShowDone, _, cx| this.show_view(TriageState::Done, cx)))
            .on_action(cx.listener(|this, _: &ShowScreener, _, cx| this.show_screener(cx)))
            .on_action(cx.listener(|this, _: &ToggleHelp, _, cx| {
                this.help = !this.help;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &OpenSnoozePicker, w, cx| {
                if !this.modal_open() {
                    this.open_snooze(w, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &AcceptSuggestions, _, cx| {
                if let Some(id) = this.cursor_id() {
                    let now = this.now();
                    this.mailbox.accept_suggestions(id, now);
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &RejectSuggestions, _, cx| {
                if let Some(id) = this.cursor_id() {
                    this.mailbox.reject_suggestions(id);
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &AcceptRule, w, cx| this.accept_rule(w, cx)))
            .on_action(cx.listener(|this, _: &DismissRule, _, cx| this.dismiss_rule(cx)))
            .on_action(cx.listener(|this, _: &ToggleRules, w, cx| this.toggle_rules(w, cx)))
            .on_action(cx.listener(|this, _: &AllowSender, w, cx| this.screen_sender(true, w, cx)))
            .on_action(cx.listener(|this, _: &BlockSender, w, cx| this.screen_sender(false, w, cx)))
            .on_action(cx.listener(|this, _: &MuteThread, w, cx| this.mute_thread(w, cx)))
            .on_action(cx.listener(|this, _: &Unsubscribe, w, cx| this.unsubscribe(w, cx)))
            .on_action(cx.listener(|this, _: &SummarizeThread, w, cx| this.summarize(w, cx)))
            .on_action(cx.listener(|this, _: &ToggleSettings, w, cx| this.toggle_settings(w, cx)))
            .on_action(cx.listener(|this, _: &StartSession, _, cx| this.start_session(cx)))
            .on_action(cx.listener(|this, _: &ClassifyVisible, w, cx| {
                let (auto, review) = this.classify_visible();
                this.show_toast(format!("{auto} auto-applied · {review} to review"), w, cx);
            }))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(
                        div()
                            .w(px(148.))
                            .flex_none()
                            .h_full()
                            .bg(sidebar)
                            .border_r_1()
                            .border_color(border)
                            .child(
                                ViewTabs::new(active_view, counts)
                                    .screener(screener_count, screener_active),
                            ),
                    )
                    .children(list)
                    .child(reader),
            )
            .when_some(banner, |d, rule| {
                d.child(div().flex_none().child(RuleBanner::new(&rule)))
            })
            .child(
                div()
                    .flex_none()
                    .h(px(28.))
                    .border_t_1()
                    .border_color(border)
                    .child(HintBar::new(hint)),
            )
            .when_some(self.toast.clone(), |d, text| {
                d.child(
                    div()
                        .absolute()
                        .bottom(px(40.))
                        .left_0()
                        .right_0()
                        .flex()
                        .justify_center()
                        .child(
                            div()
                                .px_3()
                                .py_1()
                                .rounded_md()
                                .bg(primary)
                                .text_color(bg)
                                .text_size(px(12.))
                                .font_weight(FontWeight::MEDIUM)
                                .child(text),
                        ),
                )
            })
            .when(self.help, |d| {
                d.child(
                    div()
                        .absolute()
                        .inset_0()
                        .occlude()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(bg.opacity(0.85))
                        .text_color(muted)
                        .child(HelpOverlay::new()),
                )
            })
            .when_some(self.palette.clone(), |d, palette| {
                d.child(
                    div()
                        .absolute()
                        .inset_0()
                        .occlude()
                        .flex()
                        .justify_center()
                        .pt(px(80.))
                        .child(palette),
                )
            })
            .when_some(self.snooze.clone(), |d, picker| d.child(overlay(picker)))
            .when_some(self.settings.clone(), |d, panel| d.child(overlay(panel)))
            .when_some(self.rules_panel.clone(), |d, panel| d.child(overlay(panel)))
    }
}

/// Centered modal wrapper for a panel entity.
fn overlay(view: impl IntoElement) -> Div {
    div()
        .absolute()
        .inset_0()
        .occlude()
        .flex()
        .justify_center()
        .pt(px(80.))
        .child(view)
}
