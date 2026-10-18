use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::ContextMenuExt as _;
use super::*;
use crate::app::row_parts;
use crate::app::tile::{TileSize, TileSpec, sender_tile};
use crate::row_style::Chips;
use crate::theme::ThemeColor;

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

    /// Cursor / open / selected / unread state of the row for message `id`.
    pub fn row_visual(&self, id: MessageId) -> RowVisual {
        let unread = !self.mailbox.is_read(id)
            && self.mailbox.state_of(id) == Some(TriageState::Inbox);
        RowVisual {
            cursor: self.cursor_id() == Some(id),
            open: self.opened() == Some(id),
            selected: self.triage().is_selected(id),
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
        let glyphs: Vec<Glyph> = glyphs
            .into_iter()
            .filter(|g| {
                self.row_style.chips_as_icons()
                    || !matches!(g, Glyph::FolderSuggestion | Glyph::UrgentHigh | Glyph::UrgentMid)
            })
            .collect();
        icons::split_overflow(&glyphs, icons::max_icons(width))
    }

    pub(super) fn row_click(
        ix: usize,
        cx: &Context<Self>,
    ) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
        cx.listener(move |this, ev: &ClickEvent, window, cx| this.click_row(ix, ev, window, cx))
    }

    /// The sender's account icon in its account color, in the cross-account views only.
    pub(super) fn account_slot(&self, msg: &Message, t: &ThemeColor) -> Option<AnyElement> {
        if !row::shows_account_icon(&self.location().unwrap_or(Location::AllInboxes)) {
            return None;
        }
        let account = self.mailbox.account(&msg.account)?;
        let icon = icons::account_icon(crate::account_style::icon_key(account), &account.color, t, 13.);
        Some(
            div()
                .id(("row-account-icon", msg.id as usize))
                .test_support()
                .flex_none()
                .flex()
                .items_center()
                .child(icon)
                .into_any_element(),
        )
    }

    /// The icon cluster of message `id`: classification badge plus the icons, all opening the
    /// classifier menu (the menu is where the user accepts or rejects, and corrects a label).
    pub(super) fn icon_cluster(
        &self,
        id: MessageId,
        shown: &[Glyph],
        hidden: &[Glyph],
        folder_hint: Option<&str>,
        t: &ThemeColor,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let ix_id = id as usize;
        div()
            .id(("row-badges", ix_id))
            .h(px(row::LINE_H))
            .flex()
            .items_center()
            .when_some(self.classification_state(id), |d, state| {
                d.child(icons::classification_badge(("row-classify", ix_id), state, t, 12.))
            })
            .child(self.menu_trigger(
                MenuKind::Classifier(id),
                Button::new(("row-menu", ix_id))
                    .ghost()
                    .h(px(row::LINE_H))
                    .px_0()
                    .child(icons::cluster(shown, hidden, folder_hint, t, ix_id)),
                cx,
            ))
    }

    /// The chips a row carries: the pending folder suggestion and the urgency (a set label,
    /// else a pending suggestion); empty when chips are off.
    pub(super) fn row_chips(&self, id: MessageId, t: &ThemeColor) -> Vec<row_parts::Chip> {
        if self.row_style.chips_as_icons() {
            return Vec::new();
        }
        let pending = self.mailbox.pending(id);
        let folder = self.folder_hint(id);
        let urgency = self
            .mailbox
            .tags(id)
            .iter()
            .find_map(|tag| if let Tag::Urgent(n) = tag { Some(*n) } else { None })
            .or_else(|| pending.iter().find_map(|s| s.urgency()));
        row_parts::chips_for(folder, urgency, t)
    }

    /// Path of the folder a pending Folder suggestion on `id` points at.
    pub(super) fn folder_hint(&self, id: MessageId) -> Option<String> {
        self.mailbox
            .pending(id)
            .iter()
            .find(|s| s.key == QuestionKey::Folder)
            .and_then(|s| self.mailbox.suggested_folder(s))
            .map(|f| self.mailbox.folder_path(f))
    }

    /// Standard message row: rail (gutter time | marker) or sender tile + time column, then the
    /// body: optional chips, subject line, `sender · preview` block, optional chips.
    pub(super) fn render_row(&self, msg: &Message, ix: usize, _newest: &str, cx: &Context<Self>) -> crate::app::ui::Observable {
        let t = cx.theme();
        let style = self.row_style;
        let mut visual = self.row_visual(msg.id);
        visual.cursor = ix == self.cursor_ix();
        let unread = visual.unread;
        let (shown, hidden) = self.row_icons(msg.id, self.list_w);
        let ix_id = msg.id as usize;
        let sender = row::sender_label(msg);
        let account_icon = self.account_slot(msg, t);
        let folder_hint = self.folder_hint(msg.id);
        let chips = self.row_chips(msg.id, t);
        let (above, below) = match style.chips_placement() {
            Chips::Above => (row_parts::chips_line(chips, t), None),
            Chips::Below => (None, row_parts::chips_line(chips, t)),
            Chips::Off => (None, None),
        };
        let lines = self.preview_lines;
        let snippet = if lines > 0 {
            crate::preview::snippet(&crate::reading::reader_text(msg))
        } else {
            String::new()
        };
        let rail = style.rail();
        let cluster = self.icon_cluster(msg.id, &shown, &hidden, folder_hint.as_deref(), t, cx);
        let (rail_cluster, side_cluster) =
            if rail { (Some(cluster.into_any_element()), None) } else { (None, Some(cluster)) };

        let (first, second, block) = if style.subject_first() {
            (
                row_parts::title(msg.subject.clone(), unread, t),
                None,
                row_parts::text_block(Some(&sender), &snippet, lines.max(1), unread, t),
            )
        } else {
            (
                row_parts::title(sender.clone(), unread, t),
                Some(row_parts::title(msg.subject.clone(), unread, t).h(px(row::LINE_H)).truncate()),
                row_parts::text_block(None, &snippet, lines, false, t),
            )
        };
        let line1 = row_parts::title_line(account_icon, first, rail_cluster);
        let body = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .when_some(above, |d, c| d.child(c))
            .child(line1)
            .when_some(second, |d, s| d.child(s))
            .when_some(block, |d, b| d.child(b))
            .when_some(below, |d, c| d.child(c));

        let account_color = self
            .mailbox
            .account(&msg.account)
            .filter(|_| row::shows_account_icon(&self.location().unwrap_or(Location::AllInboxes)))
            .and_then(|a| crate::theme::parse_color(&a.color));
        let inner = div()
            .id(("row-body", ix_id))
            .w_full()
            .min_w_0()
            .py(px(4.))
            .flex()
            .items_start()
            .gap(px(if rail { 8. } else { 10. }))
            .on_click(Self::row_click(ix, cx))
            .context_menu(self.context_menu(MenuKind::Message(msg.id), cx));
        let inner = if rail {
            inner
                .child(row_parts::gutter(self.gutter_label(msg), unread, t))
                .child(row_parts::marker(visual, account_color, t))
                .child(body)
        } else {
            let who = if msg.outgoing { msg.to.to_string() } else { msg.from_name.clone() };
            inner
                .child(
                    div().flex_none().mt(px(1.)).child(sender_tile(
                        TileSpec {
                            id: ("row-tile", ix_id).into(),
                            senders: vec![who],
                            style: style.group_tile(),
                            size: TileSize::Row,
                            selected: visual.selected,
                            unread: usize::from(unread),
                            row_bg: row::row_bg(visual, t),
                        },
                        t,
                    )),
                )
                .child(body)
                .child(
                    div()
                        .flex_none()
                        .flex()
                        .flex_col()
                        .items_end()
                        .child(
                            div()
                                .h(px(row::LINE_H))
                                .flex()
                                .items_center()
                                .text_size(px(11.))
                                .text_color(if unread { t.primary } else { t.muted_foreground })
                                .child(self.gutter_label(msg)),
                        )
                        .children(side_cluster),
                )
        };

        let container = div().id(("row", ix_id));
        let container = self.draggable(
            container,
            &[msg.id],
            self.thread_of(msg.id),
            (SharedString::from(sender), SharedString::from(msg.subject.clone())),
        );
        row::frame(container, visual, t)
            .when(rail, |d| d.child(super::day::rail_segment(t)))
            .child(self.row_selection_target(ix, cx))
            .child(inner)
    }
}
