//! Standard-density thread rows: the header of a thread (collapsed or expanded) and its child
//! rows, in the rail and the tiles layout. Geometry follows `mockups/thread-default.html` (rail)
//! and the thread parts of `mockups/tiles-default.html` (tiles); colors are theme roles.
//! Click selects, the chevron expands/collapses, the whole row drags.
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::menu::ContextMenuExt as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::assets::IconName;
use crate::theme::ThemeColor;
use super::day::rail_segment;
use super::*;
use crate::app::row_parts;
use crate::app::tile::{TileSize, TileSpec, sender_tile};
use crate::threads;

/// Width of the count node on the rail for its label: 15px for one digit, wider for two digits
/// and `99+` (the mockup grows the pill with its text).
fn node_width(label: &str, small: bool) -> f32 {
    if small {
        22.
    } else if label.len() > 1 {
        20.
    } else {
        15.
    }
}

/// A 16px chip on the title line: `N new` (primary) or the message count (muted).
fn count_chip(text: String, color: Hsla, t: &ThemeColor) -> Div {
    div()
        .h(px(16.))
        .px(px(6.))
        .rounded(px(4.))
        .flex_none()
        .flex()
        .items_center()
        .bg(t.accent)
        .text_size(px(11.))
        .text_color(color)
        .child(text)
}

/// The background of a child row at rest: darker than the list, so children read as nested.
fn child_bg(v: RowVisual, t: &ThemeColor) -> Hsla {
    if v.cursor || v.selected || v.open {
        row::row_bg(v, t)
    } else {
        t.background.blend(black().opacity(0.1))
    }
}

/// One 1px curve of the rail detour, `width` x 13px, drawn with its top-left at the element's
/// origin: `down` bends out to the right (`M.5 0 C.5 7 14.5 6 14.5 13`), otherwise back in.
fn detour_curve(down: bool, color: Hsla) -> Canvas<()> {
    canvas(
        |_, _, _| (),
        move |bounds, (), window, _| {
            let at = |x: f32, y: f32| point(bounds.origin.x + px(x), bounds.origin.y + px(y));
            let (from, c1, c2, to) = if down {
                (at(0.5, 0.), at(0.5, 7.), at(14.5, 6.), at(14.5, 13.))
            } else {
                (at(14.5, 0.), at(14.5, 7.), at(0.5, 6.), at(0.5, 13.))
            };
            let mut path = PathBuilder::stroke(px(1.));
            path.move_to(from);
            path.cubic_bezier_to(to, c1, c2);
            if let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
        },
    )
    .w(px(15.))
    .h(px(13.))
}

/// The ring marker of a read child: the 7px marker with its hole in the child background.
fn child_marker(v: RowVisual, account: Option<Hsla>, t: &ThemeColor) -> Div {
    if v.selected || v.unread {
        return row_parts::marker(v, account, t).ml(px(14.));
    }
    let hover = t.list_hover;
    let color = account.unwrap_or(t.muted_foreground);
    let hole = div()
        .absolute()
        .left(px(1.5))
        .top(px(1.5))
        .size(px(4.))
        .rounded_full()
        .bg(child_bg(v, t))
        .when(row::hover_bg(v, t).is_some(), |d| d.group_hover(row_parts::ROW_GROUP, move |s| s.bg(hover)));
    div()
        .w(px(7.))
        .h(px(row::LINE_H))
        .flex_none()
        .relative()
        .ml(px(14.))
        .child(div().absolute().left_0().top(px(5.5)).size(px(7.)).rounded_full().bg(color).child(hole))
}

impl MailApp {
    fn unread_in(&self, id: MessageId) -> bool {
        !self.mailbox.is_read(id) && self.mailbox.state_of(id) == Some(TriageState::Inbox)
    }

    /// The people line of a header: `names` joined, bold, then the muted `+N`, then the newest
    /// preview unless expanded. `lead` replaces the names (sender-first rows lead with them
    /// on line 1 and put the subject here).
    fn people_block(
        &self,
        lead: &str,
        extra: usize,
        snippet: &str,
        lines: u8,
        emphasis: bool,
        t: &ThemeColor,
    ) -> Div {
        let mut text = lead.to_owned();
        let bold = 0..text.len();
        if extra > 0 {
            text.push_str(&format!(" +{extra}"));
        }
        if !snippet.is_empty() {
            text.push_str(" · ");
            text.push_str(snippet);
        }
        let weight = if emphasis { FontWeight::SEMIBOLD } else { FontWeight::NORMAL };
        let style = HighlightStyle { color: Some(t.foreground), font_weight: Some(weight), ..Default::default() };
        div()
            .line_clamp(lines as usize)
            .text_size(px(12.))
            .line_height(px(row::PREVIEW_LINE_H))
            .text_color(t.muted_foreground)
            .child(StyledText::new(text).with_highlights(Some((bold, style))))
    }

