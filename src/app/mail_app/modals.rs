use super::*;

impl MailApp {
    /// Host `view` in a top-anchored kit dialog `width(viewport_width)` wide. Escape and
    /// backdrop clicks close it through the dialog itself and land in
    /// [`Self::dialog_dismissed`]; the owner closes it programmatically through
    /// [`Self::close_modals`].
    pub(super) fn host_in_dialog<V: Render>(
        &mut self,
        view: Entity<V>,
        width: fn(f32) -> f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let app = cx.weak_entity();
        window.open_dialog(cx, move |dialog, window, _| {
            let (view, app) = (view.clone(), app.clone());
            dialog
                .close_button(false)
                // `enter` belongs to the hosted view (it propagates out of an input); the view
                // closes the dialog itself once it has acted on it.
                .on_ok(|_, _, _| false)
                .p_0()
                .w(px(width(f32::from(window.viewport_size().width))))
                .on_close(move |_, _, cx| {
                    app.update(cx, |this, cx| this.dialog_dismissed(cx)).ok();
                })
                .content(move |content, _, _| content.child(view.clone()))
        });
    }

    /// The hosting dialog closed itself (escape, backdrop): forget its view.
    fn dialog_dismissed(&mut self, cx: &mut Context<Self>) {
        self.palette = None;
        self.folder_picker = None;
        self.snooze = None;
        self.settings = None;
        self.rules_panel = None;
        self.dialog = None;
        self.help = None;
        self._modal_sub = None;
        cx.notify();
    }

    pub(super) fn open_palette(&mut self, prefill: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        let palette = cx.new(|cx| CommandPalette::new(prefill.unwrap_or(""), window, cx));
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
        self.palette = Some(palette.clone());
        self.host_in_dialog(palette.clone(), |vw| 480f32.min(vw * 0.9), window, cx);
        palette.update(cx, |p, cx| p.focus(window, cx));
        cx.notify();
    }

    pub(super) fn start_search(&mut self, raw: &str, cx: &mut Context<Self>) {
        let q = raw.trim().trim_start_matches('/').trim().to_string();
        self.end_session();
        self.mode = ListMode::Search(q);
        self.alt_cursor = 0;
        cx.notify();
    }

    pub(super) fn open_compose(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(msg) = self.cursor_id().and_then(|id| self.mailbox.get(id)).cloned() else {
            return;
        };
        self.open_compose_for(&msg, None, window, cx);
    }

    /// Reply to message `id` (the reply buttons on each message of the reader).
    pub(super) fn reply_to(&mut self, id: MessageId, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal_open() {
            return;
        }
        if let Some(msg) = self.mailbox.get(id).cloned() {
            self.open_compose_for(&msg, None, window, cx);
        }
    }

