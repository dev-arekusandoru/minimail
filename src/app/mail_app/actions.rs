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
        self.rules_panel = None;
        self.dialog = None;
        self.folder_picker = None;
        self._modal_sub = None;
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
        self.file_ids(&ids, state, window, cx);
    }

    /// Move `ids` to `state` as one undo step: clear the selection, toast, advance a session.
    pub(super) fn file_ids(
        &mut self,
        ids: &[MessageId],
        state: TriageState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if ids.is_empty() {
            return;
        }
        let n = self.mailbox.set_state(ids, state);
        self.triage_mut().clear_selection();
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
        self.triage_mut().clear_selection();
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
            .ids_matching(&self.mailbox.location_query(&Location::AllInboxes), &self.local_now())
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

    /// `g i`: the current account's Inbox, or All Inboxes when already there.
    pub(super) fn go_inbox(&mut self, cx: &mut Context<Self>) {
        let location = match self.location() {
            Some(Location::AllInboxes) | None => Location::AllInboxes,
            _ => match self.nav_account() {
                Some(account) => Location::Inbox(account),
                None => return,
            },
        };
        self.show_location(location, false, cx);
    }

    /// `g s` / `g t` / `g a` / `g d`: the current account's location.
    fn go_account_location(
        &mut self,
        location: fn(AccountId) -> Location,
        cx: &mut Context<Self>,
    ) {
        if let Some(account) = self.nav_account() {
            self.show_location(location(account), false, cx);
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

    /// Messages a classifier-label action applies to: the one its menu row names, else the
    /// cursor/selection.
    pub(super) fn label_targets(&self, message: Option<MessageId>) -> Vec<MessageId> {
        message.map(|id| vec![id]).unwrap_or_else(|| self.target_ids())
    }

    /// `SetKind`: set (or clear) the Kind label by hand.
    pub(super) fn set_kind_action(&mut self, message: Option<MessageId>, kind: Option<Kind>, cx: &mut Context<Self>) {
        let ids = self.label_targets(message);
        self.set_label(&ids, QuestionKey::Kind, kind.map(Tag::Kind), cx);
    }

    /// `SetUrgency`: set the Urgency label by hand. Level 1 ("None") clears it.
    pub(super) fn set_urgency_action(&mut self, message: Option<MessageId>, level: u8, cx: &mut Context<Self>) {
        let ids = self.label_targets(message);
        self.set_label(&ids, QuestionKey::Urgency, Tag::for_urgency(level.clamp(1, 5)), cx);
    }

    /// `SetSpam`: mark (or unmark) the spam flag by hand. Spam never moves the message.
    pub(super) fn set_spam_action(&mut self, message: Option<MessageId>, spam: bool, cx: &mut Context<Self>) {
        let ids = self.label_targets(message);
        self.set_label(&ids, QuestionKey::Spam, spam.then_some(Tag::PossibleSpam), cx);
    }

    /// `ResetLabel`: drop the hand-set label and put the classifier's own value back.
    pub(super) fn reset_label_action(&mut self, message: Option<MessageId>, key: QuestionKey, cx: &mut Context<Self>) {
        let ids = self.label_targets(message);
        let mut changed = false;
        for id in ids {
            let tag = self.classifier_label(id, key);
            changed |= self.mailbox.reset_label(id, key, tag);
        }
        if changed {
            cx.notify();
        }
    }

    /// Set `key`'s label on `ids` as the user's own: it sticks until it is reset.
    fn set_label(&mut self, ids: &[MessageId], key: QuestionKey, tag: Option<Tag>, cx: &mut Context<Self>) {
        let mut changed = false;
        for id in ids {
            changed |= self.mailbox.set_label(*id, key, tag.clone());
        }
        if changed {
            cx.notify();
        }
    }

    /// Close the Triage tab, whether the session is running or only its summary is up. The
    /// dock removes the tab's group on its next sync; the user's tabs were never touched.
    pub(super) fn end_session(&mut self) {
        self.session = None;
        self.session_end = None;
        self.session_view = None;
    }

    pub(super) fn start_session(&mut self, cx: &mut Context<Self>) {
        let query = self.mailbox.location_query(&Location::AllInboxes);
        let ids = self.mailbox.ids_matching(&query, &self.local_now());
        if ids.is_empty() {
            return;
        }
        // The session's list is its own: the user's tabs, cursors and reader stay as they are.
        self.session_view = Some(ListView::new(query, Location::AllInboxes));
        self.session_end = None;
        self.session = Some(Session {
            ids,
            index: 0,
            handled: 0,
            started: self.now(),
        });
        cx.notify();
    }

    /// Re-route cached answers over the visible Inbox and, when automatic classification is on,
    /// queue the messages with due questions. Returns `(auto-applied, queued for review)`,
    /// counted from the cached answers only; anything queued is judged in the background and
    /// applied when its answers land. With automatic classification off, only cached answers are
    /// re-routed (nothing goes to Jev).
    pub(super) fn classify_visible(&mut self, cx: &mut Context<Self>) -> (usize, usize) {
        if self.judge.is_none() {
            // No key: classification is off, but the status bar must say so.
            self.refresh_judge_status();
            return (0, 0);
        }
        let inbox = self.mailbox.location_query(&Location::AllInboxes);
        let ids: Vec<MessageId> = self
            .mailbox
            .ids_matching(&inbox, &self.local_now())
            .into_iter()
            .filter(|id| self.mailbox.pending(*id).is_empty())
            .collect();
        let (mut auto, mut review) = (0, 0);
        for id in ids {
            // Re-route whatever answers are cached, whether or not new requests are allowed.
            if let Some(stored) = self.cached_answers(id).map(|s| s.to_vec())
                && !stored.is_empty()
            {
                let (a, r) = self.apply_answers(id, &stored);
                auto += a;
                review += r;
            }
            if self.auto_classify() && self.needs_questions(id) {
                self.enqueue_judge(JudgeJob::Message { id, force: false });
            }
        }
        self.refresh_judge_status();
        self.drain_judge_queue(cx);
        (auto, review)
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
