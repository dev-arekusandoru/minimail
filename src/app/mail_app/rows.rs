use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::ContextMenuExt as _;
use gpui_kit::component::menu::PopupMenu;
use super::*;

impl MailApp {
    /// Which surface the footer sits under right now.
    pub fn hint_mode(&self) -> HintMode {
        if self.compose.is_some() {
            HintMode::Compose
        } else if self.snooze.is_some() {
            HintMode::Snooze
        } else if self.settings.is_some() {
            HintMode::Settings
        } else if self.rules_panel.is_some() {
            HintMode::Rules
        } else if self.in_session() || self.session_end.is_some() {
            HintMode::Session(self.session_end.is_some())
        } else if self.new_senders_open() {
            HintMode::NewSenders
        } else {
            match self.triage().selected().len() {
                0 if self.opened().is_some() => HintMode::Reader,
                0 => HintMode::List,
                n => HintMode::Selection(n),
            }
        }
    }

    /// The mode plus the facts about the focused message that decide which keys apply.
    pub fn hint_context(&self) -> HintContext {
        let mut ctx = HintContext::new(self.hint_mode());
        let focused = self.opened().or_else(|| self.cursor_id());
        ctx.empty = focused.is_none();
        ctx.outside_inbox = !matches!(
            self.location(),
            Some(Location::AllInboxes | Location::Inbox(_))
        );
        if let Some(id) = focused {
            ctx.new_sender = self.mailbox.is_new_sender(id);
            ctx.suggestions = !self.mailbox.pending(id).is_empty();
            if let Some(m) = self.mailbox.get(id) {
                let tid = m.thread_id;
                ctx.multi_message =
                    self.mailbox.messages().iter().filter(|x| x.thread_id == tid).take(2).count() > 1;
            }
        }
        ctx
    }

    /// The date column of a row: the time when it falls on the same local day as the newest
    /// message, otherwise `"Sep 14"`. An unreadable `received` reads as written.
    pub(super) fn clock_label(&self, received: &str, newest: &str, now: &Now) -> String {
        fn day_of(s: &str) -> &str {
            s.get(..10).unwrap_or(s)
        }
        let local_day = |ts: Timestamp| now.day(ts);
        let parsed = crate::model::parse_rfc3339(received);
        let newest_parsed = crate::model::parse_rfc3339(newest);
        match (parsed, newest_parsed) {
            (Some(ts), Some(newest_ts)) if local_day(ts) == local_day(newest_ts) => now.hhmm(ts),
            (Some(ts), _) => {
                let (_, month, day) = now.civil(ts);
                format!("{} {day}", MONTHS[(month - 1) as usize])
            }
            // An unparseable stamp keeps its old shape: the time when it matches the newest
            // message's written day, else its written month and day.
            (None, _) if day_of(received) == day_of(newest) => {
                received.get(11..16).unwrap_or("").to_string()
            }
            (None, _) => {
                let month = received
                    .get(5..7)
                    .and_then(|m| m.parse::<usize>().ok())
                    .and_then(|m| MONTHS.get(m.wrapping_sub(1)))
                    .unwrap_or(&"?");
                let day = received.get(8..10).and_then(|d| d.parse::<u32>().ok()).unwrap_or(0);
                format!("{month} {day}")
            }
        }
    }

    pub(super) fn newest(&self) -> String {
        self.mailbox
            .messages()
            .iter()
            .map(|m| m.received.as_str())
            .max()
            .unwrap_or("")
            .to_string()
    }

    /// Height of a message row (sender/date, subject, preview): the rows the list measures.
    pub(super) fn message_row_h(&self) -> f32 {
        row::message_row_height(self.preview_lines)
    }

    /// Height of a thread header row: one compact line plus the preview lines it shows.
    pub(super) fn thread_row_h(&self) -> f32 {
        row::thread_row_height(self.preview_lines)
    }

    /// Cursor / open / selected / unread state of the row for message `id`.
    pub fn row_visual(&self, id: MessageId) -> RowVisual {
        let unread = !self.mailbox.is_read(id)
            && self.mailbox.state_of(id) == Some(TriageState::Inbox);
        RowVisual {
            cursor: self.cursor_id() == Some(id),
            open: self.opened() == Some(id),
            selected: self.triage().is_selected(id),
            partial: false,
            unread,
        }
    }

    /// Icons of the right-hand cluster of message `id` that fit the list width: `(shown, hidden)`.
    pub fn row_icons(&self, id: MessageId, width: f32) -> (Vec<Glyph>, Vec<Glyph>) {
        let Some(msg) = self.mailbox.get(id) else {
            return (Vec::new(), Vec::new());
        };
        let pending = self.mailbox.pending(id);
        let return_time_shown = matches!(self.location(), Some(Location::Snoozed(_)));
        let glyphs = icons::glyphs_for(&GlyphInputs {
            tags: self.mailbox.tags(id),
            pending: &pending,
            muted: self.mailbox.is_muted(msg.thread_id),
            new_sender: self.mailbox.is_new_sender(id),
            snoozed: self.mailbox.snoozed_until(id).is_some() && !return_time_shown,
            attachment: !msg.attachments.is_empty(),
        });
        icons::split_overflow(&glyphs, icons::max_icons(width))
    }

