use super::*;
use crate::search::Field;

impl MailApp {
    pub fn palette_open(&self) -> bool {
        self.palette.is_some()
    }

    /// Whether a draft is being composed anywhere: inline under a thread or in a tab.
    pub fn compose_open(&self) -> bool {
        self.inline_composer.is_some() || self.tabs.has_drafts()
    }

    /// Whether the active tab is a draft.
    pub fn compose_in_tab(&self) -> bool {
        self.tabs.active_draft().is_some()
    }

    /// Whether a reply is open inline under the conversation it answers.
    pub fn inline_compose_open(&self) -> bool {
        self.inline_composer.is_some()
    }

    /// The drafts that currently have a tab. An inline reply has none, so this is not the
    /// count of open drafts: [`MailApp::composer`] holds every one of them.
    pub fn open_drafts(&self) -> Vec<DraftId> {
        self.tabs.draft_ids()
    }

    /// Whether the Drafts pane is the list the app is showing.
    pub fn drafts_pane_open(&self) -> bool {
        self.in_drafts()
    }

    /// The draft rows the Drafts pane shows, newest edit first: every account's unfinished
    /// drafts in the All view, else the current account's. Empty while another location is
    /// the list.
    pub fn draft_rows(&self) -> Vec<crate::app::compose::drafts::DraftRow> {
        match self.location() {
            Some(Location::Drafts(account)) => self.account_drafts(Some(&account)),
            Some(Location::AllDrafts) => self.account_drafts(None),
            _ => Vec::new(),
        }
    }

    pub fn help_open(&self) -> bool {
        self.help.is_some()
    }

    pub fn snooze_open(&self) -> bool {
        self.snooze.is_some()
    }

    pub fn settings_open(&self) -> bool {
        self.settings.is_some()
    }

    /// The settings window, if it is open.
    pub fn settings_window(&self) -> Option<AnyWindowHandle> {
        self.settings_window
    }

    /// The settings panel of the open settings window.
    pub fn settings_panel(&self) -> Option<Entity<SettingsPanel>> {
        self.settings.clone()
    }

    /// The theme preferences in force: mode plus the light and dark theme names.
    pub fn theme_preferences(&self) -> (crate::theme::ThemeMode, String, String) {
        (
            self.theme_mode,
            self.light_theme.clone(),
            self.dark_theme.clone(),
        )
    }

    pub fn rules_open(&self) -> bool {
        self.rules_panel.is_some()
    }

    /// A choice dialog is open.
    pub fn dialog_open(&self) -> bool {
        self.dialog.is_some()
    }

    /// The folder picker is open.
    pub fn folder_picker_open(&self) -> bool {
        self.folder_picker.is_some()
    }

    /// Title of the open choice dialog, if any.
    pub fn dialog_title(&self, cx: &App) -> Option<String> {
        self.dialog.as_ref().map(|d| d.read(cx).title())
    }

    /// Option labels of the open choice dialog, top to bottom.
    pub fn dialog_options(&self, cx: &App) -> Vec<String> {
        self.dialog
            .as_ref()
            .map(|d| d.read(cx).options())
            .unwrap_or_default()
    }

    /// Rows of the open folder picker, top to bottom.
    pub fn folder_rows(&self, cx: &App) -> Vec<String> {
        self.folder_picker
            .as_ref()
            .map(|p| p.read(cx).rows())
            .unwrap_or_default()
    }

    pub fn new_senders_open(&self) -> bool {
        crate::known_senders::KNOWN_SENDERS_ENABLED
            && self.triage().query.has(Field::Is, "new")
    }

    /// Titlebar / list-header label for the current location, e.g. `"Work · Archive"`.
    /// A query that is not anchored to one place (global search, several `in:` values)
    /// reads as `"All mail"`.
    pub fn location_label(&self) -> String {
        self.location().map_or_else(
            || "All mail".to_owned(),
            |loc| sidebar::location_label(&loc, &self.mailbox),
        )
    }

    /// A popup menu is open.
    pub fn menu_open(&self) -> bool {
        self.open_menu.is_some()
    }

    /// The open menu of `kind`, if that is the one showing.
    pub(super) fn menu_is(&self, kind: MenuKind) -> bool {
        self.open_menu == Some(kind)
    }

    /// The message shown in the reader: the session's current one during a session, nothing
    /// on the session end card, otherwise the active tab's message.
    pub fn opened(&self) -> Option<MessageId> {
        match self.slot() {
            Slot::Triage => self.session_current(),
            Slot::Tab => self.tabs.opened(),
        }
    }