    /// Thread header row: the newest message's time, the thread's count node (rail) or group
    /// tile (tiles), subject, `N new` / message-count chip, icons and the chevron, then the
    /// first senders and the newest preview. It sizes to its content.
    pub(super) fn render_group_header(
        &self,
        row: &Row,
        ix: usize,
        _newest: &str,
        cx: &Context<Self>,
    ) -> crate::app::ui::Observable {
        let Row::Header { thread_id, ids, expanded } = row else {
            unreachable!("render_group_header takes header rows")
        };
        let t = cx.theme();
        let thread = *thread_id as usize;
        let container = div().id(("thread-row", thread));
        let Some(latest) = self.mailbox.get(ids[0]) else {
            return row::frame(container, RowVisual::default(), t);
        };
        let selected_count = ids.iter().filter(|id| self.triage().is_selected(**id)).count();
        let unread_n = ids.iter().filter(|id| self.unread_in(**id)).count();
        let visual = RowVisual {
            cursor: ix == self.cursor_ix(),
            open: self.opened().is_some_and(|id| ids.contains(&id)),
            selected: selected_count == ids.len() && selected_count > 0,
            partial: selected_count > 0 && selected_count < ids.len(),
            unread: unread_n > 0,
        };
        let style = self.row_style;
        let names = threads::senders(ids, |id| self.mailbox.get(id));
        let (people, more) = threads::people(&names);
        let sender_label = row::sender_label(latest);
        let snippet = if *expanded {
            String::new()
        } else {
            crate::preview::snippet(&crate::reading::reader_text(latest))
        };
        let lines = threads::header_preview_lines(self.preview_lines);
        // Tiles lists the people without `+N` while expanded; the rail keeps it.
        let more_shown = if *expanded && style.tiles() { 0 } else { more };
        let (title, block) = if style.subject_first() {
            (
                latest.subject.clone(),
                self.people_block(&people, more_shown, &snippet, lines, visual.unread, t),
            )
        } else {
            let lead = if more > 0 { format!("{people} +{more}") } else { people.clone() };
            (lead, self.people_block(&latest.subject, 0, &snippet, lines, visual.unread, t))
        };
        let account = self.account_slot(latest, t);
        let account_color = account.as_ref().and_then(|_| {
            self.mailbox.account(&latest.account).and_then(|a| crate::theme::parse_color(&a.color))
        });

        let pending = self.mailbox.pending(ids[0]);
        let return_time_shown = matches!(self.location(), Some(Location::Snoozed(_)));
        let glyphs = icons::glyphs_for(&GlyphInputs {
            tags: self.mailbox.tags(ids[0]),
            pending: &pending,
            muted: self.mailbox.is_muted(latest.thread_id),
            new_sender: self.mailbox.is_new_sender(ids[0]),
            snoozed: self.mailbox.snoozed_until(ids[0]).is_some() && !return_time_shown,
            attachment: ids
                .iter()
                .any(|id| self.mailbox.get(*id).is_some_and(|m| !m.attachments.is_empty())),
        });
        let (shown, hidden) = icons::split_overflow(&glyphs, icons::max_icons(self.list_w));
        let folder_hint = self.folder_hint(ids[0]);
        let cluster = self.icon_cluster(ids[0], &shown, &hidden, folder_hint.as_deref(), t, cx);
        let toggle = *thread_id;
        let chevron = div()
            .id(("thread-chevron", thread))
            .flex_none()
            .flex()
            .items_center()
            .text_color(t.muted_foreground)
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.cursor_to(ix);
                this.toggle_thread(toggle);
                window.focus(&this.focus_handle, cx);
                cx.notify();
            }))
            .child(Icon::new(if *expanded { IconName::ChevronDown } else { IconName::ChevronRight }).with_size(px(13.)));
        let icons_group = div().flex_none().flex().items_center().gap(px(4.)).child(cluster).child(chevron);

        let label = (SharedString::from(sender_label), SharedString::from(latest.subject.clone()));
        let container = self.draggable(container, ids, Some(*thread_id), label);
        let frame = row::frame(container, visual, t).child(self.row_selection_target(ix, cx));
        let title = row_parts::title(title, visual.unread, t);
        let click = Self::row_click(ix, cx);

        if style.tiles() {
            let tile = sender_tile(
                TileSpec {
                    id: ("thread-tile", thread).into(),
                    senders: names,
                    style: style.group_tile(),
                    size: TileSize::Row,
                    selected: visual.selected,
                    unread: unread_n,
                    row_bg: row::row_bg(visual, t),
                },
                t,
            );
            let count = ids.len();
            let chip = count_chip(count.to_string(), t.muted_foreground, t)
                .id(("thread-count", thread))
                .tooltip(move |window, cx| {
                    Tooltip::new(format!("{count} messages in this thread")).build(window, cx)
                });
            let line1 = div()
                .h(px(row::LINE_H))
                .flex()
                .items_center()
                .gap(px(6.))
                .min_w_0()
                .when_some(account, |d, a| d.child(a))
                .child(title.min_w_0().truncate())
                .child(chip);
            return frame.child(
                div()
                    .w_full()
                    .min_w_0()
                    .flex()
                    .items_start()
                    .gap(px(10.))
                    .py(px(4.))
                    .child(div().flex_none().mt(px(1.)).child(tile))
                    .child(
                        div()
                            .id(("thread-body", thread))
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .on_click(Self::row_click(ix, cx))
                            .child(line1)
                            .child(block),
                    )
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .flex_col()
                            .items_end()
                            .child(
                                div()
                                    .id(("thread-date", thread))
                                    .h(px(row::LINE_H))
                                    .flex()
                                    .items_center()
                                    .text_size(px(11.))
                                    .text_color(if visual.unread { t.primary } else { t.muted_foreground })
                                    .on_click(click)
                                    .child(self.gutter_label(latest)),
                            )
                            .child(div().h(px(16.)).flex().items_center().child(icons_group)),
                    ),
            );
        }

        // Rail: gutter time | count node | body; the straight rail runs behind the whole header.
        let (label, small) = threads::node_label(ids.len());
        let width = node_width(&label, small);
        let hover = t.list_hover;
        let fill = account_color.unwrap_or(t.primary);
        let ring = account_color.unwrap_or(t.muted_foreground);
        let node = div()
            .absolute()
            .left(px(3.5 - width / 2.))
            .top(px(1.5))
            .w(px(width))
            .h(px(15.))
            .rounded(px(8.))
            .bg(if visual.unread { fill } else { ring })
            .child(
                div()
                    .absolute()
                    .left(px(1.5))
                    .right(px(1.5))
                    .top(px(1.5))
                    .bottom(px(1.5))
                    .rounded(px(6.5))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(if visual.unread { fill } else { row::row_bg(visual, t) })
                    .when(!visual.unread && row::hover_bg(visual, t).is_some(), |d| {
                        d.group_hover(row_parts::ROW_GROUP, move |s| s.bg(hover))
                    })
                    .text_size(px(if small { 8. } else { 9. }))
                    .line_height(px(9.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(if visual.unread { t.background } else { t.muted_foreground })
                    .child(label),
            );
        let mark = div().w(px(7.)).h(px(row::LINE_H)).flex_none().relative().child(node);
        let trailing = div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(4.))
            .when(unread_n > 0, |d| d.child(count_chip(format!("{unread_n} new"), t.primary, t)))
            .child(icons_group)
            // The cluster and chevron have their own click targets.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
        frame.child(rail_segment(t)).child(
            div()
                .w_full()
                .min_w_0()
                .flex()
                .items_start()
                .gap(px(8.))
                .py(px(4.))
                .child(row_parts::gutter(self.gutter_label(latest), visual.unread, t))
                .child(mark)
                .child(
                    div()
                        .id(("thread-body", thread))
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .on_click(click)
                        .child(row_parts::title_line(account, title, Some(trailing.into_any_element())))
                        .child(block),
                ),
        )
    }

    /// A message under an expanded thread header: sender line (paperclip at the right), the
    /// preview underneath, on a darker background indented under the header. The rail detours
    /// around it; with sender tiles a 24px tile leads instead.
    pub(super) fn render_thread_child(
        &self,
        row: &Row,
        ix: usize,
        _newest: &str,
        cx: &Context<Self>,
    ) -> crate::app::ui::Observable {
        let Row::Child { thread_id, id } = row else {
            unreachable!("render_thread_child takes child rows")
        };
        let t = cx.theme();
        let ix_id = *id as usize;
        let container = div().id(("row", ix_id));
        let Some(msg) = self.mailbox.get(*id) else {
            return row::frame(container, RowVisual::default(), t);
        };
        let mut visual = self.row_visual(*id);
        visual.cursor = ix == self.cursor_ix();
        let style = self.row_style;
        let bg = child_bg(visual, t);
        let sender = row::sender_label(msg);
        let label = (SharedString::from(sender.clone()), SharedString::from(msg.subject.clone()));
        let container = self.draggable(container, &[*id], Some(*thread_id), label);
        let menu = self.context_menu(MenuKind::Message(*id), cx);
        let plain = !visual.cursor && !visual.selected && !visual.open;
        let frame = row::frame(container, visual, t)
            .when(plain, move |d| d.bg(bg))
            .child(self.row_selection_target(ix, cx));

        let clip = (!msg.attachments.is_empty()).then(|| {
            div()
                .id(("thread-child-clip", ix_id))
                .flex_none()
                .child(icons::icon(Glyph::Attachment, t, 13.))
                .tooltip(|window, cx| Tooltip::new(Glyph::Attachment.spec().label).build(window, cx))
                .into_any_element()
        });
        let snippet = if self.preview_lines > 0 {
            crate::preview::snippet(&crate::reading::reader_text(msg))
        } else {
            String::new()
        };
        let block = row_parts::text_block(None, &snippet, self.preview_lines, false, t);
        let title = row_parts::title(sender.clone(), visual.unread, t);
        let click = Self::row_click(ix, cx);

        if style.tiles() {
            let tile = sender_tile(
                TileSpec {
                    id: ("thread-child-tile", ix_id).into(),
                    senders: vec![msg.from_name.clone()],
                    style: style.group_tile(),
                    size: TileSize::Child,
                    selected: visual.selected,
                    unread: usize::from(visual.unread),
                    row_bg: bg,
                },
                t,
            );
            return frame.pl(px(34.)).child(
                div()
                    .w_full()
                    .min_w_0()
                    .flex()
                    .items_start()
                    .gap(px(10.))
                    .py(px(4.))
                    .child(div().flex_none().child(tile))
                    .child(
                        div()
                            .id(("row-body", ix_id))
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .on_click(Self::row_click(ix, cx))
                            .context_menu(menu)
                            .child(row_parts::title_line(None, title, None))
                            .children(block),
                    )
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .flex_col()
                            .items_end()
                            .child(
                                div()
                                    .id(("row-date", ix_id))
                                    .h(px(row::LINE_H))
                                    .flex()
                                    .items_center()
                                    .text_size(px(11.))
                                    .text_color(if visual.unread { t.primary } else { t.muted_foreground })
                                    .on_click(click)
                                    .child(self.gutter_label(msg)),
                            )
                            .child(div().h(px(16.)).flex().items_center().children(clip)),
                    ),
            );
        }

        let (first, last) = threads::child_position(&self.rows(), ix);
        let account_color = self
            .account_slot(msg, t)
            .and_then(|_| self.mailbox.account(&msg.account))
            .and_then(|a| crate::theme::parse_color(&a.color));
        let border = t.border;
        // The detour: a vertical line right of the header's rail, bent out above the first
        // child and back in below the last.
        let vertical = div()
            .absolute()
            .left(px(75.))
            .w(px(1.))
            .top(px(if first { 12. } else { -1. }))
            .bottom(px(if last { 12. } else { -1. }))
            .bg(border);
        frame
            .child(vertical)
            .when(first, |d| d.child(detour_curve(true, border).absolute().left(px(61.)).top(px(-1.))))
            .when(last, |d| d.child(detour_curve(false, border).absolute().left(px(61.)).bottom(px(-1.))))
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .flex()
                    .items_start()
                    .gap(px(8.))
                    .py(px(4.))
                    .child(row_parts::gutter(self.gutter_label(msg), visual.unread, t))
                    .child(child_marker(visual, account_color, t))
                    .child(
                        div()
                            .id(("row-body", ix_id))
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .on_click(click)
                            .context_menu(menu)
                            .child(row_parts::title_line(None, title, clip))
                            .children(block),
                    ),
            )
    }
}