    pub(super) fn open_compose_for(
        &mut self,
        msg: &Message,
        body: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pin_message(msg.id);
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
                        let expects_reply = crate::judge::expects_reply(&StubJudge, body);
                        this.mailbox.send_reply_at(*in_reply_to, body.clone(), expects_reply, now);
                        this.close_modals(window, cx);
                        this.open_post_send_dialog(*in_reply_to, window, cx);
                        this.show_toast(
                            "Reply queued · u to undo send".into(),
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

    pub(super) fn open_snooze(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_snooze_for(None, window, cx);
    }

    /// Snooze picker for `ids` (sender-wide flows), or for the current targets.
    pub(super) fn open_snooze_for(
        &mut self,
        ids: Option<Vec<MessageId>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let quick = self.in_session() && ids.is_none();
        // Captured now: a message menu's target only lives for its own dispatch.
        let ids = ids.unwrap_or_else(|| self.target_ids());
        if quick {
            let now = self.now();
            if let Some((_, until)) = self.mailbox.snooze_presets(now).into_iter().next() {
                self.mailbox.snooze(&ids, until, now);
                self.triage.clear_selection();
                self.session_advance();
            }
            cx.notify();
            return;
        }
        if ids.is_empty() {
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
            move |this, _, event: &SnoozeEvent, window, cx| {
                let SnoozeEvent::Pick(until) = event;
                let until = *until;
                this.close_modals(window, cx);
                let ids = &ids;
                let now = this.now();
                let n = this.mailbox.snooze(ids, until, now);
                this.triage.clear_selection();
                this.show_toast(
                    format!("Snoozed {n} until {} · u to undo", format_when(until)),
                    window,
                    cx,
                );
                this.scroll_to_cursor();
            },
        ));
        self.snooze = Some(picker.clone());
        self.host_in_dialog(picker.clone(), |vw| 320f32.min(vw * 0.9), window, cx);
        window.focus(&picker.focus_handle(cx), cx);
        cx.notify();
    }

    pub(super) fn toggle_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings.is_some() {
            self.close_modals(window, cx);
            return;
        }
        if self.modal_open() {
            return;
        }
        let group = self.group_threads;
        let preview = self.preview_lines;
        let panel = cx.new(|cx| {
            SettingsPanel::new(
                self.policy.clone(),
                self.summaries_enabled,
                group,
                preview,
                self.panes.orientation(),
                window,
                cx,
            )
            .tab_avatars(self.tab_avatars)
            .mailbox_state(
                self.mailbox.blocked(),
                self.mailbox.unsubscribed().to_vec(),
                self.mailbox.follow_up_timeout(),
            )
        });
        self._modal_sub = Some(cx.subscribe_in(
            &panel,
            window,
            |this, _, event: &SettingsEvent, window, cx| match event {
                SettingsEvent::Changed(policy, summaries) => {
                    this.policy = policy.clone();
                    this.summaries_enabled = *summaries;
                    cx.notify();
                }
                SettingsEvent::TabAvatars(on) => {
                    this.tab_avatars = *on;
                    cx.notify();
                }
                SettingsEvent::Grouping(on) => {
                    this.set_grouping(*on);
                    cx.notify();
                }
                SettingsEvent::PreviewLines(n) => {
                    this.preview_lines = (*n).min(5);
                    cx.notify();
                }
                SettingsEvent::PaneLayout(orientation) => {
                    if this.panes.orientation() != *orientation {
                        this.set_pane_layout(*orientation, window, cx);
                    }
                }
                SettingsEvent::FollowUp(timeout) => {
                    this.mailbox.set_follow_up_timeout(*timeout);
                    cx.notify();
                }
                SettingsEvent::Unblock(email) => {
                    this.mailbox.unblock_sender(email);
                    cx.notify();
                }
                SettingsEvent::Close => this.close_modals(window, cx),
            },
        ));
        self.settings = Some(panel.clone());
        self.host_in_dialog(panel.clone(), |vw| 760f32.min(vw * 0.9), window, cx);
        window.focus(&panel.focus_handle(cx), cx);
        cx.notify();
    }

    pub(super) fn toggle_rules(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        self.rules_panel = Some(panel.clone());
        self.host_in_dialog(panel.clone(), |vw| 420f32.min(vw * 0.9), window, cx);
        window.focus(&panel.focus_handle(cx), cx);
        cx.notify();
    }

    pub(super) fn mute_thread(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(thread) = self.cursor_id().and_then(|id| self.mailbox.get(id)).map(|m| m.thread_id)
        else {
            return;
        };
        self.mailbox.mute_thread(thread);
        self.show_toast("Thread muted · u to undo".into(), window, cx);
        self.session_advance();
        cx.notify();
    }

    pub(super) fn unsubscribe(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((name, email)) = self.cursor_sender() else {
            return;
        };
        self.open_block_dialog(name, email, true, window, cx);
    }

    /// `a` allows the sender outright; `b` asks what to do with their Inbox mail first.
    pub(super) fn screen_sender(&mut self, allow: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some((name, email)) = self.cursor_sender() else {
            return;
        };
        if !allow {
            self.open_block_dialog(name, email, false, window, cx);
            return;
        }
        if self.mailbox.allow_sender(&email) {
            self.show_toast(format!("Allowed {email} · u to undo"), window, cx);
        }
        self.move_cursor(0);
        cx.notify();
    }

    pub(super) fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal_open() {
            self.close_modals(window, cx);
        } else if matches!(self.mode, ListMode::Search(_)) {
            self.mode = ListMode::State;
        } else if let Some(s) = self.session.take() {
            if s.handled > 0 {
                self.session_end = Some((s.handled, self.now() - s.started));
            }
        } else if self.session_end.is_some() {
            self.end_session();
        } else {
            self.triage.clear_selection();
        }
        cx.notify();
    }
}

