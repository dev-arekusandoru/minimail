use super::*;

impl MailApp {
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

    /// A popup menu is open.
    pub fn menu_open(&self) -> bool {
        self.menu.is_some()
    }

    /// The open menu of `kind`, if that is the one showing.
    pub(super) fn menu_is(&self, kind: MenuKind) -> bool {
        self.menu.as_ref().is_some_and(|m| m.kind == kind)
    }

    /// The open menu's panel, to draw it.
    pub(super) fn menu_panel(&self) -> Option<Entity<MenuPanel>> {
        self.menu.as_ref().map(|m| m.panel.clone())
    }

    /// Whether the contextual action bar applies right now: a message to act on, in a
    /// view that is not the screener (it has its own controls), the composer or a
    /// finished session.
    pub fn context_actions(&self) -> bool {
        if self.compose.is_some() || self.mode == ListMode::Screener || self.session_end.is_some() {
            return false;
        }
        !self.target_ids().is_empty()
    }

    /// Number of AI suggestions still pending on the message the actions target.
    pub fn pending_suggestions(&self) -> usize {
        self.cursor_id()
            .map(|id| self.mailbox.pending(id).len())
            .unwrap_or(0)
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


    pub(super) fn now(&self) -> Timestamp {
        self.clock.now()
    }

    pub(super) fn search_ids(&self, q: &str) -> Vec<MessageId> {
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

    pub(super) fn in_session(&self) -> bool {
        self.session.is_some()
    }

    pub(super) fn session_current(&self) -> Option<MessageId> {
        self.session.as_ref().and_then(|s| s.ids.get(s.index).copied())
    }

    /// Message under the cursor (or the session's current message).
    pub(super) fn cursor_id(&self) -> Option<MessageId> {
        if let Some(id) = self.session_current() {
            return Some(id);
        }
        if self.grouped() {
            return self.cursor_row().map(|r| r.primary());
        }
        match self.mode {
            ListMode::State => self.triage.cursor(&self.mailbox),
            _ => {
                let ids = self.visible_ids();
                ids.get(self.alt_cursor.min(ids.len().saturating_sub(1))).copied()
            }
        }
    }

    pub(super) fn cursor_ix(&self) -> usize {
        if self.grouped() {
            return self.row_cursor();
        }
        match self.mode {
            ListMode::State => self.triage.cursor_index(),
            _ => self.alt_cursor,
        }
    }

    /// Ids an action applies to: session message, selection/cursor, or cursor.
    pub(super) fn target_ids(&self) -> Vec<MessageId> {
        if self.grouped() && self.triage.selected().is_empty() {
            return self.cursor_row().map(|r| r.ids()).unwrap_or_default();
        }
        if self.in_session() || self.mode != ListMode::State {
            return self.cursor_id().into_iter().collect();
        }
        self.triage.targets(&self.mailbox)
    }

    pub(super) fn move_cursor(&mut self, delta: isize) {
        if self.grouped() {
            let len = self.rows().len();
            let max = len.saturating_sub(1) as isize;
            self.row_cursor = (self.row_cursor() as isize + delta).clamp(0, max) as usize;
        } else if self.mode == ListMode::State {
            self.triage.move_cursor(&self.mailbox, delta);
        } else {
            let len = self.visible_ids().len();
            let max = len.saturating_sub(1) as isize;
            self.alt_cursor = (self.alt_cursor as isize + delta).clamp(0, max) as usize;
        }
    }

    pub(super) fn modal_open(&self) -> bool {
        self.palette.is_some()
            || self.compose.is_some()
            || self.snooze.is_some()
            || self.settings.is_some()
            || self.rules_panel.is_some()
    }

    /// Scroll the cursor row fully into view; rows have their own heights, so the list state
    /// works it out.
    pub(super) fn scroll_to_cursor(&self) {
        self.list_state.scroll_to_reveal_item(self.cursor_ix());
    }
}
