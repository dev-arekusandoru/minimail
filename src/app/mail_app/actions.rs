use super::*;
use gpui_kit::component::button::{Button, ButtonVariants as _};

impl MailApp {
    /// Show `text` as a kit notification at the bottom of the window. A new toast replaces the
    /// previous one. `self.toast` mirrors the visible message because the kit keeps its
    /// notification text private.
    pub(super) fn show_toast(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.push_toast(text, false, window, cx);
    }

    /// Same, with an Undo action: the one undo step `u` performs.
    pub(super) fn show_undo_toast(
        &mut self,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.push_toast(text, true, window, cx);
    }

    /// One row: status icon, message, then the Undo action the kit lays beside it.
    fn push_toast(
        &mut self,
        text: String,
        undo: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        if undo {
            note = note
                .action(|_, _, cx| {
                    Button::new("toast-undo")
                        .label("Undo")
                        .ghost()
                        .child(crate::app::ui::shortcut("u"))
                        .on_click(cx.listener(|note, _, window, cx| {
                            window.dispatch_action(Box::new(Undo), cx);
                            note.dismiss(window, cx);
                        }))
                })
                // The action slot turns autohide off; the toast still goes on its own.
                .autohide(true);
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
        self.new_folder = None;
        self.folder_picker = None;
        self._modal_sub = None;
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    /// After the current message left the queue: count it as `outcome` and move on. The
    /// summary takes over when the queue is done.
    pub(super) fn session_advance(&mut self, outcome: Outcome) {
        // A menu acting on a message other than the session's current one handles nothing
        // of the session.
        if let (Some(ids), Some(current)) = (&self.menu_target, self.session_current())
            && !ids.contains(&current)
        {
            return;
        }
        let Some(s) = self.session.as_mut() else { return };
        s.run.handle(outcome, std::mem::take(&mut s.taking));
        if s.run.is_finished() {
            self.end_run();
        } else {
            self.refresh_suggestion();
        }
    }

    /// Recompute the suggestion for the current message.
    pub(super) fn refresh_suggestion(&mut self) {
        let suggestion = self.session.as_ref().and_then(|s| {
            let id = s.run.current().filter(|_| s.run.options.suggestions)?;
            crate::triage_suggest::suggest(&self.mailbox, id)
        });
        if let Some(s) = self.session.as_mut() {
            s.suggestion = suggestion;
        }
    }

    /// Stop the running session: its Summary when anything was handled, else straight back to
    /// the user's own views.
    pub(super) fn end_run(&mut self) {
        let Some(s) = self.session.take() else { return };
        if s.run.handled() == 0 {
            return self.end_session();
        }
        let remaining = self.mailbox.ids_matching(&self.mailbox.location_query(&s.run.options.location), &self.local_now()).len();
        self.session_summary = Some((s.run.summary(self.now(), remaining), s.run.options));
    }

    /// `space`: defer the current message to the end of the queue. Not a mailbox change, so
    /// no undo step.
    pub fn skip_current(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(s) = self.session.as_mut() else { return };
        s.run.skip();
        self.refresh_suggestion();
        self.scroll_to_cursor();
        cx.notify();
    }

    /// `enter` while Triage mode is open: start from the Start screen, take the suggestion in a
    /// running session, keep going from the Summary. `false` when Triage mode is not open.
    pub(super) fn triage_enter(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.session_setup.is_some() {
            self.start_session_from_setup(cx);
        } else if self.session_summary.is_some() {
            self.keep_going(cx);
        } else if self.session.is_some() {
            self.accept_session_suggestion(window, cx);
        } else {
            return false;
        }
        true
    }

    /// `enter`: take the suggested action on the current message.
    pub fn accept_session_suggestion(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(suggestion) = self.session_suggestion().cloned() else { return };
        let Some(id) = self.session_current() else { return };
        if let Some(s) = self.session.as_mut() {
            s.taking = true;
        }
        match suggestion.action {
            SuggestedAction::Archive => self.file_ids(&[id], TriageState::Archived, window, cx),
            SuggestedAction::File(folder) => self.file_ids(&[id], TriageState::Filed(folder), window, cx),
            SuggestedAction::Delete => self.mark(TriageState::Deleted, window, cx),
        }
        if let Some(s) = self.session.as_mut() {
            s.taking = false;
        }
    }

    pub(super) fn mark(&mut self, state: TriageState, window: &mut Window, cx: &mut Context<Self>) {
        let ids = self.target_ids();
        // Deleting a message that carries the spam flag accepts the suggestion.
        let spam_sender = (state == TriageState::Deleted)
            .then(|| ids.iter().filter_map(|id| self.mailbox.get(*id)).find(|m| {
                !m.outgoing
                    && m.state == TriageState::Inbox
                    && self.mailbox.tags(m.id).contains(&Tag::PossibleSpam)
            }))
            .flatten()
            .map(|m| m.from_email.clone());
        self.file_ids(&ids, state, window, cx);
        if let Some(email) = spam_sender {
            self.offer_delete_and_always_spam(email, window, cx);
        }
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
                TriageState::Inbox => format!("Moved {n} to inbox"),
                s => format!("Marked {n} {}", s.label().to_lowercase()),
            };
            self.show_undo_toast(msg, window, cx);
        }
        self.session_advance(Outcome::of(state));
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
            let msg = format!("Marked {n} from {name} {}", state.label().to_lowercase());
            self.show_undo_toast(msg, window, cx);
        }
        if let Some(rule) = self.rules.record(&email, state) {
            self.pending_rule = Some(rule);
        }
        self.session_advance(Outcome::of(state));
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
    /// Clearing the flag offers to never ask about that sender again.
    pub(super) fn set_spam_action(
        &mut self,
        message: Option<MessageId>,
        spam: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ids = self.label_targets(message);
        let sender = ids.first().and_then(|id| self.mailbox.get(*id)).map(|m| m.from_email.clone());
        let before = self.mailbox.undo_depth();
        self.set_label(&ids, QuestionKey::Spam, spam.then_some(Tag::PossibleSpam), cx);
        if let Some(email) = sender.filter(|_| !spam && self.mailbox.undo_depth() > before) {
            self.offer_never_spam(email, window, cx);
        }
    }