    pub(super) fn row_click(
        ix: usize,
        cx: &Context<Self>,
    ) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
        cx.listener(move |this, ev: &ClickEvent, window, cx| this.click_row(ix, ev, window, cx))
    }

    /// Message row: sender + date, full-width subject, optional preview, icon cluster.
    pub(super) fn render_row(&self, msg: &Message, ix: usize, newest: &str, cx: &Context<Self>) -> crate::app::ui::Observable {
        let t = cx.theme();
        let mut visual = self.row_visual(msg.id);
        visual.cursor = ix == self.cursor_ix();
        let date = match self.mailbox.snoozed_until(msg.id) {
            Some(until) if matches!(self.location(), Some(Location::Snoozed(_))) => {
                format!("↩ {}", format_when(until, &self.local_now()))
            }
            _ => self.clock_label(&msg.received, newest, &self.local_now()),
        };
        let unread = visual.unread;
        let (shown, hidden) = self.row_icons(msg.id, self.list_w);
        let pending = self.mailbox.pending(msg.id);
        let emphasis = if unread || visual.open { FontWeight::SEMIBOLD } else { FontWeight::NORMAL };
        let lines = self.preview_lines;
        let folder_hint = pending
            .iter()
            .find(|s| s.key == QuestionKey::Folder)
            .and_then(|s| self.mailbox.suggested_folder(s))
            .map(|f| self.mailbox.folder_path(f));
        let ix_id = msg.id as usize;
        let sender = row::sender_label(msg);
        let account_icon = if row::shows_account_icon(&self.location().unwrap_or(Location::AllInboxes)) {
            self.mailbox.account(&msg.account).map(|account| {
                icons::account_icon(crate::account_style::icon_key(account), &account.color, t, 13.)
            })
        } else {
            None
        };
        let row_ids = [msg.id];
        let label = (SharedString::from(sender.clone()), SharedString::from(msg.subject.clone()));
        let container = div().id(("row", ix_id)).h(px(self.message_row_h()));
        let container = self.draggable(container, &row_ids, self.thread_of(msg.id), label);
        // Right-clicking the row's body opens the same classifier menu as its icons.
        let owner = cx.weak_entity();
        let menu_id = msg.id;
        let classifier_menu =
            move |menu: PopupMenu, window: &mut Window, cx: &mut Context<PopupMenu>| -> PopupMenu {
                Self::populate_menu(owner.clone(), MenuKind::Classifier(menu_id), menu, window, cx)
            };
        row::frame(container, visual, t)
            .child(self.row_selection_target(ix, cx))
            .child(
                div()
                    .id(("row-body", ix_id))
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .justify_center()
                    .overflow_hidden()
                    .on_click(Self::row_click(ix, cx))
                    .context_menu(classifier_menu)
                    .child(
                        div()
                            .h(px(18.))
                            .flex()
                            .items_center()
                            .gap_1()
                            .when_some(account_icon, |d, icon| {
                                d.child(
                                    div()
                                        .id(("row-account-icon", ix_id))
                                        .test_support()
                                        .flex_none()
                                        .flex()
                                        .items_center()
                                        .child(icon),
                                )
                            })
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_color(t.foreground)
                                    .font_weight(emphasis)
                                    .child(sender),
                            ),
                    )
                    .child(
                        div()
                            .h(px(18.))
                            .truncate()
                            .text_color(t.foreground)
                            .font_weight(emphasis)
                            .child(msg.subject.clone()),
                    )
                    .when(lines > 0, |d| {
                        d.child(
                            div()
                                .h(px(row::PREVIEW_LINE_H * f32::from(lines)))
                                .line_clamp(lines as usize)
                                .text_size(px(12.))
                                .line_height(px(row::PREVIEW_LINE_H))
                                .text_color(t.muted_foreground)
                                .child(crate::preview::snippet(&crate::reading::reader_text(msg))),
                        )
                    }),
            )
            .child(
                div()
                    .h_full()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .items_end()
                    .justify_start()
                    .pt(px(3.))
                    .child(
                        div()
                            .id(("row-date", ix_id))
                            .h(px(18.))
                            .flex()
                            .items_center()
                            .text_size(px(11.))
                            .text_color(if unread { t.primary } else { t.muted_foreground })
                            .on_click(Self::row_click(ix, cx))
                            .child(date),
                    )
                    .child(
                        div()
                            .id(("row-badges", ix_id))
                            .h(px(18.))
                            .flex()
                            .items_center()
                            .when_some(self.classification_state(msg.id), |d, state| {
                                d.child(icons::classification_badge(
                                    ("row-classify", ix_id),
                                    state,
                                    t,
                                    12.,
                                ))
                            })
                            // Any icon of the cluster opens the classifier menu: clicking one no
                            // longer accepts a suggestion directly, the menu is where the user
                            // accepts or rejects, and where a label is corrected.
                            .child(self.menu_trigger(
                                MenuKind::Classifier(msg.id),
                                Button::new(("row-menu", ix_id))
                                    .ghost()
                                    .h(px(18.))
                                    .px_0()
                                    .child(icons::cluster(
                                        &shown,
                                        &hidden,
                                        folder_hint.as_deref(),
                                        t,
                                        ix_id,
                                    )),
                                cx,
                            )),
                    ),
            )
    }
}
