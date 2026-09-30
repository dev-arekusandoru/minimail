use super::*;

impl MailApp {
    pub(super) fn open_palette(&mut self, prefill: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
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

    pub(super) fn start_search(&mut self, raw: &str, cx: &mut Context<Self>) {
        let q = raw.trim().trim_start_matches('/').trim().to_string();
        self.end_session();
        self.mode = ListMode::Search(q);
        self.alt_cursor = 0;
        self.opened = None;
        cx.notify();
    }

    pub(super) fn open_compose(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(msg) = self.cursor_id().and_then(|id| self.mailbox.get(id)).cloned() else {
            return;
        };
        self.open_compose_for(&msg, None, window, cx);
    }

    pub(super) fn open_compose_for(
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
                        let expects_reply = crate::judge::expects_reply(&StubJudge, body);
                        this.mailbox.send_reply_at(*in_reply_to, body.clone(), expects_reply, now);
                        this.close_modals(window, cx);
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
        if self.in_session() {
            let ids = self.target_ids();
            let now = self.now();
            if let Some((_, until)) = self.mailbox.snooze_presets(now).into_iter().next() {
                self.mailbox.snooze(&ids, until, now);
                self.triage.clear_selection();
                self.session_advance();
            }
            cx.notify();
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
                SettingsEvent::Close => this.close_modals(window, cx),
            },
        ));
        window.focus(&panel.focus_handle(cx), cx);
        self.settings = Some(panel);
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
        window.focus(&panel.focus_handle(cx), cx);
        self.rules_panel = Some(panel);
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
        let Some(email) = self
            .cursor_id()
            .and_then(|id| self.mailbox.get(id))
            .map(|m| m.from_email.clone())
        else {
            return;
        };
        self.mailbox.unsubscribe(&email, None);
        self.show_toast(format!("Unsubscribed from {email} · u to undo"), window, cx);
        self.session_advance();
        cx.notify();
    }

    pub(super) fn screen_sender(&mut self, allow: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.new_senders_open() {
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
            self.mailbox.block_sender(&email, None);
            format!("Blocked {email} · u to undo")
        };
        self.show_toast(text, window, cx);
        self.move_cursor(0);
        cx.notify();
    }

    pub(super) fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.help {
            self.help = false;
        } else if self.menu_open() {
            self.close_menu(window, cx);
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
}
