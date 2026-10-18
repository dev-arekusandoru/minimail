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
                .on_close(move |_, window, cx| {
                    app.update(cx, |this, cx| this.dialog_dismissed(window, cx)).ok();
                })
                .content(move |content, _, _| content.child(view.clone()))
        });
    }

    /// The hosting dialog closed itself (escape, backdrop): forget its view and take focus
    /// back, as `close_modals` does, so keys work even when the opener (a menu) is gone.
    fn dialog_dismissed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette = None;
        self.new_folder = None;
        self.folder_picker = None;
        self.snooze = None;
        self.rules_panel = None;
        self.dialog = None;
        self.help = None;
        self._modal_sub = None;
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    pub(super) fn open_palette(&mut self, prefill: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        let targets = self.label_targets(None);
        // Rename and Delete folder act on the folder the list is showing: only offered then.
        let in_folder = self.target_folder(None).is_some();
        let commands = crate::app::actions::commands()
            .into_iter()
            .filter(|spec| {
                let action = (spec.action)();
                let any = action.as_any();
                (in_folder || !(any.is::<RenameFolder>() || any.is::<DeleteFolder>()))
                    && any.downcast_ref::<ResetLabel>().is_none_or(|reset| {
                        targets.iter().any(|id| self.mailbox.is_overridden(*id, reset.key))
                    })
            })
            .collect();
        let palette = cx.new(|cx| CommandPalette::new(prefill.unwrap_or(""), commands, window, cx));
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

    /// Run a palette query: parsed into the list's one query, on top of the folder being
    /// browsed (so searching from inside a folder keeps it, until the pill is removed).
    pub(super) fn start_search(&mut self, raw: &str, cx: &mut Context<Self>) {
        let mut query = self.implicit_query();
        query.overlay(&Query::parse(raw));
        self.apply_query(query, cx);
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

    /// Reply all to message `id`: sender + To + Cc, minus our own address.
    pub(super) fn reply_all_to(&mut self, id: MessageId, window: &mut Window, cx: &mut Context<Self>) {
        self.compose_kind_for(DraftKind::ReplyAll, id, window, cx);
    }

    /// Forward message `id` with the forwarded-message block prefilled and To empty.
    pub(super) fn forward(&mut self, id: MessageId, window: &mut Window, cx: &mut Context<Self>) {
        self.compose_kind_for(DraftKind::Forward, id, window, cx);
    }

    /// The keyboard/palette path: the same, on the cursor message.
    pub(super) fn open_compose_kind(&mut self, kind: DraftKind, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.cursor_id() else {
            return;
        };
        match kind {
            DraftKind::Reply => self.reply_to(id, window, cx),
            DraftKind::ReplyAll => self.reply_all_to(id, window, cx),
            DraftKind::Forward => self.forward(id, window, cx),
        }
    }

    fn compose_kind_for(&mut self, kind: DraftKind, id: MessageId, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal_open() {
            return;
        }
        if let Some(msg) = self.mailbox.get(id).cloned() {
            self.open_compose_kind_for(kind, &msg, None, window, cx);
        }
    }

    pub(super) fn open_compose_for(
        &mut self,
        msg: &Message,
        body: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_compose_kind_for(DraftKind::Reply, msg, body, window, cx);
    }

    fn open_compose_kind_for(
        &mut self,
        kind: DraftKind,
        msg: &Message,
        body: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.mailbox.account(&msg.account).is_some_and(|a| a.provider == crate::model::ProviderKind::Gmail) {
            self.show_toast("Sending from Gmail isn't supported yet".into(), window, cx);
            return;
        }
        self.pin_message(msg.id);
        let own = self.mailbox.account(&msg.account).map(|a| a.email.clone()).unwrap_or_default();
        let compose = cx.new(|cx| {
            let mut c = ComposeReply::new(kind, msg, &own, window, cx);
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
                    ComposeEvent::Send { kind: DraftKind::Forward, .. } => {
                        // Mock data: a forward leaves the message's triage and tags untouched.
                        this.close_modals(window, cx);
                        this.show_toast("Forward sent".into(), window, cx);
                    }
                    ComposeEvent::Send { in_reply_to, body, .. } => {
                        let now = this.now();
                        // The reply goes now; the classifier tags the thread *Awaiting Reply* if
                        // the answer says so, on the background worker (no key means no tag).
                        if let Some(seq) =
                            this.mailbox.queue_reply_at(*in_reply_to, body.clone(), false, now)
                        {
                            this.judge_reply(*in_reply_to, seq, now, body.clone(), cx);
                        }
                        this.close_modals(window, cx);
                        this.open_post_send_dialog(*in_reply_to, window, cx);
                        this.show_undo_toast("Reply queued".into(), window, cx);
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
            let now = self.local_now();
            if let Some((_, until)) = self.mailbox.snooze_presets(&now).into_iter().next() {
                self.mailbox.snooze(&ids, until, now.at());
                self.triage_mut().clear_selection();
                self.session_advance();
            }
            cx.notify();
            return;
        }
        if ids.is_empty() {
            return;
        }
        let now = self.local_now();
        let presets: Vec<(String, Timestamp)> = self
            .mailbox
            .snooze_presets(&now)
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
                let local = this.local_now();
                let n = this.mailbox.snooze(ids, until, local.at());
                this.triage_mut().clear_selection();
                this.show_undo_toast(
                    format!("Snoozed {n} until {}", format_when(until, &local)),
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

    pub(super) fn save_setting<T>(&self, setting: &crate::prefs::Setting<T>, value: T)
    where
        T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + Clone,
    {
        if let Some(store) = &self.preferences {
            setting.set(store.as_ref(), &crate::prefs::Scope::Global, value);
        }
    }
    pub(super) fn toggle_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_settings(crate::app::settings::ACCOUNTS_PAGE, window, cx);
    }

    /// Open Settings on the AI page, where the Jev key lives (the status bar's no-key prompt).
    pub(super) fn open_settings_ai(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_settings(crate::app::settings::AI_PAGE, window, cx);
    }

    /// Apply an account's picked icon and color, and keep Gmail accounts' choice in the cache.
    fn set_account_style(&mut self, id: &str, icon: &str, color: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !self.mailbox.set_account_style(id, icon, color) {
            return;
        }
        self.persist_account(id, window, cx);
        cx.notify();
    }

    /// Save a Gmail account's icon, color and nickname to the cache.
    fn persist_account(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let (Some(cache), Some(account)) = (&self.cache, self.mailbox.account(id))
            && account.provider == crate::model::ProviderKind::Gmail
            && let Err(e) = cache.upsert_account(account)
        {
            self.show_toast(format!("Could not save the account: {e}"), window, cx);
        }
    }

    /// Rows for the settings Accounts page.
    pub(super) fn account_rows(&self) -> Vec<AccountRow> {
        let now = self.now();
        self.mailbox
            .accounts()
            .iter()
            .map(|a| {
                let gmail = a.provider == crate::model::ProviderKind::Gmail;
                let (status, (sync, sync_problem)) = if gmail {
                    let status = self.account_sync_status(a);
                    (status.clone(), crate::sync_status::describe(&status, now))
                } else {
                    (crate::sync_status::SyncStatus::Synced(None), (String::new(), false))
                };
                AccountRow {
                    id: a.id.clone(),
                    name: a.name.clone(),
                    email: a.email.clone(),
                    color: a.color.clone(),
                    icon: crate::account_style::icon_key(a),
                    nickname: a.nickname.clone().unwrap_or_default(),
                    gmail,
                    sync,
                    sync_action: status.action(),
                    sync_problem,
                }
            })
            .collect()
    }

    /// Open Settings in its own native window on `page`, reusing and activating the existing
    /// window.
    pub(super) fn open_settings(&mut self, page: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(handle) = self.settings_window {
            if handle.update(cx, |_, window, _| window.activate_window()).is_ok() {
                return;
            }
            self.settings_window = None;
            self.settings = None;
            self._settings_sub = None;
        }
        if self.modal_open() {
            return;
        }
        let group = self.group_threads;
        let preview = self.preview_lines;
        let row_style = self.row_style;
        let rows = self.account_rows();
        let gmail_configured = crate::provider::gmail::ClientConfig::load().is_some();
        let policy = self.policy.clone();
        let summaries = self.summaries_enabled;
        let orientation = self.panes.orientation();
        let theme_mode = self.theme_mode;
        let light_theme = self.light_theme.clone();
        let dark_theme = self.dark_theme.clone();
        let tab_avatars = self.tab_avatars;
        let block_remote_images = self.block_remote_images;
        let blocked = self.mailbox.blocked();
        let spam_senders = self.mailbox.spam_senders().clone();
        let now = self.local_now();
        let follow_up = self.mailbox.follow_up_timeout();
        let (jev_base, jev_model) = self.jev_settings();
        let jev_has_key = self.has_jev_key();
        // The switches show the stored preference (both off by default), not the running
        // state, which a no-preferences session starts true.
        let scope = crate::prefs::Scope::Global;
        let (auto_classify, spam_cold_outreach) = match &self.preferences {
            Some(store) => (
                crate::app_settings::AUTO_CLASSIFY.get(store.as_ref(), &scope),
                crate::app_settings::SPAM_COLD_OUTREACH.get(store.as_ref(), &scope),
            ),
            None => (false, false),
        };
        let auto_max_age = match &self.preferences {
            Some(store) => crate::app_settings::AUTO_CLASSIFY_MAX_AGE_DAYS.get(store.as_ref(), &scope),
            None => self.auto_max_age_days,
        };
        let app = cx.weak_entity();
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(820.), px(760.)), cx)),
            window_min_size: Some(size(px(620.), px(460.))),
            // The panel draws the kit `TitleBar` itself, so the bar owns dragging, the
            // double click and the window controls.
            ..gpui_kit::component::TitleBar::window_options()
        };
        let Ok((handle, panel)) = gpui_kit::open_window(options, cx, move |settings_window, cx| {
            let app = app.clone();
            settings_window.on_window_should_close(cx, move |_, cx| {
                app.update(cx, |this, cx| {
                    this.settings = None;
                    this.settings_window = None;
                    this._settings_sub = None;
                    cx.notify();
                })
                .ok();
                true
            });
            let panel = cx.new(|cx| {
                SettingsPanel::new(policy, summaries, group, preview, orientation, settings_window, cx)
                    .tab_avatars(tab_avatars)
                    .row_style(row_style)
                    .block_remote_images(block_remote_images)
                    .mailbox_state(blocked, now, follow_up)
                    .spam_senders(spam_senders)
                    .theme_preferences(theme_mode, light_theme, dark_theme)
                    .accounts(rows, gmail_configured)
                    .classifier_switches(auto_classify, spam_cold_outreach)
                    .auto_max_age(auto_max_age)
                    .on_page(page)
                    .jev_connection(jev_base, jev_model, jev_has_key)
            });
            let focus = panel.read(cx).focus_handle(cx).clone();
            settings_window.focus(&focus, cx);
            panel
        }) else {
            return;
        };
        let panel_weak = panel.downgrade();
        self._settings_sub = Some(cx.subscribe_in(
            &panel,
            window,
            move |this, _, event: &SettingsEvent, window, cx| match event {
                SettingsEvent::ResetAll => {
                    if let Some(store) = &this.preferences {
                        let scope = crate::prefs::Scope::Global;
                        crate::app_settings::THEME_MODE.reset(store.as_ref(), &scope);
                        crate::app_settings::LIGHT_THEME.reset(store.as_ref(), &scope);
                        crate::app_settings::DARK_THEME.reset(store.as_ref(), &scope);
                        crate::app_settings::PANE_LAYOUT.reset(store.as_ref(), &scope);
                        crate::app_settings::TAB_AVATARS.reset(store.as_ref(), &scope);
                        crate::app_settings::BLOCK_REMOTE_IMAGES.reset(store.as_ref(), &scope);
                        crate::app_settings::GROUP_THREADS.reset(store.as_ref(), &scope);
                        crate::app_settings::PREVIEW_LINES.reset(store.as_ref(), &scope);
                        crate::app_settings::save_row_style(
                            store.as_ref(),
                            &scope,
                            crate::row_style::RowStyle::default(),
                        );
                        crate::app_settings::FOLLOW_UP_DAYS.reset(store.as_ref(), &scope);
                        crate::app_settings::SUMMARIES.reset(store.as_ref(), &scope);
                        crate::app_settings::MODE_SPAM.reset(store.as_ref(), &scope);
                        crate::app_settings::MODE_NEEDS_REPLY.reset(store.as_ref(), &scope);
                        crate::app_settings::MODE_EXPECTS_REPLY.reset(store.as_ref(), &scope);
                        crate::app_settings::MODE_URGENCY.reset(store.as_ref(), &scope);
                        crate::app_settings::MODE_KIND.reset(store.as_ref(), &scope);
                        crate::app_settings::MODE_FOLDER.reset(store.as_ref(), &scope);
                        crate::app_settings::AUTO_CLASSIFY.reset(store.as_ref(), &scope);
                        crate::app_settings::AUTO_CLASSIFY_MAX_AGE_DAYS.reset(store.as_ref(), &scope);
                        crate::app_settings::SPAM_COLD_OUTREACH.reset(store.as_ref(), &scope);
                        crate::app_settings::JEV_BASE_URL.reset(store.as_ref(), &scope);
                        crate::app_settings::JEV_MODEL.reset(store.as_ref(), &scope);
                    }
                    this.policy = JudgePolicy::default();
                    this.theme_mode = crate::theme::ThemeMode::System;
                    this.light_theme = crate::theme::names_for(cx, true)
                        .first()
                        .map(ToString::to_string)
                        .unwrap_or_default();
                    this.dark_theme = crate::app_settings::DEFAULT_DARK_THEME.to_owned();
                    cx.set_window_appearance(None);
                    this.summaries_enabled = false;
                    this.group_threads = false;
                    this.tab_avatars = true;
                    this.block_remote_images = false;
                    this.preview_lines = crate::preview::DEFAULT_LINES;
                    this.row_style = crate::row_style::RowStyle::default();
                    this.mailbox.set_follow_up_timeout(crate::model::DEFAULT_FOLLOW_UP_TIMEOUT);
                    this.panes.set_orientation(PaneLayout::SideBySide);
                    let light = crate::theme::names_for(cx, true).first().cloned().unwrap_or_default();
                    let dark = crate::app_settings::DEFAULT_DARK_THEME;
                    let system_is_dark =
                        matches!(cx.window_appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark);
                    let name = crate::theme::active_theme_name(
                        crate::theme::ThemeMode::System,
                        &light,
                        dark,
                        system_is_dark,
                    );
                    crate::theme::apply(cx, name);
                    // The endpoint and model are back at their defaults: rebuild the judge so
                    // the worker stops using the old ones.
                    this.configure_jev(cx);
                    this.apply_classifier_settings(cx);
                    panel_weak.update(cx, |panel, cx| panel.reset_fields(cx)).ok();
                    cx.notify();
                }
                SettingsEvent::AutoClassifyAge(days) => {
                    this.auto_max_age_days = *days;
                    this.save_setting(&crate::app_settings::AUTO_CLASSIFY_MAX_AGE_DAYS, *days);
                    this.apply_classifier_settings(cx);
                    cx.notify();
                }
                SettingsEvent::Changed { policy, summaries, auto_classify, spam_cold_outreach } => {
                    this.policy = policy.clone();
                    this.summaries_enabled = *summaries;
                    this.save_setting(&crate::app_settings::SUMMARIES, *summaries);
                    this.save_setting(&crate::app_settings::AUTO_CLASSIFY, *auto_classify);
                    this.save_setting(&crate::app_settings::SPAM_COLD_OUTREACH, *spam_cold_outreach);
                    for question in QuestionKey::ALL {
                        let setting = match question {
                            QuestionKey::Spam => &crate::app_settings::MODE_SPAM,
                            QuestionKey::NeedsReply => &crate::app_settings::MODE_NEEDS_REPLY,
                            QuestionKey::ExpectsReply => &crate::app_settings::MODE_EXPECTS_REPLY,
                            QuestionKey::Urgency => &crate::app_settings::MODE_URGENCY,
                            QuestionKey::Kind => &crate::app_settings::MODE_KIND,
                            QuestionKey::Folder => &crate::app_settings::MODE_FOLDER,
                        };
                        this.save_setting(setting, policy.mode(question));
                    }
                    // The switches and modes take effect now: re-route and re-queue.
                    this.apply_classifier_settings(cx);
                    cx.notify();
                }
                SettingsEvent::Theme { mode, light, dark } => {
                    this.theme_mode = *mode;
                    this.light_theme = light.clone();
                    this.dark_theme = dark.clone();
                    this.save_setting(&crate::app_settings::THEME_MODE, *mode);
                    this.save_setting(&crate::app_settings::LIGHT_THEME, light.clone());
                    this.save_setting(&crate::app_settings::DARK_THEME, dark.clone());
                    cx.set_window_appearance(match mode {
                        crate::theme::ThemeMode::Light => Some(WindowAppearance::Light),
                        crate::theme::ThemeMode::Dark => Some(WindowAppearance::Dark),
                        crate::theme::ThemeMode::System => None,
                    });
                    this.refresh_theme(cx);
                    cx.notify();
                }
                SettingsEvent::TabAvatars(on) => {
                    this.tab_avatars = *on;
                    this.save_setting(&crate::app_settings::TAB_AVATARS, *on);
                    cx.notify();
                }
                SettingsEvent::BlockRemoteImages(on) => {
                    this.block_remote_images = *on;
                    this.save_setting(&crate::app_settings::BLOCK_REMOTE_IMAGES, *on);
                    cx.notify();
                }
                SettingsEvent::Grouping(on) => {
                    this.set_grouping(*on);
                    this.save_setting(&crate::app_settings::GROUP_THREADS, *on);
                    cx.notify();
                }
                SettingsEvent::PreviewLines(n) => {
                    this.preview_lines = (*n).min(5);
                    this.save_setting(&crate::app_settings::PREVIEW_LINES, this.preview_lines);
                    cx.notify();
                }
                SettingsEvent::RowStyle(style) => {
                    this.row_style = *style;
                    if let Some(store) = &this.preferences {
                        crate::app_settings::save_row_style(
                            store.as_ref(),
                            &crate::prefs::Scope::Global,
                            *style,
                        );
                    }
                    cx.notify();
                }
                SettingsEvent::PaneLayout(orientation) => {
                    if this.panes.orientation() != *orientation {
                        this.set_pane_layout(*orientation, window, cx);
                    }
                    this.save_setting(
                        &crate::app_settings::PANE_LAYOUT,
                        if *orientation == PaneLayout::Stacked { "stacked" } else { "side_by_side" }.to_owned(),
                    );
                }
                SettingsEvent::FollowUp(timeout) => {
                    this.mailbox.set_follow_up_timeout(*timeout);
                    this.save_setting(
                        &crate::app_settings::FOLLOW_UP_DAYS,
                        (*timeout).div_euclid(DAY).clamp(1, 14) as u8,
                    );
                    cx.notify();
                }
                SettingsEvent::Unblock(email) => {
                    if this.mailbox.unblock_sender(email) {
                        this.show_undo_toast(format!("Unblocked {email}"), window, cx);
                    }
                }
                SettingsEvent::ClearSpamSender(email) => {
                    if this.mailbox.set_spam_sender(email, None) {
                        this.show_undo_toast(format!("Removed {email}"), window, cx);
                    }
                }
                SettingsEvent::AddGmail => this.add_gmail_account(window, cx),
                SettingsEvent::RemoveAccount(id) => this.remove_linked_account(id, window, cx),
                SettingsEvent::AccountNickname { id, nickname } => {
                    this.mailbox.set_account_nickname(id, nickname);
                    this.persist_account(id, window, cx);
                    cx.notify();
                }
                SettingsEvent::AccountStyle { id, icon, color } => this.set_account_style(id, icon, color, window, cx),
                SettingsEvent::SyncAction { id, action } => match action {
                    crate::sync_status::SyncAction::SignInAgain => this.sign_in_again(id, window, cx),
                    crate::sync_status::SyncAction::Retry => this.retry_sync(id, cx),
                },
                SettingsEvent::SaveJev { key, base, model } => {
                    if let Err(e) = this.save_jev_key(key.as_str()) {
                        this.show_toast(format!("Could not save the Jev key: {e}"), window, cx);
                    }
                    this.save_setting(&crate::app_settings::JEV_BASE_URL, base.clone());
                    this.save_setting(&crate::app_settings::JEV_MODEL, model.clone());
                    this.configure_jev(cx);
                    cx.notify();
                }
                SettingsEvent::RemoveJevKey => {
                    this.remove_jev_key(panel_weak.clone(), cx);
                    cx.notify();
                }
                SettingsEvent::TestJev { key, base, model } => {
                    this.test_jev(key.as_str(), base, model, panel_weak.clone(), cx);
                }
                SettingsEvent::Close => this.close_settings(window, cx),
            },
        ));
        self.settings = Some(panel);
        self.settings_window = Some(handle);
        cx.notify();
    }

    /// Keep the open Settings window's blocked list in step with the mailbox: undo in the
    /// main window restores a block behind the panel's back.
    pub(super) fn sync_settings_blocked(&mut self, cx: &mut Context<Self>) {
        // Every change to the spam-sender lists, undo included, is written here.
        let senders = self.mailbox.spam_senders().clone();
        if senders != self.saved_spam_senders {
            self.save_setting(&crate::app_settings::NEVER_SPAM_SENDERS, senders.never());
            self.save_setting(&crate::app_settings::ALWAYS_SPAM_SENDERS, senders.always());
            self.saved_spam_senders = senders;
        }
        let Some(panel) = &self.settings else {
            return;
        };
        let now = self.local_now();
        let blocked = self.mailbox.blocked();
        let senders = self.mailbox.spam_senders().clone();
        panel.update(cx, |panel, cx| {
            panel.set_blocked(blocked, now, cx);
            panel.set_spam_senders(senders, cx);
        });
    }

    fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.settings_window.take();
        self.settings = None;
        self._settings_sub = None;
        if let Some(handle) = handle {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
        window.focus(&self.focus_handle, cx);
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
        self.show_undo_toast("Thread muted".into(), window, cx);
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
        if allow && !crate::known_senders::KNOWN_SENDERS_ENABLED {
            return;
        }
        let Some((name, email)) = self.cursor_sender() else {
            return;
        };
        if !allow {
            self.open_block_dialog(name, email, false, window, cx);
            return;
        }
        if self.mailbox.allow_sender(&email) {
            self.show_undo_toast(format!("Allowed {email}"), window, cx);
        }
        self.move_cursor(0);
        cx.notify();
    }

    pub(super) fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal_open() {
            self.close_modals(window, cx);
        } else if self.filter_popover.is_some() {
            self.close_filter_popover();
        } else if self.is_filtered() {
            self.clear_filters(cx);
        } else if let Some(s) = self.session.take() {
            if s.handled > 0 {
                self.session_end = Some((s.handled, self.now() - s.started));
            } else {
                self.session_view = None;
            }
        } else if self.session_end.is_some() {
            self.end_session();
        } else {
            self.triage_mut().clear_selection();
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

    /// Standalone "New folder": pick the account first when there are several and none was
    /// given, then name the folder. Creation is one undo step.
    pub(super) fn new_folder(
        &mut self,
        account: Option<AccountId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let accounts = self.mailbox.accounts().to_vec();
        match (account, accounts.as_slice()) {
            (_, []) => {}
            (Some(account), _) => self.open_folder_name(account, window, cx),
            (None, [only]) => self.open_folder_name(only.id.clone(), window, cx),
            (None, _) => {
                let picker = cx.new(|cx| AccountPicker::new(accounts, window, cx));
                self._modal_sub = Some(cx.subscribe_in(
                    &picker,
                    window,
                    |this, _, event: &NewFolderEvent, window, cx| {
                        if let NewFolderEvent::Account(id) = event {
                            this.close_modals(window, cx);
                            this.open_folder_name(id.clone(), window, cx);
                        }
                    },
                ));
                self.new_folder = Some(picker.clone().into());
                self.host_in_dialog(picker.clone(), |vw| 420f32.min(vw * 0.9), window, cx);
                picker.update(cx, |p, cx| p.focus(window, cx));
                cx.notify();
            }
        }
    }

    fn open_folder_name(&mut self, account: AccountId, window: &mut Window, cx: &mut Context<Self>) {
        let label = self.mailbox.account(&account).map_or_else(
            || account.clone(),
            |a| crate::account_style::display_name(a).to_owned(),
        );
        let prompt = cx.new(|cx| NamePrompt::new(format!("New folder in {label}"), "", window, cx));
        self._modal_sub = Some(cx.subscribe_in(
            &prompt,
            window,
            move |this, _, event: &NewFolderEvent, window, cx| {
                let NewFolderEvent::Name(name) = event else { return };
                let name = name.trim();
                if name.is_empty() {
                    return;
                }
                this.close_modals(window, cx);
                let exists = this
                    .mailbox
                    .folders(&account)
                    .iter()
                    .any(|f| f.parent.is_none() && f.name.eq_ignore_ascii_case(name));
                if exists {
                    this.show_toast(format!("Folder “{name}” already exists"), window, cx);
                } else {
                    this.mailbox.create_folder(&account, name, None);
                    this.show_undo_toast(format!("Created folder “{name}”"), window, cx);
                }
                cx.notify();
            },
        ));
        self.new_folder = Some(prompt.clone().into());
        self.host_in_dialog(prompt.clone(), |vw| 420f32.min(vw * 0.9), window, cx);
        prompt.update(cx, |p, cx| p.focus(window, cx));
        cx.notify();
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
        let now = self.now();
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
                    mb.block_sender(email, Some(state), now)
                }
            }),
        };
        self.triage_mut().clear_selection();
        let text = match action {
            FileAction::BlockSender {
                unsubscribe: true, ..
            } => format!("Unsubscribed and filed {n}"),
            FileAction::BlockSender { .. } => format!("Blocked and filed {n}"),
            _ => format!("Filed {n}"),
        };
        self.show_undo_toast(text, window, cx);
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
        self.show_undo_toast(format!("{verb} {n}"), window, cx);
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
                .any(|(b, _)| b.eq_ignore_ascii_case(email))
        };
        if already {
            self.show_toast(format!("Already {verb} {email}"), window, cx);
            return;
        }
        let n = if unsubscribe {
            self.mailbox.unsubscribe(email, state)
        } else {
            self.mailbox.block_sender(email, state, self.now())
        };
        let tail = match state {
            Some(_) => format!(" · moved {n}"),
            None => String::new(),
        };
        self.show_undo_toast(format!("{verb} {email}{tail}"), window, cx);
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
        let sender = ids.first().and_then(|id| self.mailbox.get(*id)).map(|m| m.from_email.clone());
        let n = self.mailbox.mark_spam(ids, block, self.now());
        self.triage_mut().clear_selection();
        let text = if block {
            format!("Marked {n} as spam and blocked the sender")
        } else {
            format!("Marked {n} as spam")
        };
        self.show_undo_toast(text, window, cx);
        self.session_advance();
        self.scroll_to_cursor();
        cx.notify();
        // Blocking already swept the sender; a plain delete asks what to do about the rest.
        if let Some(email) = sender.filter(|_| n > 0 && !block) {
            self.offer_delete_and_always_spam(email, window, cx);
        }
    }

    /// After a spam-flagged message was marked not spam: ask to never mark this address as
    /// spam. Yes records it, as part of the same undo step. Skipped for an address that is
    /// already tracked or blocked.
    pub(super) fn offer_never_spam(&mut self, email: String, window: &mut Window, cx: &mut Context<Self>) {
        if !self.mailbox.spam_senders().should_prompt(&email, self.mailbox.is_blocked(&email)) {
            return;
        }
        let depth = self.mailbox.undo_depth();
        self.confirm(
            format!("Never mark mail from {email} as spam?"),
            move |this, window, cx| {
                this.mailbox.amend(depth, |m| m.set_spam_sender(&email, Some(SpamVerdict::Never)));
                this.show_undo_toast(format!("{email} is never spam"), window, cx);
            },
            window,
            cx,
        );
    }

    /// After spam was accepted by deleting without blocking: ask to delete the sender's other
    /// mail, then whether to always mark them as spam (Esc at either step ends the sequence).
    /// Both answers fold into the action's undo step. Skipped for a tracked or blocked address.
    pub(super) fn offer_delete_and_always_spam(
        &mut self,
        email: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.mailbox.spam_senders().should_prompt(&email, self.mailbox.is_blocked(&email)) {
            return;
        }
        let depth = self.mailbox.undo_depth();
        if self.mailbox.sender_ids_in(&email, TriageState::Inbox).is_empty() {
            return self.ask_always_spam(email, depth, window, cx);
        }
        self.confirm(
            format!("Delete all mail from {email}?"),
            move |this, window, cx| {
                let n = this.mailbox.amend(depth, |m| {
                    m.set_state_for_sender(&email, TriageState::Inbox, TriageState::Deleted)
                });
                this.show_undo_toast(format!("Deleted {n} more from {email}"), window, cx);
                this.session_advance();
                this.scroll_to_cursor();
                this.ask_always_spam(email.clone(), depth, window, cx);
            },
            window,
            cx,
        );
    }

    fn ask_always_spam(&mut self, email: String, depth: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm(
            format!("Always mark mail from {email} as spam?"),
            move |this, window, cx| {
                this.mailbox.amend(depth, |m| m.set_spam_sender(&email, Some(SpamVerdict::Always)));
                this.show_undo_toast(format!("{email} is always spam"), window, cx);
            },
            window,
            cx,
        );
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

    /// Ask before re-asking Jev about every incoming message the list shows.
    pub(super) fn confirm_classify_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ids: Vec<MessageId> = self
            .visible_ids()
            .into_iter()
            .filter(|id| self.mailbox.get(*id).is_some_and(|m| !m.outgoing))
            .collect();
        if ids.is_empty() {
            return;
        }
        let n = ids.len();
        let title = format!("Classify {n} message{} with Jev?", if n == 1 { "" } else { "s" });
        self.confirm(
            title,
            move |this, _, cx| this.classify_messages(&ids, cx),
            window,
            cx,
        );
    }

    /// Confirm dialog: `enter` runs `on_confirm`, `escape` cancels.
    fn confirm(
        &mut self,
        title: String,
        on_confirm: impl Fn(&mut MailApp, &mut Window, &mut Context<MailApp>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_dialog(
            DialogSpec::new(title, String::new(), vec![DialogOption::new("", "Confirm", "")], 0),
            move |this, _, window, cx| on_confirm(this, window, cx),
            window,
            cx,
        );
    }

    /// `Delete…` on a folder: ask first. The folder and its subfolders go, but their mail is
    /// archived, not deleted (as removing a Gmail label leaves the mail in All Mail). `enter`
    /// cancels, `1` deletes.
    pub(super) fn confirm_delete_folder(
        &mut self,
        folder: Option<FolderId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.modal_open() {
            return;
        }
        let Some(id) = self.target_folder(folder) else { return };
        let Some(name) = self.mailbox.folder(id).map(|f| f.name.clone()) else { return };
        let n = self.mailbox.folder_message_count(id);
        let tree = if self.mailbox.folder_subtree(id).len() > 1 { " and its subfolders" } else { "" };
        let mail = match n {
            0 => "It holds no messages.".to_owned(),
            1 => "Its 1 message is not deleted: it moves to Archive.".to_owned(),
            n => format!("Its {n} messages are not deleted: they move to Archive."),
        };
        let options = vec![
            DialogOption::new("1", "Delete folder", "Mail moves to Archive"),
            DialogOption::new("2", "Cancel", "Keep the folder"),
        ];
        self.open_dialog(
            DialogSpec::new(
                format!("Delete “{name}”?"),
                format!("“{name}”{tree} will be deleted. {mail}"),
                options,
                1,
            ),
            move |this, ix, window, cx| {
                if ix == 0 {
                    this.delete_folder_now(id, window, cx);
                }
            },
            window,
            cx,
        );
    }
}
