//! One message surface of the reader: header, recipients, banners, suggestion strip, body and
//! attachments. The opened message and every expanded thread message share it.

use super::super::*;
use super::parts::{stamp, Look, Role};
use crate::judge::QuestionKey;
use crate::reading;
use gpui_kit::assets::IconName;
use gpui_kit::component::text::TextView;

impl MailApp {
    /// Reply and the `⋯` menu of one expanded message. Both act on that message, whichever one
    /// the reader is opened on. Pressing them never counts as a click on the header.
    fn message_buttons(&self, mid: MessageId, cx: &Context<Self>) -> Div {
        let id = mid as usize;
        let kind = MenuKind::Message(mid);
        let selection = theme::active(cx).selection;
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                icon_button(("btn-reply", id), IconName::Reply, "Reply to this message", "r", cx)
                    .on_click(cx.listener(move |this, _, window, cx| this.reply_to(mid, window, cx))),
            )
            .child(
                div()
                    .relative()
                    .flex_none()
                    .child(
                        icon_button(("btn-message-more", id), IconName::Ellipsis, "More actions for this message", "", cx)
                            .when(self.menu_is(kind), |b| b.bg(selection))
                            .on_click(cx.listener(move |this, _, window, cx| this.toggle_menu(kind, window, cx))),
                    )
                    .child(self.anchor_probe(kind)),
            )
    }

    pub(super) fn message_surface(
        &self,
        m: &Message,
        role: Role,
        look: &Look,
        cx: &Context<Self>,
    ) -> AnyElement {
        let t = &look.t;
        let opened = role == Role::Opened;
        let (id, mid) = (m.id as usize, m.id);
        let me = self.mailbox.account(&m.account).map_or("", |a| a.email.as_str());
        let (pos, total) = reading::thread_position(self.mailbox.messages(), m);
        let recipients_open = self.reader.recipients_open(m.thread_id, m.id);

        let head_id: ElementId =
            if opened { ("reader-msg-head", id).into() } else { ("reader-thread-msg", id).into() };
        let top = div()
            .id(head_id)
            .test_support()
            .h(px(22.))
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .when(!opened, |d| {
                d.cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_reader_expanded(mid, cx)))
            })
            .child(look.mono(format!("MSG {pos:02} / {total:02}"), t.text_muted))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .when(!opened, |d| d.child(look.mono("COLLAPSE", t.text_muted)))
                    .child(look.mono(stamp(&m.received), t.text_muted))
                    .child(self.message_buttons(m.id, cx)),
            );

        let subject = if m.subject.trim().is_empty() {
            "(no subject)".to_owned()
        } else {
            m.subject.clone()
        };
        let subject = div()
            .text_size(px(22.))
            .line_height(relative(1.25))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(t.text)
            .child(subject);

        let sender = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .w(px(28.))
                    .h(px(28.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_md()
                    .border_1()
                    .border_color(t.border)
                    .font_family(look.mono.clone())
                    .text_size(px(11.))
                    .text_color(t.text_muted)
                    .child(reading::initials(&m.from_name, &m.from_email)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_baseline()
                    .gap_2()
                    .child(
                        div()
                            .flex_none()
                            .max_w(px(260.))
                            .truncate()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.text)
                            .child(m.from_name.clone()),
                    )
                    .child(
                        look.mono(m.from_email.clone(), t.text_muted).flex_1().min_w_0().truncate(),
                    ),
            );

        let hover = t.text;
        let recipient_toggle = div()
            .id(("reader-recipients", id))
            .test_support()
            .ml(px(36.))
            .flex()
            .flex_none()
            .items_center()
            .h(px(18.))
            .cursor_pointer()
            .font_family(look.mono.clone())
            .text_size(px(11.))
            .text_color(t.text_muted)
            .hover(move |s| s.text_color(hover))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_recipients(mid, cx)))
            .child(format!(
                "{} {}",
                reading::recipient_summary(m, me),
                if recipients_open { "▴" } else { "▾" }
            ));

        let header = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(top)
            .child(subject)
            .child(sender)
            .child(recipient_toggle)
            .child(self.labels_row(m, look).ml(px(36.)));

        let strip = if opened { self.suggestion_strip(m, look, cx) } else { None };

        div()
            .flex()
            .flex_col()
            .gap_3()
            .min_w_0()
            .p_4()
            .rounded_md()
            .border_1()
            .border_color(t.border)
            .bg(t.surface)
            .child(header)
            .when(recipients_open, |d| d.child(self.recipient_rows(m, look)))
            .when(opened, |d| d.children(self.banners(m, look, cx)))
            .when_some(strip, |d, s| d.child(s))
            .child(self.message_body(m, role, look, cx))
            .when(!m.attachments.is_empty(), |d| d.child(Self::attachment_chips(m, look)))
            .into_any_element()
    }

    /// State badge (with snooze wake time or folder name), tag badges, category and sender label.
    fn labels_row(&self, m: &Message, look: &Look) -> Div {
        let t = &look.t;
        let state_text = match m.state {
            TriageState::Snoozed => match self.mailbox.snoozed_until(m.id) {
                Some(until) => format!("Snoozed · {}", super::format_when(until)),
                None => "Snoozed".to_owned(),
            },
            TriageState::Filed(folder) => format!(
                "Filed · {}",
                self.mailbox.folder(folder).map_or("folder", |f| f.name.as_str())
            ),
            other => other.label().to_owned(),
        };
        let mut row = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1()
            .child(look.badge(state_text, t.state_color(m.state), None));
        for tag in self.mailbox.tags(m.id) {
            let inputs = icons::GlyphInputs {
                tags: std::slice::from_ref(tag),
                pending: &[],
                muted: false,
                new_sender: false,
                snoozed: false,
                attachment: false,
            };
            let Some(&glyph) = icons::glyphs_for(&inputs).first() else { continue };
            let label = match tag {
                Tag::Urgent(n) => format!("Urgent {n}"),
                Tag::Kind(Kind::Other) => continue,
                _ => glyph.spec().label.to_owned(),
            };
            row = row.child(look.badge(label, glyph.spec().token.color(t), Some(glyph)));
        }
        if !m.outgoing && self.mailbox.is_new_sender(m.id) {
            let spec = Glyph::NewSender.spec();
            row = row.child(look.badge(spec.label, spec.token.color(t), Some(Glyph::NewSender)));
        }
        row
    }

    /// `To:` / `Cc:` / `Bcc:` rows, empty ones omitted.
    fn recipient_rows(&self, m: &Message, look: &Look) -> Div {
        let t = &look.t;
        let fields = [("To:", &m.to), ("Cc:", &m.cc), ("Bcc:", &m.bcc)];
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(div().h(px(1.)).bg(t.border))
            .children(fields.into_iter().filter_map(|(label, field)| {
                let list = reading::parse_recipients(field);
                if list.is_empty() {
                    return None;
                }
                let text = list
                    .iter()
                    .map(|r| match &r.name {
                        Some(name) => format!("{name} <{}>", r.email),
                        None => r.email.clone(),
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                Some(
                    div()
                        .flex()
                        .items_start()
                        .gap_2()
                        .child(look.mono(label, t.text_muted).w(px(32.)).flex_none())
                        .child(div().flex_1().min_w_0().text_size(px(12.)).child(text)),
                )
            }))
    }

    /// New-sender and possible-spam banners, with their action buttons.
    fn banners(&self, m: &Message, look: &Look, cx: &Context<Self>) -> Vec<AnyElement> {
        let t = &look.t;
        let banner = |id: &'static str, color: Hsla| {
            div()
                .id(id)
                .test_support()
                .flex_none()
                .flex()
                .items_center()
                .gap_3()
                .px_3()
                .py_2()
                .rounded_sm()
                .bg(color.opacity(0.12))
                .border_1()
                .border_color(color.opacity(0.5))
        };
        let mut out = Vec::new();
        if !m.outgoing && self.mailbox.is_new_sender(m.id) {
            out.push(
                banner("banner-new-sender", t.new_sender)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(12.))
                            .child(format!("New sender · {} <{}>", m.from_name, m.from_email)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap_2()
                            .child(look.action_button(
                                "btn-banner-allow",
                                "Allow",
                                "Allow this sender",
                                "a",
                                AllowSender,
                                cx,
                            ))
                            .child(look.action_button(
                                "btn-banner-block",
                                "Block",
                                "Block this sender",
                                "b",
                                BlockSender,
                                cx,
                            )),
                    )
                    .into_any_element(),
            );
        }
        if self.mailbox.tags(m.id).contains(&Tag::PossibleSpam) {
            out.push(
                banner("banner-possible-spam", t.possible_spam)
                    .child(div().flex_1().min_w_0().text_size(px(12.)).child("Possible spam"))
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap_2()
                            .child(look.action_button(
                                "btn-banner-spam-block",
                                "Block & Delete",
                                "Block the sender and delete this message",
                                "",
                                SpamBlock,
                                cx,
                            ))
                            .child(look.action_button(
                                "btn-banner-spam-delete",
                                "Delete",
                                "Delete this message",
                                "d",
                                Delete,
                                cx,
                            )),
                    )
                    .into_any_element(),
            );
        }
        out
    }

    /// `Suggested: Needs reply 87% · Newsletter 92%` with Accept / Reject buttons (`y` / `n`).
    /// `None` when nothing is pending.
    fn suggestion_strip(&self, m: &Message, look: &Look, cx: &Context<Self>) -> Option<AnyElement> {
        let t = &look.t;
        let pending = self.mailbox.pending(m.id);
        if pending.is_empty() {
            return None;
        }
        let parts: Vec<String> = pending
            .iter()
            .map(|s| {
                let label = match s.key {
                    QuestionKey::Spam => "Possible spam".to_owned(),
                    QuestionKey::NeedsReply => "Needs reply".to_owned(),
                    QuestionKey::ExpectsReply => "Expects reply".to_owned(),
                    QuestionKey::Urgency => {
                        s.urgency().map_or("Urgent".to_owned(), |n| format!("Urgent {n}"))
                    }
                    QuestionKey::Kind => s.kind().map_or("Kind".to_owned(), |k| {
                        Glyph::of_kind(k).spec().label.to_owned()
                    }),
                };
                format!("{label} {}%", (s.answer.confidence * 100.).round() as u32)
            })
            .collect();
        Some(
            div()
                .id("reader-suggestions")
                .test_support()
                .flex_none()
                .flex()
                .items_center()
                .gap_3()
                .px_3()
                .py_2()
                .rounded_sm()
                .border_1()
                .border_color(t.accent.opacity(0.4))
                .bg(t.accent.opacity(0.08))
                .child(icons::icon(Glyph::Suggestion, t, 12.))
                .child(
                    look.mono(format!("Suggested: {}", parts.join(" · ")), t.text)
                        .flex_1()
                        .min_w_0(),
                )
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap_2()
                        .child(look.action_button(
                            "btn-suggest-accept",
                            "Accept",
                            "Accept the suggestions",
                            "y",
                            AcceptSuggestions,
                            cx,
                        ))
                        .child(look.action_button(
                            "btn-suggest-reject",
                            "Reject",
                            "Reject the suggestions",
                            "n",
                            RejectSuggestions,
                            cx,
                        )),
                )
                .into_any_element(),
        )
    }

    /// The body: HTML (remote images blocked) unless Reader mode is on, else text with the quoted
    /// history folded. Messages with an HTML part carry the per-message Reader mode toggle.
    fn message_body(
        &self,
        m: &Message,
        role: Role,
        look: &Look,
        cx: &Context<Self>,
    ) -> AnyElement {
        let t = &look.t;
        let opened = role == Role::Opened;
        let (id, mid) = (m.id as usize, m.id);
        let body_id: ElementId =
            if opened { "reader-body".into() } else { ("reader-thread-body", id).into() };
        let html = m.html.as_deref().filter(|h| !h.trim().is_empty());
        let plain = self.reader.plain(m.thread_id, m.id);
        let mut body = div()
            .id(body_id)
            .test_support()
            .flex()
            .flex_col()
            .gap_2()
            .min_w_0()
            .text_size(px(13.5))
            .line_height(relative(1.55))
            .text_color(t.text);

        let mut blocked = 0;
        let content = match html {
            Some(h) if !plain => {
                let safe = reading::safe_html(h);
                blocked = safe.blocked_images;
                div()
                    .w_full()
                    .min_w_0()
                    .child(TextView::html(("reader-html", id), safe.html))
                    .into_any_element()
            }
            _ => self.text_content(m, look, cx),
        };

        if html.is_some() {
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(if blocked > 0 {
                        look.mono(
                            format!(
                                "{blocked} REMOTE IMAGE{} BLOCKED",
                                if blocked == 1 { "" } else { "S" }
                            ),
                            t.text_muted,
                        )
                    } else {
                        div()
                    })
                    .child(
                        look.link(
                            ("reader-mode", id),
                            if plain { "SHOW ORIGINAL" } else { "READER MODE" },
                            opened.then_some("v"),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| this.toggle_reader_mode(mid, cx))),
                    ),
            );
        }
        body.child(content).into_any_element()
    }

    /// Plain-text body: the text part (or text extracted from the HTML), quoted history folded
    /// under `SHOW QUOTED TEXT`.
    fn text_content(&self, m: &Message, look: &Look, cx: &Context<Self>) -> AnyElement {
        let t = &look.t;
        let (id, mid) = (m.id as usize, m.id);
        let text = reading::reader_text(m);
        let split = reading::split_quoted(&text);
        let quoted_open = self.reader.quoted_open(m.thread_id, m.id);
        let main = if split.main.trim().is_empty() {
            div().text_color(t.text_muted).child("(no text)")
        } else {
            div().child(split.main.to_owned())
        };
        div()
            .flex()
            .flex_col()
            .gap_2()
            .min_w_0()
            .child(main)
            .when_some(split.quoted, |d, quoted| {
                d.child(
                    look.link(
                        ("reader-quoted", id),
                        if quoted_open { "HIDE QUOTED TEXT" } else { "SHOW QUOTED TEXT" },
                        None,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_quoted(mid, cx))),
                )
                .when(quoted_open, |d| {
                    d.child(
                        div()
                            .pl_3()
                            .border_l_2()
                            .border_color(t.border)
                            .text_color(t.text_muted)
                            .child(quoted.to_owned()),
                    )
                })
            })
            .into_any_element()
    }

    /// Attachment chips: paperclip, file name, mono size.
    fn attachment_chips(m: &Message, look: &Look) -> Div {
        let t = &look.t;
        div().flex().flex_wrap().gap_2().children(m.attachments.iter().map(|a| {
            div()
                .flex()
                .items_center()
                .gap_2()
                .h(px(24.))
                .px_2()
                .rounded_sm()
                .border_1()
                .border_color(t.border)
                .child(icons::icon(Glyph::Attachment, t, 12.))
                .child(div().text_size(px(12.)).child(a.name.clone()))
                .child(look.mono(reading::format_size(a.size), t.text_muted))
        }))
    }
}
