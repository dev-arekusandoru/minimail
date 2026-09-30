use super::*;

impl MailApp {
    pub(super) fn show_toast(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
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

    pub(super) fn close_modals(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette = None;
        self.compose = None;
        self.snooze = None;
        self.settings = None;
        self.rules_panel = None;
        self.menu = None;
        self._modal_sub = None;
        self._menu_sub = None;
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    /// After a state change in a session: count it and move to the next message.
    pub(super) fn session_advance(&mut self) {
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

    pub(super) fn mark(&mut self, state: TriageState, window: &mut Window, cx: &mut Context<Self>) {
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

    pub(super) fn mark_sender(&mut self, state: TriageState, window: &mut Window, cx: &mut Context<Self>) {
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

    pub(super) fn accept_rule(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    pub(super) fn dismiss_rule(&mut self, cx: &mut Context<Self>) {
        if let Some(rule) = self.pending_rule.take() {
            self.rules.dismiss(rule);
            cx.notify();
        }
    }

    pub(super) fn show_view(&mut self, view: TriageState, cx: &mut Context<Self>) {
        self.end_session();
        self.mode = ListMode::State;
        self.triage.switch_view(view);
        self.row_cursor = 0;
        self.row_anchor = None;
        self.opened = None;
        self.scroll_to_cursor();
        cx.notify();
    }

    pub(super) fn show_screener(&mut self, cx: &mut Context<Self>) {
        self.end_session();
        self.mode = ListMode::Screener;
        self.alt_cursor = 0;
        self.opened = None;
        cx.notify();
    }

    pub(super) fn end_session(&mut self) {
        self.session = None;
        self.session_end = None;
    }

    pub(super) fn start_session(&mut self, cx: &mut Context<Self>) {
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
    pub(super) fn classify_visible(&mut self) -> (usize, usize) {
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
                self.opened = Some(msg.id);
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
