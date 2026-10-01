use super::*;

impl MailApp {
    /// Show `text` as a kit notification at the bottom of the window. A new toast replaces the
    /// previous one; one that mentions undo carries an Undo button. `self.toast` mirrors the
    /// visible text because the kit keeps its notification text private.
    pub(super) fn show_toast(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.toast_gen += 1;
        let generation = self.toast_gen;
        let text: SharedString = text.into();
        self.toast = Some(text.clone());
        let app = cx.weak_entity();
        let mut note = Notification::success(text.clone())
            .id::<ToastId>()
            .placement(Anchor::BottomCenter)
            .on_close(move |_, cx| {
                app.update(cx, |this, cx| {
                    if this.toast_gen == generation {
                        this.toast = None;
                        cx.notify();
                    }
                })
                .ok();
            });
        if text.contains("undo") {
            note = note.content(|_, _, cx| {
                button("toast-undo", "Undo", "Undo", "u", cx)
                    .on_click(cx.listener(|note, _, window, cx| {
                        window.dispatch_action(Box::new(Undo), cx);
                        note.dismiss(window, cx);
                    }))
                    .into_any_element()
            });
        }
        window.push_notification(note, cx);
        cx.notify();
    }

    pub(super) fn close_modals(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog_hosted() {
            window.close_dialog(cx);
        }
        self.palette = None;
        self.compose = None;
        self.help = None;
        self.snooze = None;
        self.settings = None;
        self.rules_panel = None;
        self.dialog = None;
        self.folder_picker = None;
        self.menu = None;
        self._modal_sub = None;
        self._menu_sub = None;
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    /// After a state change in a session: count it and move to the next message.
    pub(super) fn session_advance(&mut self) {
        // A menu acting on a message other than the session's current one handles nothing
        // of the session.
        if let (Some(ids), Some(current)) = (&self.menu_target, self.session_current())
            && !ids.contains(&current)
        {
            return;
        }
        let Some(s) = self.session.as_mut() else { return };
        s.handled += 1;
        s.index += 1;
        if s.index >= s.ids.len() {
            let elapsed = self.clock.now() - s.started;
            let handled = s.handled;
            self.session = None;
            self.session_end = Some((handled, elapsed));
        }
    }

    pub(super) fn mark(&mut self, state: TriageState, window: &mut Window, cx: &mut Context<Self>) {
        let ids = self.target_ids();
        if ids.is_empty() {
            return;
        }
        let n = self.mailbox.set_state(&ids, state);
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

    /// Move the sender's mail in `from` to `state` (after the confirm dialog).
    pub(super) fn mark_sender(
        &mut self,
        email: &str,
        name: &str,
        from: TriageState,
        state: TriageState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (name, email) = (name.to_owned(), email.to_owned());
        let n = self.mailbox.set_state_for_sender(&email, from, state);
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

    pub(super) fn accept_rule(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rule) = self.pending_rule.take() else { return };
        self.rules.accept(rule.clone());
        let ids: Vec<MessageId> = self
            .mailbox
            .ids_in_view(&View::default())
            .into_iter()
            .filter(|id| {
                self.mailbox
                    .get(*id)
                    .is_some_and(|m| m.from_email == rule.sender)
            })
            .collect();
        let n = self.mailbox.set_state(&ids, rule.state);
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

    pub(super) fn dismiss_rule(&mut self, cx: &mut Context<Self>) {
        if let Some(rule) = self.pending_rule.take() {
            self.rules.dismiss(rule);
            cx.notify();
        }
    }

    pub(super) fn show_view(&mut self, view: View, cx: &mut Context<Self>) {
        self.end_session();
        self.mode = ListMode::State;
        self.triage.switch_view(view);
        self.row_cursor = 0;
        self.row_anchor = None;
        self.scroll_to_cursor();
        cx.notify();
    }

    /// Show a location the way the sidebar does: chip and filters reset.
    pub(super) fn show_location(&mut self, location: Location, cx: &mut Context<Self>) {
        self.show_view(
            View {
                location,
                ..View::default()
            },
            cx,
        );
    }

    /// Re-point the current view without leaving the reader (chips and filters).
    fn filter_view(&mut self, view: View, cx: &mut Context<Self>) {
        self.end_session();
        self.mode = ListMode::State;
        self.triage.switch_view(view);
        self.row_cursor = 0;
        self.row_anchor = None;
        self.scroll_to_cursor();
        cx.notify();
    }

    /// Pick a chip. Chips live on Inbox views only, so elsewhere this is a no-op.
    pub(super) fn select_chip(&mut self, chip: Chip, cx: &mut Context<Self>) {
        if !sidebar::is_inbox_location(&self.triage.view.location) {
            return;
        }
        let view = View {
            chip,
            ..self.triage.view.clone()
        };
        self.filter_view(view, cx);
    }

    /// Add or remove one tag from the Filter ▾ menu.
    pub(super) fn toggle_tag_filter(&mut self, tag: TagFilter, cx: &mut Context<Self>) {
        let mut view = self.triage.view.clone();
        if let Some(ix) = view.filter.tags.iter().position(|t| *t == tag) {
            view.filter.tags.remove(ix);
        } else {
            view.filter.tags.push(tag);
        }
        self.filter_view(view, cx);
    }

    /// Pick the Filter ▾ menu's Kind (`None` = any kind).
    pub(super) fn set_filter_kind(&mut self, kind: Option<Kind>, cx: &mut Context<Self>) {
        let mut view = self.triage.view.clone();
        view.filter.kind = kind;
        self.filter_view(view, cx);
    }

    /// Pick the Filter ▾ menu's account (`None` = every account).
    pub(super) fn set_filter_account(
        &mut self,
        account: Option<AccountId>,
        cx: &mut Context<Self>,
    ) {
        let mut view = self.triage.view.clone();
        view.filter.account = account;
        self.filter_view(view, cx);
    }

    /// Drop every Filter ▾ entry (the chip stays as it is).
    pub(super) fn clear_filters(&mut self, cx: &mut Context<Self>) {
        let mut view = self.triage.view.clone();
        view.filter = Filter::default();
        self.filter_view(view, cx);
    }

    /// Fold or unfold a folder's children in the sidebar.
    pub(super) fn toggle_folder(&mut self, folder: FolderId, cx: &mut Context<Self>) {
        if !self.collapsed_folders.remove(&folder) {
            self.collapsed_folders.insert(folder);
        }
        cx.notify();
    }

    /// Fold or unfold an account's section in the sidebar.
    pub(super) fn toggle_account(&mut self, account: AccountId, cx: &mut Context<Self>) {
        if !self.collapsed_accounts.remove(&account) {
            self.collapsed_accounts.insert(account);
        }
        cx.notify();
    }

    /// `g i`: the current account's Inbox, or All Inboxes when already there.
    pub(super) fn go_inbox(&mut self, cx: &mut Context<Self>) {
        let location = match self.triage.view.location.clone() {
            Location::AllInboxes => Location::AllInboxes,
            _ => match self.nav_account() {
                Some(account) => Location::Inbox(account),
                None => return,
            },
        };
        self.show_location(location, cx);
    }

    /// `g s` / `g t` / `g a` / `g d`: the current account's location.
    fn go_account_location(
        &mut self,
        location: fn(AccountId) -> Location,
        cx: &mut Context<Self>,
    ) {
        if let Some(account) = self.nav_account() {
            self.show_location(location(account), cx);
        }
    }

    pub(super) fn go_snoozed(&mut self, cx: &mut Context<Self>) {
        self.go_account_location(Location::Snoozed, cx);
    }

    pub(super) fn go_sent(&mut self, cx: &mut Context<Self>) {
        self.go_account_location(Location::Sent, cx);
    }

    pub(super) fn go_archive(&mut self, cx: &mut Context<Self>) {
        self.go_account_location(Location::Archive, cx);
    }

    pub(super) fn go_trash(&mut self, cx: &mut Context<Self>) {
        self.go_account_location(Location::Trash, cx);
    }

    pub(super) fn end_session(&mut self) {
        self.session = None;
        self.session_end = None;
    }

    pub(super) fn start_session(&mut self, cx: &mut Context<Self>) {
        let ids = self.mailbox.ids_in_view(&View::default());
        if ids.is_empty() {
            return;
        }
        self.mode = ListMode::State;
        self.triage.switch_view(View::default());
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
    pub(super) fn classify_visible(&mut self) -> (usize, usize) {
        let ids: Vec<MessageId> = self
            .mailbox
            .ids_in_view(&View::default())
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

    pub(super) fn summarize(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
                self.open_message(msg.id, false);
            }
            Err(e) => self.show_toast(format!("Summary failed: {e:?}"), window, cx),
        }
        cx.notify();
    }

    pub(super) fn undo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
}