// ------------------------------------------------------------------ Choice dialogs

/// What a folder picker does with the folder it returns.
#[derive(Clone)]
pub(super) enum FileAction {
    /// File these messages (`f`, the reader bar).
    Ids(Vec<MessageId>),
    /// File every message from this sender in the given state (`shift-f`).
    Sender(String, TriageState),
    /// Post-send: file the original and its sent replies.
    AfterReply(MessageId),
    /// Block (`b`) or unsubscribe (`shift-u`), then move that sender's Inbox mail.
    BlockSender { email: String, unsubscribe: bool },
}

/// What the folder picker returned, before the folder exists.
enum Pick {
    File(FolderId),
    Create(String),
}

/// A sender-wide action's scope: the cursor's sender and their messages in the
/// cursor message's state.
struct SenderScope {
    name: String,
    email: String,
    ids: Vec<MessageId>,
    from: TriageState,
    /// Where those messages live, for the confirm title ("Inbox", "Archive", a folder).
    place: String,
}

impl SenderScope {
    fn title(&self, verb: &str) -> String {
        let n = self.ids.len();
        let noun = if n == 1 { "message" } else { "messages" };
        format!("{verb} all {n} {noun} from {} in {}?", self.name, self.place)
    }
}

/// Title, message, options and default option of a choice dialog.
struct DialogSpec {
    title: String,
    message: String,
    options: Vec<DialogOption>,
    default: usize,
}

impl DialogSpec {
    fn new(
        title: impl Into<String>,
        message: impl Into<String>,
        options: Vec<DialogOption>,
        default: usize,
    ) -> Self {
        Self {
            title: title.into(),
            message: message.into(),
            options,
            default,
        }
    }
}

impl MailApp {
    /// The sender of the message the actions target.
    pub(super) fn cursor_sender(&self) -> Option<(String, String)> {
        self.cursor_id()
            .and_then(|id| self.mailbox.get(id))
            .map(|m| (m.from_name.clone(), m.from_email.clone()))
    }

    /// The cursor's sender plus their messages in the cursor message's state:
    /// the scope of a sender-wide action. `None` when there is no cursor.
    fn sender_scope(&self) -> Option<SenderScope> {
        let msg = self.cursor_id().and_then(|id| self.mailbox.get(id))?;
        let from = msg.state;
        let place = match from {
            TriageState::Inbox => "Inbox".to_owned(),
            TriageState::Snoozed => "Snoozed".to_owned(),
            TriageState::Archived => "Archive".to_owned(),
            TriageState::Deleted => "Trash".to_owned(),
            TriageState::Filed(id) => self
                .mailbox
                .folder(id)
                .map_or_else(|| "this folder".to_owned(), |f| f.name.clone()),
        };
        Some(SenderScope {
            name: msg.from_name.clone(),
            email: msg.from_email.clone(),
            ids: self.mailbox.sender_ids_in(&msg.from_email, from),
            from,
            place,
        })
    }

    /// One account's folders in tree order, children indented under their parent.
    fn folder_options(&self, account: &str) -> Vec<FolderOption> {
        fn walk(
            folders: &[&Folder],
            parent: Option<FolderId>,
            depth: usize,
            out: &mut Vec<FolderOption>,
        ) {
            for folder in folders.iter().filter(|f| f.parent == parent) {
                out.push(FolderOption::new(
                    folder.id,
                    folder.name.clone(),
                    format!("{}{}", "  ".repeat(depth), folder.name),
                ));
                walk(folders, Some(folder.id), depth + 1, out);
            }
        }
        let folders = self.mailbox.folders(account);
        let mut out = Vec::new();
        walk(&folders, None, 0, &mut out);
        out
    }

