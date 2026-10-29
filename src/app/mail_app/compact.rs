//! Compact density: one fixed 26px line per message, thread header and thread child.
//! Spec: `mockups/compact.html`. Compact has no rail, tiles, chips or preview lines; folder and
//! urgency show as cluster icons and the preview trails the subject on the same line.
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::ContextMenuExt as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _};
use super::*;
use crate::row_style::NARROW_LIST_W;
use crate::theme::ThemeColor;

/// Height of every compact row.
const ROW_H: f32 = 26.;

/// What differs between the three compact row kinds, drawn by [`MailApp::compact_line`].
struct Line {
    /// Row index (cursor space) and the element id of the body and menu trigger (unique per row).
    ix: usize,
    id: usize,
    visual: RowVisual,
    /// Dot / count-node color: the account color in cross-account views.
    accent: Hsla,
    marker: Marker,
    time: String,
    account: Option<Icon>,
    sender: String,
    sender_w: f32,
    subject: Option<String>,
    /// Text after the subject; with no subject (a thread child) it is the row's only text.
    preview: Option<String>,
    unread_chip: Option<(usize, usize)>,
    glyphs: Vec<Glyph>,
    /// Message whose classifier menu and context menu the row opens; none on a thread header.
    menu: Option<MessageId>,
    chevron: Option<(u32, bool)>,
}

enum Marker {
    Dot,
    Count(String),
}

impl MailApp {
    fn compact_narrow(&self) -> bool {
        self.list_w < NARROW_LIST_W
    }

    /// Cluster glyphs of message `id`, capped for the list width.
    fn compact_glyphs(&self, id: MessageId) -> Vec<Glyph> {
        let Some(msg) = self.mailbox.get(id) else {
            return Vec::new();
        };
        let pending = self.mailbox.pending(id);
        let glyphs = icons::glyphs_for(&GlyphInputs {
            tags: self.mailbox.tags(id),
            pending: &pending,
            muted: self.mailbox.is_muted(msg.thread_id),
            new_sender: self.mailbox.is_new_sender(id),
            snoozed: self.mailbox.snoozed_until(id).is_some()
                && !matches!(self.location(), Some(Location::Snoozed(_))),
            attachment: !msg.attachments.is_empty(),
        });
        crate::compact::cluster_icons(&glyphs, self.compact_narrow())
    }

    /// The account icon and color of `msg`, in the cross-account views only.
    fn compact_account(&self, msg: &Message, t: &ThemeColor) -> (Option<Icon>, Hsla) {
        let cross = row::shows_account_icon(&self.location().unwrap_or(Location::AllInboxes));
        let account = cross.then(|| self.mailbox.account(&msg.account)).flatten();
        let accent = account.and_then(|a| theme::parse_color(&a.color)).unwrap_or(t.primary);
        let icon = account
            .map(|a| icons::account_icon(crate::account_style::icon_key(a), &a.color, t, 13.));
        (icon, accent)
    }

    fn compact_preview(msg: &Message) -> String {
        crate::preview::snippet(&crate::reading::reader_text(msg))
    }