    /// Rule suggestion currently shown in the banner.
    pub fn pending_rule(&self) -> Option<Rule> {
        self.pending_rule.clone()
    }

    /// `(current 1-based, total)` while a session is running (see [`crate::triage_session`]).
    pub fn session_progress(&self) -> Option<(usize, usize)> {
        self.session.as_ref().map(|s| s.run.progress())
    }

    /// The Start screen's state, while it is up.
    pub fn session_setup(&self) -> Option<&SetupView> {
        self.session_setup.as_ref()
    }

    /// The Summary screen's numbers, while it is up.
    pub fn session_summary(&self) -> Option<&SessionSummary> {
        self.session_summary.as_ref().map(|(summary, _)| summary)
    }

    /// The suggested action for the current message; `None` with suggestions off or no advice.
    pub fn session_suggestion(&self) -> Option<&ActionSuggestion> {
        self.session.as_ref().and_then(|s| s.suggestion.as_ref())
    }

    /// The options of the running session.
    pub fn session_options(&self) -> Option<&SessionOptions> {
        self.session.as_ref().map(|s| &s.run.options)
    }

    /// Summary displayed for the opened thread, if any.
    pub fn summary_shown(&self) -> Option<ThreadSummary> {
        let thread = self.opened().and_then(|id| self.mailbox.get(id))?.thread_id;
        match &self.summary {
            Some((t, s)) if *t == thread => Some(s.clone()),
            _ => None,
        }
    }