    /// Open a choice dialog; `on_choose` runs with the dialog already closed, so it may
    /// open the next step (a folder picker, say).
    fn open_dialog(
        &mut self,
        spec: DialogSpec,
        on_choose: impl Fn(&mut MailApp, usize, &mut Window, &mut Context<MailApp>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let DialogSpec {
            title,
            message,
            options,
            default,
        } = spec;
        let dialog = cx.new(|cx| ChoiceDialog::new(title, message, options, default, cx));
        self._modal_sub = Some(cx.subscribe_in(
            &dialog,
            window,
            move |this, _, event: &DialogEvent, window, cx| {
                let DialogEvent::Choose(ix) = event;
                this.close_modals(window, cx);
                on_choose(this, *ix, window, cx);
                cx.notify();
            },
        ));
        self.dialog = Some(dialog.clone());
        self.host_in_dialog(dialog.clone(), |vw| 420f32.min(vw * 0.9), window, cx);
        window.focus(&dialog.focus_handle(cx), cx);
        cx.notify();
    }

    /// `f` and the reader's File button: file whatever the list targets.
    pub(super) fn open_file_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ids = self.target_ids();
        if ids.is_empty() {
            return;
        }
        self.open_folder_picker(FileAction::Ids(ids), window, cx);
    }

    /// The account whose folders `action` files into: that of the messages it moves, not the
    /// cursor's (a message menu can act on a message the cursor is not on).
    fn action_account(&self, action: &FileAction) -> Option<String> {
        let first = match action {
            FileAction::Ids(ids) => ids.first().copied(),
            FileAction::AfterReply(original) => Some(*original),
            FileAction::Sender(email, from) => {
                self.mailbox.sender_ids_in(email, *from).first().copied()
            }
            FileAction::BlockSender { email, .. } => self
                .mailbox
                .messages()
                .iter()
                .find(|m| m.from_email == *email)
                .map(|m| m.id),
        };
        first.and_then(|id| self.mailbox.get(id)).map(|m| m.account.clone())
    }

    /// Folder picker for the account of the messages `action` files.
    fn open_folder_picker(
        &mut self,
        action: FileAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(account) = self.action_account(&action) else {
            return;
        };
        let title = match &action {
            FileAction::Ids(ids) if ids.len() == 1 => "File to folder:".to_owned(),
            FileAction::Ids(ids) => format!("File {} messages to:", ids.len()),
            FileAction::Sender(email, _) => format!("File all mail from {email} to:"),
            FileAction::AfterReply(_) => "File the original and the reply to:".to_owned(),
            FileAction::BlockSender { email, .. } => format!("File all mail from {email} to:"),
        };
        let options = self.folder_options(&account);
        let picker = cx.new(|cx| FolderPicker::new(title, options, window, cx));
        self._modal_sub = Some(cx.subscribe_in(
            &picker,
            window,
            move |this, _, event: &FolderPickerEvent, window, cx| {
                let pick = match event {
                    FolderPickerEvent::File(id) => Pick::File(*id),
                    FolderPickerEvent::Create(name) => Pick::Create(name.clone()),
                };
                this.close_modals(window, cx);
                this.apply_pick(&action, &account, pick, window, cx);
                cx.notify();
            },
        ));
        self.folder_picker = Some(picker.clone());
        self.host_in_dialog(picker.clone(), |vw| 420f32.min(vw * 0.9), window, cx);
        picker.update(cx, |p, cx| p.focus(window, cx));
        cx.notify();
    }

    /// The chosen folder, creating it when the picker offered a new name.
    /// The creation and the move are folded into one undo step by `grouped`.
    fn apply_pick(
        &mut self,
        action: &FileAction,
        account: &str,
        pick: Pick,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        fn folder_for(mb: &mut Mailbox, account: &str, pick: &Pick) -> FolderId {
            match pick {
                Pick::File(id) => *id,
                Pick::Create(name) => mb.create_folder(account, name, None),
            }
        }
        let n = match action {
            FileAction::Ids(ids) => match &pick {
                Pick::File(id) => self.mailbox.set_state(ids, TriageState::Filed(*id)),
                Pick::Create(name) => {
                    self.mailbox.create_folder_and_file(account, name, None, ids).1
                }
            },
            FileAction::Sender(email, from) => self.mailbox.grouped(|mb| {
                let state = TriageState::Filed(folder_for(mb, account, &pick));
                mb.set_state_for_sender(email, *from, state)
            }),
            FileAction::AfterReply(original) => self.mailbox.grouped(|mb| {
                let state = TriageState::Filed(folder_for(mb, account, &pick));
                mb.file_after_reply(*original, state)
            }),
            FileAction::BlockSender { email, unsubscribe } => self.mailbox.grouped(|mb| {
                let state = TriageState::Filed(folder_for(mb, account, &pick));
                if *unsubscribe {
                    mb.unsubscribe(email, Some(state))
                } else {
                    mb.block_sender(email, Some(state))
                }
            }),
        };
        self.triage.clear_selection();
        let text = match action {
            FileAction::BlockSender {
                unsubscribe: true, ..
            } => format!("Unsubscribed and filed {n} · u to undo"),
            FileAction::BlockSender { .. } => format!("Blocked and filed {n} · u to undo"),
            _ => format!("Filed {n} · u to undo"),
        };
        self.show_toast(text, window, cx);
        self.session_advance();
        self.scroll_to_cursor();
        cx.notify();
    }