    /// The line shared by all compact rows: marker | time | account | sender | text | chip | icons.
    fn compact_line(
        &self,
        container: Stateful<Div>,
        line: Line,
        cx: &Context<Self>,
    ) -> crate::app::ui::Observable {
        let t = cx.theme();
        let Line { ix, id, visual, accent, marker, time, account, sender, sender_w, subject, preview, unread_chip, glyphs, menu, chevron } = line;
        let unread = visual.unread;
        let strong = if unread { FontWeight::SEMIBOLD } else { FontWeight::NORMAL };
        let mark = match marker {
            Marker::Dot if visual.selected => div()
                .size(px(13.))
                .rounded_full()
                .bg(t.primary)
                .flex()
                .items_center()
                .justify_center()
                .child(Icon::new(IconName::Check).with_size(px(9.)).text_color(t.background))
                .into_any_element(),
            Marker::Dot if unread => div().size(px(7.)).rounded_full().bg(accent).into_any_element(),
            Marker::Dot => div().into_any_element(),
            Marker::Count(n) => {
                let node = div()
                    .min_w(px(15.))
                    .h(px(15.))
                    .px(px(3.))
                    .rounded(px(8.))
                    .border_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(9.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .font_family(crate::app::ui::mono_font(cx))
                    .child(n);
                let node = if unread {
                    node.bg(accent).border_color(accent).text_color(t.background)
                } else {
                    node.border_color(t.muted_foreground).text_color(t.muted_foreground)
                };
                node.into_any_element()
            }
        };
        let subject_shown = subject.is_some();
        let text = div()
            .flex_1()
            .min_w_0()
            .flex()
            .items_center()
            .overflow_hidden()
            .when_some(subject, |d, s| {
                d.child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_color(t.foreground)
                        .font_weight(strong)
                        .child(s),
                )
            })
            .when_some(preview, |d, p| d.child(p_cell(p, subject_shown, unread, t)));
        let chip = unread_chip.map(|(thread, n)| {
            div()
                .id(("thread-unread", thread))
                .flex_none()
                .h(px(16.))
                .px(px(6.))
                .rounded(px(4.))
                .bg(t.accent)
                .text_size(px(11.))
                .line_height(px(16.))
                .text_color(t.primary)
                .tooltip(move |window, cx| Tooltip::new(format!("{n} unread")).build(window, cx))
                .child(n.to_string())
        });
        let cluster = icons::cluster(&glyphs, &[], None, t, id);
        let trigger = match menu {
            Some(mid) => self
                .menu_trigger(
                    MenuKind::Classifier(mid),
                    Button::new(("row-menu", id)).ghost().h(px(18.)).px_0().child(cluster),
                    cx,
                )
                .into_any_element(),
            None => cluster.into_any_element(),
        };
        let chevron = chevron.map(|(thread, expanded)| {
            div()
                .id(("thread-chevron", thread as usize))
                .flex_none()
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.cursor_to(ix);
                    this.toggle_thread(thread);
                    window.focus(&this.focus_handle, cx);
                    cx.notify();
                }))
                .child(
                    Icon::new(if expanded { IconName::ChevronDown } else { IconName::ChevronRight })
                        .with_size(px(13.))
                        .text_color(t.muted_foreground),
                )
        });
        let body = div()
            .id(("row-body", id))
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .items_center()
            .gap_2()
            .overflow_hidden()
            .on_click(Self::row_click(ix, cx))
            .child(div().size(px(15.)).flex_none().flex().items_center().justify_center().child(mark))
            .child(
                div()
                    .w(px(36.))
                    .flex_none()
                    .truncate()
                    .text_size(px(11.))
                    .font_family(crate::app::ui::mono_font(cx))
                    .text_color(if unread { t.primary } else { t.muted_foreground })
                    .child(time),
            )
            .when_some(account, |d, icon| {
                d.child(
                    div()
                        .id(("row-account-icon", id))
                        .test_support()
                        .flex_none()
                        .flex()
                        .items_center()
                        .child(icon),
                )
            })
            .child(
                div()
                    .w(px(sender_w))
                    .flex_none()
                    .truncate()
                    .text_color(t.foreground)
                    .font_weight(strong)
                    .child(sender),
            )
            .child(text);
        let body = match menu {
            Some(mid) => body.context_menu(self.context_menu(MenuKind::Message(mid), cx)).into_any_element(),
            None => body.into_any_element(),
        };
        row::frame(container, visual, t)
            .child(self.row_selection_target(ix, cx))
            .child(body)
            .children(chip)
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(trigger)
                    .children(chevron),
            )
    }
}

/// The text after the subject: a dimmed `— preview`, or a child row's own preview text.
fn p_cell(text: String, after_subject: bool, unread: bool, t: &ThemeColor) -> Div {
    let color = if !after_subject && unread { t.foreground } else { t.muted_foreground };
    div()
        .flex_1()
        .min_w_0()
        .truncate()
        .when(after_subject, |d| d.ml(px(4.)))
        .text_size(px(12.))
        .text_color(color)
        .child(text)
}

impl MailApp {
    /// Compact message row.
    pub(super) fn render_compact_row(
        &self,
        msg: &Message,
        ix: usize,
        list: &ListFrame,
        cx: &Context<Self>,
    ) -> crate::app::ui::Observable {
        let t = cx.theme();
        let visual = self.row_state(msg.id, ix == list.cursor);
        let (account, accent) = self.compact_account(msg, t);
        let sender = row::sender_label(msg);
        let id = msg.id as usize;
        let label = (SharedString::from(sender.clone()), SharedString::from(msg.subject.clone()));
        let container = div().id(("row", id)).h(px(ROW_H));
        let container = self.draggable(container, &[msg.id], self.thread_of(msg.id), label);
        let line = Line {
            ix,
            id,
            visual,
            accent,
            marker: Marker::Dot,
            time: self.gutter_label(msg),
            account,
            sender,
            sender_w: if self.compact_narrow() { 80. } else { 104. },
            subject: Some(msg.subject.clone()),
            preview: (!self.compact_narrow()).then(|| format!("— {}", Self::compact_preview(msg))),
            unread_chip: None,
            glyphs: self.compact_glyphs(msg.id),
            menu: Some(msg.id),
            chevron: None,
        };
        self.compact_line(container, line, cx)
    }