    /// Text in the open palette's input.
    pub fn palette_query(&self, cx: &App) -> Option<String> {
        self.palette.as_ref().map(|p| p.read(cx).query())
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

    /// Exactly the message ids the list shows, top to bottom: collapsed days hold none.
    pub fn visible_ids(&self) -> Vec<MessageId> {
        self.mailbox.ids_shown(&self.triage().query, &self.local_now())
    }

    /// [`Self::visible_ids`] memoized for the current render pass. [`Self::render`] clears the
    /// memo before it starts, so every reader within one frame sees the same list without
    /// rescanning the mailbox; calls from outside a render must keep using `visible_ids`.
    pub(super) fn visible_in_frame(&self) -> Rc<Vec<MessageId>> {
        if let Some(ids) = self.visible_cache.borrow().as_ref() {
            return ids.clone();
        }
        let ids = Rc::new(self.visible_ids());
        *self.visible_cache.borrow_mut() = Some(ids.clone());
        ids
    }

    /// The sidebar's visible-location counts, memoized for one render pass like
    /// [`Self::visible_in_frame`], so the mailbox is scanned once instead of once per
    /// counted location.
    pub(super) fn sidebar_counts(&self) -> Rc<crate::model::SidebarCounts> {
        if let Some(counts) = self.sidebar_cache.borrow().as_ref() {
            return counts.clone();
        }
        let counts = Rc::new(self.mailbox.sidebar_counts());
        *self.sidebar_cache.borrow_mut() = Some(counts.clone());
        counts
    }

    pub fn now(&self) -> Timestamp {
        self.clock.now()
    }

    /// The current instant together with the user's zone: what every date a person reads,
    /// and every relative date, is resolved against.
    pub fn local_now(&self) -> Now {
        Now::new(self.clock.now(), self.zone.clone())
    }

    pub(super) fn in_session(&self) -> bool {
        self.session.is_some()
    }

    pub(super) fn session_current(&self) -> Option<MessageId> {
        self.session.as_ref().and_then(|s| s.run.current())
    }

    /// Message the actions target: the menu's message, else the one under the cursor (or the
    /// session's current message).
    pub(super) fn cursor_id(&self) -> Option<MessageId> {
        if let Some(id) = self.menu_target.as_ref().and_then(|ids| ids.first()) {
            return Some(*id);
        }
        if self.triage_open() && self.slot() == Slot::Triage {
            return self.session_current();
        }
        if self.grouped() {
            return self.cursor_row().map(|r| r.primary());
        }
        self.triage().cursor(&self.mailbox, &self.local_now())
    }

    pub(super) fn cursor_ix(&self) -> usize {
        if self.in_drafts() {
            return self.draft_cursor_ix();
        }
        if self.grouped() {
            return self.row_cursor();
        }
        self.triage().cursor_index()
    }

    /// Whether the Drafts pane is the list the app is showing. Drafts are not messages, so
    /// the list, the cursor and the open key all take their drafts-pane path.
    pub(super) fn in_drafts(&self) -> bool {
        matches!(self.location(), Some(Location::Drafts(_) | Location::AllDrafts))
    }

    /// The drafts `account` (`None` for every account) still has to finish that the active
    /// query matches, newest edit first. Filtering is the pane's; the sidebar badge counts the
    /// unfiltered set.
    pub(super) fn account_drafts(
        &self,
        account: Option<&str>,
    ) -> Vec<crate::app::compose::drafts::DraftRow> {
        crate::app::compose::drafts::rows(
            self.composer.drafts.iter().map(|d| &d.record),
            account,
            self.query(),
        )
    }

    /// Index of the draft under the cursor, clamped to the rows.
    fn draft_cursor_ix(&self) -> usize {
        let len = self.draft_rows().len();
        self.draft_cursor.min(len.saturating_sub(1))
    }

    /// The draft under the cursor while the Drafts pane is showing.
    pub fn cursor_draft(&self) -> Option<crate::app::compose::drafts::DraftRow> {
        self.draft_rows().into_iter().nth(self.draft_cursor_ix())
    }

    /// Ids an action applies to: the menu's target, session message, selection/cursor, or cursor.
    pub(super) fn target_ids(&self) -> Vec<MessageId> {
        // The Start and Summary screens act on nothing.
        if self.triage_open() && self.session.is_none() {
            return Vec::new();
        }
        if let Some(ids) = &self.menu_target {
            return ids.clone();
        }
        if self.grouped() && self.triage().selected().is_empty() {
            return self.cursor_row().map(|r| r.ids()).unwrap_or_default();
        }
        if self.in_session() {
            return self.cursor_id().into_iter().collect();
        }
        self.triage().targets(&self.mailbox, &self.local_now())
    }

    pub(super) fn move_cursor(&mut self, delta: isize) {
        if self.in_drafts() {
            let max = self.draft_rows().len().saturating_sub(1) as isize;
            self.draft_cursor = (self.draft_cursor as isize + delta).clamp(0, max) as usize;
        } else if self.grouped() {
            let len = self.rows().len();
            let max = len.saturating_sub(1) as isize;
            let next = (self.row_cursor() as isize + delta).clamp(0, max) as usize;
            self.view_mut().row_cursor = next;
        } else {
            let now = self.local_now();
            let (view, mailbox) = self.view_and_mailbox();
            view.triage.move_cursor(mailbox, &now, delta);
        }
    }

    /// A modal view lives in a kit dialog (everything but the composer).
    pub(super) fn dialog_hosted(&self) -> bool {
        self.palette.is_some()
            || self.help.is_some()
            || self.snooze.is_some()
            || self.rules_panel.is_some()
            || self.dialog.is_some()
            || self.folder_picker.is_some()
            || self.new_folder.is_some()
    }

    /// The documents the email views loaded without a native window (the headless platform).
    pub fn webview_loads(&self) -> Vec<crate::app::email_webview::LoadedDoc> {
        self.webviews.loads()
    }

    /// Any modal view is open. A draft in a tab is not a modal: it lives in the same strip
    /// as the threads, so list keys keep working around it.
    pub fn modal_open(&self) -> bool {
        self.dialog_hosted()
    }

    /// Scroll the cursor row fully into view; rows have their own heights, so the list state
    /// works it out. Re-clamps first: mailbox changes shrink the list without touching the cursor.
    pub(super) fn scroll_to_cursor(&mut self) {
        self.move_cursor(0);
        let item = self.list_model().item_of_row(self.cursor_ix());
        self.view().list_state.scroll_to_reveal_item(item);
    }

    /// The reader's scroll state for `thread`, as `(offset_y, max_offset_y)` in pixels, so a
    /// test can tell where a thread sits without driving a real scrollbar.
    pub fn reader_scroll(&self, thread: u32) -> Option<(f32, f32)> {
        let panes = self.reader_panes.borrow();
        let pane = panes.get(&thread)?;
        Some((
            f32::from(pane.scroll.offset().y),
            f32::from(pane.scroll.max_offset().y),
        ))
    }

    /// The open composer's subject scroll, in pixels, so a test can tell whether the field
    /// slid the text under its own left edge.
    pub fn subject_scroll(&self, cx: &App) -> Option<f32> {
        let id = self.tabs.active_draft().or(self.inline_composer)?;
        let composer = self.composers.get(&id)?;
        Some(f32::from(composer.read(cx).subject_scroll(cx)))
    }
}