    /// Post-send: the original and the reply land wherever the dialog says.
    fn open_post_send_dialog(
        &mut self,
        original: MessageId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let options = vec![
            DialogOption::new("1", "Archive", "Put it away"),
            DialogOption::new("2", "File…", "Pick a folder"),
            DialogOption::new("3", "Delete", "Move it to Trash"),
            DialogOption::new("4", "Keep in Inbox", "Change nothing"),
        ];
        self.open_dialog(
            DialogSpec::new(
                "Reply sent",
                "File the original and the reply:",
                options,
                0,
            ),
            move |this, ix, window, cx| match ix {
                0 => this.file_after_reply(original, TriageState::Archived, window, cx),
                1 => this.open_folder_picker(FileAction::AfterReply(original), window, cx),
                2 => this.file_after_reply(original, TriageState::Deleted, window, cx),
                _ => {}
            },
            window,
            cx,
        );
    }

    fn file_after_reply(
        &mut self,
        original: MessageId,
        state: TriageState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let n = self.mailbox.file_after_reply(original, state);
        let verb = match state {
            TriageState::Deleted => "Deleted",
            TriageState::Archived => "Archived",
            _ => "Filed",
        };
        self.show_toast(format!("{verb} {n} · u to undo"), window, cx);
        self.scroll_to_cursor();
        cx.notify();
    }

    /// `b` / `shift-u`: block or unsubscribe, and say what happens to their Inbox mail.
    fn open_block_dialog(
        &mut self,
        name: String,
        email: String,
        unsubscribe: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let title = if unsubscribe {
            format!("Unsubscribe from {name}?")
        } else {
            format!("Block {name}?")
        };
        let message = format!("Move {email}'s Inbox mail to:");
        let options = vec![
            DialogOption::new("1", "Delete", "Move their Inbox mail to Trash"),
            DialogOption::new("2", "Archive", "Put their Inbox mail away"),
            DialogOption::new("3", "File…", "Pick a folder for it"),
            DialogOption::new("4", "Leave", "Keep it in the Inbox"),
        ];
        // Leaving is the default: an accidental `b` + `enter` must not trash mail.
        self.open_dialog(
            DialogSpec::new(title, message, options, 3),
            move |this, ix, window, cx| {
                if ix == 2 {
                    this.open_folder_picker(
                        FileAction::BlockSender {
                            email: email.clone(),
                            unsubscribe,
                        },
                        window,
                        cx,
                    );
                    return;
                }
                let state = match ix {
                    0 => Some(TriageState::Deleted),
                    1 => Some(TriageState::Archived),
                    _ => None,
                };
                this.block_or_unsubscribe(&email, unsubscribe, state, window, cx);
            },
            window,
            cx,
        );
    }