    /// Compact thread header: count node, latest message's line, unread chip, chevron.
    pub(super) fn render_compact_header(
        &self,
        row: &Row,
        ix: usize,
        list: &ListFrame,
        cx: &Context<Self>,
    ) -> crate::app::ui::Observable {
        let Row::Header { thread_id, ids, expanded } = row else {
            unreachable!("render_compact_header takes header rows")
        };
        let t = cx.theme();
        let thread = *thread_id;
        let latest = self.mailbox.get(ids[0]);
        let unread_n = ids
            .iter()
            .filter(|id| !self.mailbox.is_read(**id))
            .count();
        let selected_count = ids.iter().filter(|id| self.triage().is_selected(**id)).count();
        let visual = RowVisual {
            cursor: ix == list.cursor,
            open: !*expanded && self.opened().is_some_and(|id| ids.contains(&id)),
            selected: selected_count == ids.len() && selected_count > 0,
            unread: unread_n > 0,
        };
        let (account, accent) = latest.map_or((None, t.primary), |m| self.compact_account(m, t));
        let senders = crate::threads::senders(ids, |id| self.mailbox.get(id));
        let sender = senders.iter().take(2).cloned().collect::<Vec<_>>().join(", ");
        let subject = latest.map(|m| m.subject.clone()).unwrap_or_default();
        let label = (SharedString::from(sender.clone()), SharedString::from(subject.clone()));
        let container = div().id(("thread-row", thread as usize)).h(px(ROW_H));
        let container = self.draggable(container, ids, Some(thread), label);
        let line = Line {
            ix,
            id: thread as usize,
            visual,
            accent,
            marker: Marker::Count(crate::threads::node_label(ids.len()).0),
            time: latest.map(|m| self.gutter_label(m)).unwrap_or_default(),
            account,
            sender,
            sender_w: if self.compact_narrow() { 80. } else { 104. },
            subject: Some(subject),
            preview: (!*expanded && !self.compact_narrow())
                .then(|| latest.map(|m| format!("— {}", Self::compact_preview(m))))
                .flatten(),
            unread_chip: (unread_n > 0).then_some((thread as usize, unread_n)),
            glyphs: self.compact_glyphs(ids[0]),
            menu: None,
            chevron: Some((thread, *expanded)),
        };
        self.compact_line(container, line, cx)
    }

    /// Compact thread child: darker, indented, preview-only text, a guide line down the left.
    pub(super) fn render_compact_child(
        &self,
        row: &Row,
        ix: usize,
        list: &ListFrame,
        cx: &Context<Self>,
    ) -> crate::app::ui::Observable {
        let t = cx.theme();
        let Some(msg) = self.mailbox.get(row.primary()) else {
            return row::frame(div().id(("row", ix)).h(px(ROW_H)), RowVisual::default(), t);
        };
        let visual = self.row_state(msg.id, ix == list.cursor);
        let rows = &list.rows;
        let first = ix.checked_sub(1).and_then(|p| rows.get(p)).is_some_and(|r| matches!(r, Row::Header { .. }));
        let last = !matches!(rows.get(ix + 1), Some(Row::Child { .. }));
        let sender = row::sender_label(msg);
        let id = msg.id as usize;
        let label = (SharedString::from(sender.clone()), SharedString::from(msg.subject.clone()));
        let mut darker = t.background;
        darker.l *= 0.89;
        let container = div().id(("row", id)).h(px(ROW_H)).bg(darker);
        let container = self.draggable(container, &[msg.id], self.thread_of(msg.id), label);
        let line = Line {
            ix,
            id,
            visual,
            accent: t.primary,
            marker: Marker::Dot,
            time: self.gutter_label(msg),
            account: None,
            sender,
            sender_w: if self.compact_narrow() { 72. } else { 88. },
            subject: None,
            preview: Some(Self::compact_preview(msg)),
            unread_chip: None,
            glyphs: self.compact_glyphs(msg.id).into_iter().filter(|g| *g == Glyph::Attachment).collect(),
            menu: Some(msg.id),
            chevron: None,
        };
        // The guide runs the full row height, but starts and ends at mid-row at the thread's ends.
        let guide = div()
            .absolute()
            .left(px(17.))
            .top(px(if first { ROW_H / 2. - 1. } else { -1. }))
            .bottom(px(if last { ROW_H / 2. - 1. } else { -1. }))
            .w(px(1.))
            .bg(t.border);
        self.compact_line(container, line, cx)
            .child(guide)
            .pl(px(28.))
    }
}
