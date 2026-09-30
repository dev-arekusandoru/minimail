use super::*;

impl Focusable for MailApp {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for MailApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let counts = TriageState::ALL.map(|s| (s, self.mailbox.count(s)));
        let screener_count = self.mailbox.screener_ids().len();
        let screener_active = self.mode == ListMode::Screener;
        let hint = self.hint_mode();
        let in_session = self.in_session() || self.session_end.is_some();
        if let Some(id) = self.opened {
            self.read.insert(id);
        }
        self.list_w = row::list_width(f32::from(window.viewport_size().width));
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
        let t = theme::active(cx);
        let (bg, fg, border, muted, sidebar) = (t.background, t.text, t.border, t.text_muted, t.sidebar);
        let active_view = self.triage.view;
        div()
            .id("mail-app")
            .track_focus(&self.focus_handle)
            .when(!self.modal_open(), |d| d.key_context(MAIL_CONTEXT))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, window, cx| {
                    // Clicking never steals key focus from the list (or a modal's input).
                    if !this.modal_open() {
                        window.focus(&this.focus_handle, cx);
                    }
                }),
            )
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(bg)
            .text_color(fg)
            .text_size(px(13.))
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| {
                if this.help {
                    return this.scroll_help_lines(1., cx);
                }
                this.move_cursor(1);
                this.scroll_to_cursor();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &SelectPrev, _, cx| {
                if this.help {
                    return this.scroll_help_lines(-1., cx);
                }
                this.move_cursor(-1);
                this.scroll_to_cursor();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ExtendNext, _, cx| {
                if this.mode == ListMode::State && !this.in_session() {
                    this.extend_by(1);
                }
                this.scroll_to_cursor();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ExtendPrev, _, cx| {
                if this.mode == ListMode::State && !this.in_session() {
                    this.extend_by(-1);
                }
                this.scroll_to_cursor();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleSelect, _, cx| {
                if this.mode == ListMode::State && !this.in_session() {
                    this.toggle_select_cursor();
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ClearSelection, window, cx| this.escape(window, cx)))
            .on_action(cx.listener(|this, _: &OpenMessage, _, cx| {
                this.open_cursor();
                cx.notify();
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
                if this.help {
                    this.close_help(cx);
                } else {
                    this.help = true;
                    this.help_scroll.set_offset(point(px(0.), px(0.)));
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &HelpPageUp, _, cx| this.scroll_help(-1., cx)))
            .on_action(cx.listener(|this, _: &HelpPageDown, _, cx| this.scroll_help(1., cx)))
            .on_action(cx.listener(|this, _: &OpenSnoozePicker, w, cx| {
                if !this.modal_open() {
                    this.open_snooze(w, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &AcceptSuggestions, _, cx| this.accept_suggestions(cx)))
            .on_action(cx.listener(|this, _: &RejectSuggestions, _, cx| this.reject_suggestions(cx)))
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
            .on_action(cx.listener(|this, _: &ToggleGrouping, w, cx| this.toggle_grouping(w, cx)))
            .on_action(cx.listener(|this, _: &CyclePreviewLines, w, cx| {
                this.preview_lines = crate::preview::cycle_lines(this.preview_lines);
                let text = format!("Preview: {}", crate::preview::lines_label(this.preview_lines));
                this.show_toast(text, w, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ExpandThread, _, cx| {
                this.set_expanded(true);
                this.scroll_to_cursor();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &CollapseThread, _, cx| {
                this.set_expanded(false);
                this.scroll_to_cursor();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &NextInThread, w, cx| this.step_thread(1, w, cx)))
            .on_action(cx.listener(|this, _: &PrevInThread, w, cx| this.step_thread(-1, w, cx)))
            .child(self.render_toolbar(cx))
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
                                .flex()
                                .items_center()
                                .gap_3()
                                .px_3()
                                .py_1()
                                .rounded_md()
                                .bg(t.success)
                                .text_color(t.on_accent)
                                .text_size(px(12.))
                                .font_weight(FontWeight::MEDIUM)
                                .child(text.clone())
                                .when(text.contains("undo"), |d| {
                                    d.child(
                                        button("toast-undo", "Undo", "Undo", "u", cx)
                                            .on_click(run(Undo)),
                                    )
                                }),
                        ),
                )
            })
            .when(self.help, |d| {
                let scroll = self.help_scroll.clone();
                d.child(
                    div()
                        .id("help-backdrop")
                        .absolute()
                        .inset_0()
                        .occlude()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(bg.opacity(0.85))
                        .text_color(muted)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| this.close_help(cx)),
                        )
                        .child(HelpOverlay::new(
                            scroll,
                            cx.listener(|this, _, _, cx| this.close_help(cx)),
                        )),
                )
            })
            .when_some(self.palette.clone(), |d, palette| {
                d.child(overlay(window, palette).on_mouse_down(MouseButton::Left, close_on_backdrop(cx)))
            })
            .when_some(self.snooze.clone(), |d, picker| {
                d.child(overlay(window, picker).on_mouse_down(MouseButton::Left, close_on_backdrop(cx)))
            })
            .when_some(self.settings.clone(), |d, panel| {
                d.child(overlay(window, panel).on_mouse_down(MouseButton::Left, close_on_backdrop(cx)))
            })
            .when_some(self.rules_panel.clone(), |d, panel| {
                d.child(overlay(window, panel).on_mouse_down(MouseButton::Left, close_on_backdrop(cx)))
            })
    }
}