    fn block_or_unsubscribe(
        &mut self,
        email: &str,
        unsubscribe: bool,
        state: Option<TriageState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let verb = if unsubscribe { "Unsubscribed from" } else { "Blocked" };
        let already = if unsubscribe {
            self.mailbox
                .unsubscribed()
                .iter()
                .any(|u| u.eq_ignore_ascii_case(email))
        } else {
            self.mailbox
                .blocked()
                .iter()
                .any(|b| b.eq_ignore_ascii_case(email))
        };
        if already {
            self.show_toast(format!("Already {verb} {email}"), window, cx);
            return;
        }
        let n = if unsubscribe {
            self.mailbox.unsubscribe(email, state)
        } else {
            self.mailbox.block_sender(email, state)
        };
        let tail = match state {
            Some(_) => format!(" · moved {n} · u to undo"),
            None => " · u to undo".to_owned(),
        };
        self.show_toast(format!("{verb} {email}{tail}"), window, cx);
        self.session_advance();
        self.scroll_to_cursor();
        cx.notify();
    }

    /// `!`: block and delete, or just delete.
    pub(super) fn open_spam_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ids = self.target_ids();
        if ids.is_empty() {
            return;
        }
        let name = self
            .cursor_sender()
            .map(|(name, _)| name)
            .unwrap_or_default();
        let title = format!(
            "Mark {} message{} from {name} as spam?",
            ids.len(),
            if ids.len() == 1 { "" } else { "s" }
        );
        let options = vec![
            DialogOption::new("1", "Block & Delete", "Trash it and block the sender"),
            DialogOption::new("2", "Delete", "Move it to Trash"),
        ];
        self.open_dialog(
            DialogSpec::new(title, String::new(), options, 0),
            move |this, ix, window, cx| this.spam(&ids, ix == 0, window, cx),
            window,
            cx,
        );
    }

    /// Mark `ids` as spam (optionally blocking their senders).
    pub(super) fn spam(
        &mut self,
        ids: &[MessageId],
        block: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let n = self.mailbox.mark_spam(ids, block);
        self.triage.clear_selection();
        let text = if block {
            format!("Marked {n} as spam and blocked the sender · u to undo")
        } else {
            format!("Marked {n} as spam · u to undo")
        };
        self.show_toast(text, window, cx);
        self.session_advance();
        self.scroll_to_cursor();
        cx.notify();
    }

    /// Sender-wide archive/delete/inbox: confirm first.
    pub(super) fn confirm_sender(
        &mut self,
        state: TriageState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(scope) = self.sender_scope() else {
            return;
        };
        if scope.ids.is_empty() || scope.from == state {
            return;
        }
        let verb = match state {
            TriageState::Archived => "Archive",
            TriageState::Deleted => "Delete",
            TriageState::Inbox => "Move to Inbox",
            _ => "Move",
        };
        let title = scope.title(verb);
        let SenderScope { name, email, from, .. } = scope;
        self.confirm(
            title,
            move |this, window, cx| this.mark_sender(&email, &name, from, state, window, cx),
            window,
            cx,
        );
    }

    /// Sender-wide file: confirm, then pick a folder.
    pub(super) fn confirm_sender_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(scope) = self.sender_scope() else {
            return;
        };
        if scope.ids.is_empty() {
            return;
        }
        let title = scope.title("File");
        let SenderScope { email, from, .. } = scope;
        self.confirm(
            title,
            move |this, window, cx| {
                this.open_folder_picker(FileAction::Sender(email.clone(), from), window, cx)
            },
            window,
            cx,
        );
    }

    /// Sender-wide snooze: confirm, then pick a return time.
    pub(super) fn confirm_sender_snooze(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(scope) = self.sender_scope() else {
            return;
        };
        if scope.ids.is_empty() || scope.from == TriageState::Snoozed {
            return;
        }
        let title = scope.title("Snooze");
        let ids = scope.ids;
        self.confirm(
            title,
            move |this, window, cx| this.open_snooze_for(Some(ids.clone()), window, cx),
            window,
            cx,
        );
    }

    /// Confirm/Cancel dialog; `on_confirm` runs when Confirm is chosen.
    fn confirm(
        &mut self,
        title: String,
        on_confirm: impl Fn(&mut MailApp, &mut Window, &mut Context<MailApp>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let options = vec![
            DialogOption::new("1", "Confirm", ""),
            DialogOption::new("2", "Cancel", ""),
        ];
        self.open_dialog(
            DialogSpec::new(title, String::new(), options, 0),
            move |this, ix, window, cx| {
                if ix == 0 {
                    on_confirm(this, window, cx);
                }
            },
            window,
            cx,
        );
    }
}