    /// `SetNeedsReply`: set (or clear) the Needs reply tag by hand. It never moves the message.
    pub(super) fn set_needs_reply_action(&mut self, message: Option<MessageId>, needs_reply: bool, cx: &mut Context<Self>) {
        let ids = self.label_targets(message);
        self.set_label(&ids, QuestionKey::NeedsReply, needs_reply.then_some(Tag::NeedsReply), cx);
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

    /// Close Triage mode, whichever screen it is on; the user's own views were never touched.
    pub(super) fn end_session(&mut self) {
        self.session = None;
        self.session_setup = None;
        self.session_summary = None;
        self.session_view = None;
    }

    /// `t`: open the Start screen with the default options.
    pub(super) fn start_session(&mut self, cx: &mut Context<Self>) {
        if self.triage_open() {
            return;
        }
        self.open_setup(SessionOptions::default(), cx);
    }

    /// The Start screen over `options`. Its list view is the session's own from now on, so the
    /// user's tabs, cursors and reader stay as they are.
    fn open_setup(&mut self, options: SessionOptions, cx: &mut Context<Self>) {
        let query = self.mailbox.location_query(&options.location);
        self.session_view = Some(ListView::new(query, options.location.clone()));
        self.session_summary = None;
        self.session_setup = Some(SetupView::new(&self.mailbox, options, &self.local_now()));
        cx.notify();
    }

    fn change_setup(&mut self, edit: impl FnOnce(&mut SessionOptions), cx: &mut Context<Self>) {
        let Some(mut options) = self.session_setup.as_ref().map(|v| v.options.clone()) else { return };
        edit(&mut options);
        self.open_setup(options, cx);
    }

    pub fn setup_set_location(&mut self, location: Location, cx: &mut Context<Self>) {
        self.change_setup(|o| o.location = location, cx);
    }

    pub fn setup_set_filter(&mut self, filter: SessionFilter, cx: &mut Context<Self>) {
        self.change_setup(|o| o.filter = filter, cx);
    }

    pub fn setup_set_size(&mut self, size: SessionSize, cx: &mut Context<Self>) {
        self.change_setup(|o| o.size = size, cx);
    }

    pub fn setup_set_order(&mut self, order: SessionOrder, cx: &mut Context<Self>) {
        self.change_setup(|o| o.order = order, cx);
    }

    pub fn setup_set_suggestions(&mut self, on: bool, cx: &mut Context<Self>) {
        self.change_setup(|o| o.suggestions = on, cx);
    }

    /// `enter` on the Start screen: run the queue the options build. An empty queue stays put.
    pub fn start_session_from_setup(&mut self, cx: &mut Context<Self>) {
        let Some(options) = self.session_setup.as_ref().map(|v| v.options.clone()) else { return };
        self.begin_run(options, cx);
    }

    /// `esc` on the Start screen.
    pub fn cancel_setup(&mut self, cx: &mut Context<Self>) {
        self.end_session();
        cx.notify();
    }

    /// `esc` in a running session: its Summary when anything was handled, else back out.
    pub fn end_session_from_ui(&mut self, cx: &mut Context<Self>) {
        self.end_run();
        cx.notify();
    }

    /// `enter` on the Summary: a new session with the same options.
    pub fn keep_going(&mut self, cx: &mut Context<Self>) {
        let Some((_, options)) = self.session_summary.clone() else { return };
        self.begin_run(options, cx);
    }

    /// `esc` on the Summary.
    pub fn close_summary(&mut self, cx: &mut Context<Self>) {
        self.end_session();
        cx.notify();
    }

    fn begin_run(&mut self, options: SessionOptions, cx: &mut Context<Self>) {
        let ids = crate::triage_session::build_queue(&self.mailbox, &options, &self.local_now());
        if ids.is_empty() {
            return;
        }
        let query = self.mailbox.location_query(&options.location);
        self.session_view = Some(ListView::new(query, options.location.clone()));
        self.session_setup = None;
        self.session_summary = None;
        self.session = Some(Session { run: Run::new(options, ids, self.now()), suggestion: None, taking: false });
        self.refresh_suggestion();
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
            if self.auto_wanted(id) && self.needs_questions(id) {
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
        // An undone rename, delete or folder creation changes what tabs point at.
        let anchors = self.tab_anchors();
        if let Some(reply) = self.mailbox.recall_last(self.now()) {
            if let Some(msg) = self.mailbox.get(reply.in_reply_to).cloned() {
                self.close_modals(window, cx);
                self.open_compose_for(&msg, Some(reply.body), window, cx);
            }
            self.show_toast("Send recalled · editing draft".into(), window, cx);
        } else if self.mailbox.undo() {
            self.refresh_suggestion();
            self.follow_folders(anchors, window, cx);
            self.show_toast("Undone".into(), window, cx);
        }
        self.scroll_to_cursor();
        cx.notify();
    }
}
