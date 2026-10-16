//! One message surface of the reader: header, recipients, banners, suggestion strip, body and
//! attachments. The opened message and every expanded thread message share it.

use super::super::*;
use super::parts::{selectable_verbatim, stamp, Look, Role};
use crate::app::chrome::humanize_time;
use crate::app::email_webview::{luminance, rgb, DocStyle, ViewKey};
use crate::app::menu::MenuItem;
use crate::find::Segment;
use crate::judge::{AnswerValue, QuestionKey, Routed, Suggestion};
use crate::reading;
use crate::theme::ThemeColor;
use gpui_kit::assets::IconName;
use gpui_kit::base::{SelectableText, TextSelection};
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::collapsible::Collapsible;
use gpui_kit::component::description_list::DescriptionList;
use gpui_kit::component::menu::DropdownMenu as _;
use gpui_kit::component::separator::Separator;
use gpui_kit::component::tooltip::Tooltip;

impl MailApp {
    /// Reply, Reply all, Forward and the `⋯` menu of one expanded message. All act on that
    /// message, whichever one the reader is opened on. Pressing them never counts as a click on
    /// the header.
    fn message_buttons(&self, mid: MessageId, cx: &Context<Self>) -> Div {
        let id = mid as usize;
        let kind = MenuKind::Message(mid);
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
                icon_button(("btn-reply-all", id), IconName::ReplyAll, "Reply to everyone", "shift-a", cx)
                    .on_click(cx.listener(move |this, _, window, cx| this.reply_all_to(mid, window, cx))),
            )
            .child(
                icon_button(("btn-forward", id), IconName::Forward, "Forward this message", "w", cx)
                    .on_click(cx.listener(move |this, _, window, cx| this.forward(mid, window, cx))),
            )
            .child(div().flex_none().child(self.menu_trigger(
                kind,
                icon_button(("btn-message-more", id), IconName::Ellipsis, "More actions for this message", "", cx),
                cx,
            )))
    }

    pub(super) fn message_surface(
        &self,
        m: &Message,
        role: Role,
        look: &Look<'_>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let t = &look.t;
        let opened = role == Role::Opened;
        let (id, mid) = (m.id as usize, m.id);
        let me = self.mailbox.account(&m.account).map_or("", |a| a.email.as_str());
        let (pos, total) = reading::thread_position(self.mailbox.messages(), m);
        let recipients_open = self.reader.recipients_open(m.thread_id, m.id);

        let head_id: ElementId =
            if opened { ("reader-msg-head", id).into() } else { ("reader-thread-head", id).into() };
        let cluster = div()
            .h(px(22.))
            .flex()
            .flex_none()
            .items_center()
            .gap_3()
            .when(total > 1, |d| {
                d.child(look.mono_selectable(
                    ("reader-sel-msgno", id),
                    format!("MSG {pos:02} / {total:02}"),
                    t.muted_foreground,
                ))
            })
            .when(!opened, |d| d.child(look.mono("COLLAPSE", t.muted_foreground)))
            .child({
                let tip = m
                    .received_at()
                    .map_or_else(|| stamp(&m.received), |ts| humanize_time(ts, &self.local_now()));
                div()
                    .id(("reader-msg-stamp", id))
                    .text_size(px(13.))
                    .text_color(t.muted_foreground)
                    .child(SelectableText::new(("reader-sel-stamp", id), reading::received_label(&m.received, &self.local_now())))
                    .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            })
            .when_some(self.classification_state(m.id), |d, state| {
                d.child(icons::classification_badge(("reader-classify", id), state, t, 13.))
            })
            .when(opened && !m.outgoing, |d| {
                d.child(
                    icon_button(("btn-classify", id), IconName::ScanSearch, "Classify this message", "", cx)
                        .on_click(cx.listener(move |this, _, _, cx| this.classify_message(mid, cx))),
                )
            })
            .child(self.message_buttons(m.id, cx));

        let subject = reading::display_subject(m);
        let subject = if opened {
            self.find_text(m.thread_id, m.id, Segment::Subject, &subject, cx)
        } else {
            subject.into_any_element()
        };
        let subject = div()
            .text_size(px(22.))
            .line_height(relative(1.25))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(t.foreground)
            .child(subject);

        let hover = t.foreground;
        let recipient_toggle = div()
            .id(("reader-recipients", id))
            .test_support()
            .self_start()
            .max_w_full()
            .flex()
            .items_center()
            .h(px(18.))
            .cursor_pointer()
            .text_size(px(13.))
            .text_color(t.muted_foreground)
            .hover(move |s| s.text_color(hover))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, window, cx| {
                if TextSelection::selected_text(window, cx).is_empty() {
                    this.toggle_recipients(mid, cx);
                }
            }))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .child(SelectableText::new(("reader-sel-recipients", id), reading::recipient_line(m, me))),
            )
            .child(div().flex_none().pl_1().child(if recipients_open { "▴" } else { "▾" }));

        let sender = div()
            .id(head_id)
            .test_support()
            .flex()
            .items_start()
            .gap_3()
            .when(!opened, |d| {
                d.cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        // A drag that selected text is not a click on the header.
                        if TextSelection::selected_text(window, cx).is_empty() {
                            this.toggle_reader_expanded(mid, cx);
                        }
                    }))
            })
            .child(look.monogram(&m.from_name, &m.from_email, 40.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(
                        div()
                            .h(px(22.))
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex_none()
                                    .max_w(px(260.))
                                    .truncate()
                                    .text_size(px(15.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(t.foreground)
                                    .child(SelectableText::new(("reader-sel-name", id), m.from_name.clone())),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(13.))
                                    .text_color(t.muted_foreground)
                                    .child(SelectableText::new(("reader-sel-email", id), m.from_email.clone())),
                            ),
                    )
                    .child(recipient_toggle),
            )
            .child(cluster);

        let header = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(subject)
            .child(sender)
            .child(self.labels_row(m, look, cx).ml(px(52.)));

        let strip = if opened { self.folder_strip(m, look, cx) } else { None };
        let dev = (opened && cfg!(debug_assertions)).then(|| self.dev_answers(m, look)).flatten();

        div()
            .flex()
            .flex_col()
            .gap_3()
            .min_w_0()
            .p_4()
            .rounded_md()
            .border_1()
            .border_color(t.border)
            .bg(message_card_bg(t))
            .child(header)
            .when(recipients_open, |d| d.child(self.recipient_rows(m)))
            .when(opened, |d| d.children(self.banners(m, look, cx)))
            .when_some(strip, |d, s| d.child(s))
            .when_some(dev, |d, el| d.child(el))
            .child(self.message_body(m, role, look, cx))
            .when(!m.attachments.is_empty(), |d| d.child(Self::attachment_chips(m, look)))
            // A message synced from headers only still holds just its snippet.
            .when(m.partial, |d| {
                d.child(
                    div()
                        .text_size(px(12.))
                        .text_color(t.muted_foreground)
                        .child("Loading message…"),
                )
            })
            .into_any_element()
    }

    /// State badge (with snooze wake time or folder name), tag badges, category and sender label.
    /// The Kind, Urgency and Spam chips are clickable and open their own override menu.
    fn labels_row(&self, m: &Message, look: &Look<'_>, cx: &Context<Self>) -> Div {
        let t = &look.t;
        let state_text = match m.state {
            TriageState::Snoozed => match self.mailbox.snoozed_until(m.id) {
                Some(until) => format!("Snoozed · {}", super::format_when(until, &self.local_now())),
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
            .child(look.badge(state_text, theme::state_color(t, m.state), None));
        for tag in self.mailbox.tags(m.id) {
            let (label, glyph, key, items) = match tag {
                Tag::Urgent(level) => (
                    urgency_word(*level),
                    urgency_glyph(*level),
                    "reader-chip-urgency",
                    self.urgency_items(m.id),
                ),
                Tag::Kind(kind) => (
                    Glyph::of_kind(*kind).spec().label,
                    Glyph::of_kind(*kind),
                    "reader-chip-kind",
                    self.kind_items(m.id),
                ),
                Tag::PossibleSpam => (
                    "Possible spam",
                    Glyph::PossibleSpam,
                    "reader-chip-spam",
                    self.spam_items(m.id),
                ),
                Tag::NeedsReply => (
                    "Needs reply",
                    Glyph::NeedsReply,
                    "reader-chip-needs-reply",
                    self.needs_reply_items(m.id),
                ),
                Tag::AwaitingReply => (
                    "Awaiting reply",
                    Glyph::AwaitingReply,
                    "reader-chip-awaiting-reply",
                    Vec::new(),
                ),
                Tag::FollowUp => ("Follow up", Glyph::FollowUp, "reader-chip-follow-up", Vec::new()),
                Tag::Reminder => ("Reminder", Glyph::Reminder, "reader-chip-reminder", Vec::new()),
            };
            row = row.child(self.chip(m.id, (key, label, glyph), items, look, cx));
        }
        if !m.outgoing && self.mailbox.is_new_sender(m.id) {
            let spec = Glyph::NewSender.spec();
            row = row.child(look.badge(spec.label, spec.token.color(t), Some(Glyph::NewSender)));
        }
        row
    }

    /// One labels-row chip, drawn from its `(id key, word, glyph)`. With `items` it is a ghost
    /// button that opens that section's override menu under the chip; without them it is a plain
    /// badge (the reply-status chips).
    fn chip(
        &self,
        mid: MessageId,
        (key, label, glyph): (&'static str, &'static str, Glyph),
        items: Vec<MenuItem>,
        look: &Look<'_>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let badge = look.badge(label, glyph.spec().token.color(look.t), Some(glyph));
        let wrapper = div().id((key, u64::from(mid))).test_support().aria_label(label);
        if items.is_empty() {
            return wrapper.child(badge).into_any_element();
        }
        let owner = cx.weak_entity();
        wrapper
            .child(
                Button::new(SharedString::from(format!("chip-menu-{key}-{mid}")))
                    .ghost()
                    .h(px(18.))
                    .px_0()
                    .child(badge)
                    .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, window, cx| {
                        MailApp::populate_items(owner.clone(), items.clone(), Some(vec![mid]), menu, window, cx)
                    }),
            )
            .into_any_element()
    }

    /// `To:` / `Cc:` / `Bcc:` rows, empty ones omitted.
    fn recipient_rows(&self, m: &Message) -> Div {
        let fields = [("To:", &m.to), ("Cc:", &m.cc), ("Bcc:", &m.bcc)];
        let mut list = DescriptionList::horizontal().columns(1).label_width(px(32.)).bordered(false);
        for (row, (label, field)) in fields.into_iter().enumerate() {
            let row = row as u64;
            let recipients = reading::parse_recipients(field);
            if recipients.is_empty() {
                continue;
            }
            let text = recipients
                .iter()
                .map(|r| match &r.name {
                    Some(name) => format!("{name} <{}>", r.email),
                    None => r.email.clone(),
                })
                .collect::<Vec<_>>()
                .join(", ");
            let value = selectable_verbatim(("reader-sel-recipient-row", u64::from(m.id) * 4 + row), &text);
            list = list.item(label, value.into_any_element(), 1);
        }
        div().flex().flex_col().gap_1().child(Separator::horizontal()).child(list)
    }

    /// New-sender and possible-spam banners, with their action buttons.
    fn banners(&self, m: &Message, look: &Look<'_>, cx: &Context<Self>) -> Vec<AnyElement> {
        let mut out = Vec::new();
        if !m.outgoing && self.mailbox.is_new_sender(m.id) {
            let actions = div()
                .flex()
                .flex_none()
                .items_center()
                .gap_2()
                .child(look.action_button("btn-banner-block", "Block", "Block this sender", "b", BlockSender, cx).ghost())
                .child(look.action_button("btn-banner-allow", "Allow", "Allow this sender", "a", AllowSender, cx).primary());
            out.push(suggestion_banner(
                "banner-new-sender",
                Glyph::NewSender.spec().icon,
                "New sender",
                selectable_verbatim(("reader-sel-new-sender", u64::from(m.id)), &format!("{} <{}>", m.from_name, m.from_email)).into_any_element(),
                look.t.info,
                look.t,
                actions,
            ));
        }
        if self.mailbox.tags(m.id).contains(&Tag::PossibleSpam) {
            out.push(self.spam_banner(m.id, look, cx));
        }
        out
    }

    /// Accepting a spam suggestion uses the danger variant on Delete; other actions stay quiet.
    fn spam_banner(&self, id: MessageId, look: &Look<'_>, cx: &Context<Self>) -> AnyElement {
        let actions = div()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .child(look.action_button("btn-banner-spam-not", "Not spam", "Clear the Possible spam flag", "", SetSpam { message: Some(id), spam: false }, cx).ghost())
            .child(
                look.action_button("btn-banner-spam-block", "Block & Delete", "Block the sender and delete this message", "", SpamBlock, cx)
                    .ghost()
                    .ml_2(),
            )
            .child(look.action_button("btn-banner-spam-delete", "Delete", "Delete this message", "d", Delete, cx).danger());
        suggestion_banner(
            "banner-possible-spam",
            Glyph::PossibleSpam.spec().icon,
            "Possible spam",
            div().into_any_element(),
            look.t.danger,
            look.t,
            actions,
        )
    }

    /// The pending Folder suggestion: `Move` (`y`) accepts and `Dismiss` (`n`) rejects.
    fn folder_strip(&self, m: &Message, look: &Look<'_>, cx: &Context<Self>) -> Option<AnyElement> {
        let s = self.mailbox.pending(m.id).into_iter().find(|s| s.key == QuestionKey::Folder)?;
        let folder = self.mailbox.suggested_folder(s)?;
        let actions = div()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .child(look.action_button("btn-folder-dismiss", "Dismiss", "Dismiss the suggested folder", "n", RejectSuggestions, cx).ghost())
            .child(look.action_button("btn-folder-move", "Move", "Move the message to the suggested folder", "y", AcceptSuggestions, cx).primary());
        Some(suggestion_banner(
            "reader-folder-suggestion",
            Glyph::FolderSuggestion.spec().icon,
            "Suggested folder",
            div().min_w_0().truncate().text_size(px(13.)).text_color(look.t.foreground).child(self.mailbox.folder_path(folder)).into_any_element(),
            look.t.info,
            look.t,
            actions,
        ))
    }

    /// Debug-only: every stored classifier answer of `m`, with its version, value, confidence,
    /// probabilities and the routing the current policy gives it. `None` when nothing is cached.
    fn dev_answers(&self, m: &Message, look: &Look<'_>) -> Option<AnyElement> {
        let answers = self.cached_answers(m.id)?;
        let t = &look.t;
        let mut table = div()
            .id("reader-dev-answers")
            .test_support()
            .flex()
            .flex_col()
            .gap_1()
            .px_3()
            .py_2()
            .rounded_md()
            .border_1()
            .border_color(t.border)
            .font_family(look.mono.clone())
            .text_size(px(11.5))
            .text_color(t.foreground);
        // Fixed column widths keep the rows aligned like a table.
        let row = |cells: [AnyElement; 5]| {
            let [q, value, conf, route, probs] = cells;
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(div().w(px(120.)).flex_none().child(q))
                .child(div().w(px(150.)).flex_none().truncate().child(value))
                .child(div().w(px(48.)).flex_none().text_right().child(conf))
                .child(div().w(px(64.)).flex_none().child(route))
                .child(div().flex_1().min_w_0().truncate().child(probs))
        };
        let muted = |s: &str| div().text_color(t.muted_foreground).child(s.to_owned()).into_any_element();
        table = table
            .child(div().text_xs().text_color(t.muted_foreground).child("Classifier answers · dev"))
            .child(row(["Question", "Answer", "Conf", "Routing", "Probabilities"].map(muted)));
        for a in answers {
            let routing = match crate::judge::route(&self.policy, m, vec![(a.key, a.answer.clone())])
                .into_iter()
                .next()
            {
                Some(Routed::Auto(_)) => ("Auto", t.success),
                Some(Routed::Review(_)) => ("Review", t.warning),
                _ => ("Drop", t.muted_foreground),
            };
            let value = match &a.answer.value {
                AnswerValue::Bool(b) => if *b { "yes" } else { "no" }.to_owned(),
                AnswerValue::Choice(i) => match a.key {
                    QuestionKey::Kind => Kind::from_index(*i)
                        .map_or_else(|| format!("#{i}"), |k| k.label().to_owned()),
                    QuestionKey::Folder => self
                        .mailbox
                        .suggested_folder(&Suggestion {
                            message: m.id,
                            key: a.key,
                            answer: a.answer.clone(),
                        })
                        .map_or_else(|| "none".to_owned(), |f| self.mailbox.folder_path(f)),
                    _ => format!("#{i}"),
                },
                AnswerValue::Score(s) => format!("{s:.0} / 5"),
            };
            let probs = a
                .answer
                .probabilities
                .iter()
                .map(|p| format!("{p:.2}"))
                .collect::<Vec<_>>()
                .join("  ");
            // Folder's version is a long folder-list fingerprint; its head is enough to compare.
            let version: String = a.version.chars().take(6).collect();
            table = table.child(row([
                div()
                    .flex()
                    .gap_1()
                    .child(a.key.label())
                    .child(div().text_color(t.muted_foreground).child(format!("v{version}")))
                    .into_any_element(),
                div().child(value).into_any_element(),
                div().child(format!("{:.0}%", a.answer.confidence * 100.)).into_any_element(),
                div().text_color(routing.1).child(routing.0).into_any_element(),
                div().text_color(t.muted_foreground).child(probs).into_any_element(),
            ]));
        }
        Some(table.into_any_element())
    }

    /// The body: HTML (remote images blocked only when asked) unless Reader mode is on, else text
    /// with the quoted history folded. Messages with an HTML part carry the per-message Reader mode
    /// toggle.
    fn message_body(
        &self,
        m: &Message,
        role: Role,
        look: &Look<'_>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let t = &look.t;
        let opened = role == Role::Opened;
        let (id, mid) = (m.id as usize, m.id);
        let body_id: ElementId =
            if opened { "reader-body".into() } else { ("reader-thread-body", id).into() };
        let html = m.html.as_deref().filter(|h| !h.trim().is_empty());
        let plain = self.reader.plain(m.thread_id, m.id) || self.find_forces_plain(m);
        let mut body = div()
            .id(body_id)
            .test_support()
            .flex()
            .flex_col()
            .gap_2()
            .min_w_0()
            .text_size(px(13.5))
            .line_height(relative(1.55))
            .text_color(t.foreground);

        let mut blocked = 0;
        let content = match html {
            Some(h) if !plain => {
                // Exactly the card's background, not the window's: the page fills the card, so
                // the loading skeleton, the no-snapshot fallback and the loaded page must all
                // resolve to this one color or the reader flashes as they swap.
                let bg = rgb(message_card_bg(t));
                let style = DocStyle {
                    block: self.block_remote_images,
                    dark: luminance(bg) < 0.5,
                    fg: rgb(t.foreground),
                    bg,
                };
                let key = ViewKey { msg: mid, triage: self.slot() == Slot::Triage };
                let (view, count) = self.webviews.body(key, h, style);
                blocked = count;
                div()
                    .id(("reader-html", id))
                    .test_support()
                    .w_full()
                    .min_w_0()
                    .child(view)
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
                            t.muted_foreground,
                        )
                        .id("reader-images-blocked")
                        .test_support()
                        .into_any_element()
                    } else {
                        div().into_any_element()
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
    fn text_content(&self, m: &Message, look: &Look<'_>, cx: &Context<Self>) -> AnyElement {
        let t = &look.t;
        let (id, mid) = (m.id as usize, m.id);
        let text = reading::reader_text(m);
        let split = reading::split_quoted(&text);
        let quoted_open = self.reader.quoted_open(m.thread_id, m.id);
        let main = if split.main.trim().is_empty() {
            div().text_color(t.muted_foreground).child("(no text)")
        } else {
            div().child(self.find_text(m.thread_id, m.id, Segment::Main, split.main, cx))
        };
        div()
            .flex()
            .flex_col()
            .gap_2()
            .min_w_0()
            .child(main)
            .when_some(split.quoted, |d, quoted| {
                d.child(
                    Collapsible::new()
                        .open(quoted_open)
                        .child(
                            look.link(
                                ("reader-quoted", id),
                                if quoted_open { "HIDE QUOTED TEXT" } else { "SHOW QUOTED TEXT" },
                                None,
                            )
                            .on_click(cx.listener(move |this, _, _, cx| this.toggle_quoted(mid, cx))),
                        )
                        .content(
                            div()
                                .pl_3()
                                .border_l_2()
                                .border_color(t.border)
                                .text_color(t.muted_foreground)
                                .child(self.find_text(m.thread_id, m.id, Segment::Quoted, quoted, cx)),
                        ),
                )
            })
            .into_any_element()
    }

    /// Attachment chips: paperclip, file name, mono size.
    fn attachment_chips(m: &Message, look: &Look<'_>) -> Div {
        let t = &look.t;
        div().flex().flex_wrap().gap_2().children(m.attachments.iter().enumerate().map(|(i, a)| {
            let att_id = u64::from(m.id) * 1024 + i as u64;
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
                .child(
                    div()
                        .text_size(px(12.))
                        .child(SelectableText::new(("reader-sel-attachment", att_id), a.name.clone())),
                )
                .child(look.mono_selectable(
                    ("reader-sel-attachment-size", att_id),
                    reading::format_size(a.size),
                    t.muted_foreground,
                ))
        }))
    }
}

/// Shared frame for reader suggestions; only the icon and subtle tint vary by meaning.
fn suggestion_banner(
    id: &'static str,
    icon: IconName,
    title: &'static str,
    detail: AnyElement,
    tint: Hsla,
    theme: &ThemeColor,
    actions: Div,
) -> AnyElement {
    div()
        .id(id)
        .test_support()
        .flex_none()
        .flex()
        .items_center()
        .gap_3()
        .min_h(px(38.))
        .px_3()
        .py_2()
        .rounded_md()
        .border_1()
        .border_color(tint.opacity(0.22))
        .bg(tint.opacity(0.06))
        .child(Icon::new(icon).with_size(px(14.)).text_color(tint))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap_2()
                .child(div().flex_none().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(theme.foreground).child(title))
                .child(detail),
        )
        .child(actions)
        .into_any_element()
}

/// Background of a message card. The embedded web page, its loading skeleton and its
/// no-snapshot fallback all paint this same color, so the card never shows a seam between
/// them; the token keeps light and dark in step.
fn message_card_bg(t: &ThemeColor) -> Hsla {
    t.secondary
}

/// The reader chip word for urgency `level`; the raw `n/5` is never shown.
fn urgency_word(level: u8) -> &'static str {
    match level {
        2 => "Low",
        3 => "Normal",
        4 => "Urgent",
        5.. => "Very urgent",
        _ => "None",
    }
}

/// The reader chip icon for urgency `level`: Siren/Flame for 4–5, a muted Gauge for 1–3.
fn urgency_glyph(level: u8) -> Glyph {
    match level {
        5.. => Glyph::UrgentHigh,
        4 => Glyph::UrgentMid,
        _ => Glyph::UrgencyLow,
    }
}
