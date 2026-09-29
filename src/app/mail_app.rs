//! Root view: rail + message list + reader, hint bar, help overlay, undo toast,
//! command palette and reply composer.

use std::time::Duration;

use gpui_kit::component::tag::Tag;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::app::actions::*;
use crate::app::chrome::{EmptyState, HelpOverlay, HintBar, HintMode, ViewTabs};
use crate::app::compose::{ComposeEvent, ComposeReply};
use crate::app::palette::{CommandPalette, PaletteEvent};
use crate::model::{Mailbox, Message, MessageId, Triage, TriageState};

const ROW_H: f32 = 26.0;
const TOAST_MS: u64 = 4000;

pub struct MailApp {
    pub mailbox: Mailbox,
    pub triage: Triage,
    /// Message shown in the reader pane.
    pub opened: Option<MessageId>,
    pub palette: Option<Entity<CommandPalette>>,
    pub compose: Option<Entity<ComposeReply>>,
    pub help: bool,
    /// Undo toast text, if visible.
    pub toast: Option<SharedString>,
    toast_gen: u64,
    focus_handle: FocusHandle,
    list_scroll: UniformListScrollHandle,
    _modal_sub: Option<Subscription>,
}

impl MailApp {
    pub fn new(mailbox: Mailbox, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            mailbox,
            triage: Triage::new(TriageState::Inbox),
            opened: None,
            palette: None,
            compose: None,
            help: false,
            toast: None,
            toast_gen: 0,
            focus_handle: cx.focus_handle(),
            list_scroll: UniformListScrollHandle::new(),
            _modal_sub: None,
        }
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus_handle
    }

    pub fn palette_open(&self) -> bool {
        self.palette.is_some()
    }

    pub fn compose_open(&self) -> bool {
        self.compose.is_some()
    }

    pub fn help_open(&self) -> bool {
        self.help
    }

    pub fn opened(&self) -> Option<MessageId> {
        self.opened
    }

    fn modal_open(&self) -> bool {
        self.palette.is_some() || self.compose.is_some()
    }

    fn scroll_to_cursor(&self) {
        self.list_scroll
            .scroll_to_item(self.triage.cursor_index(), ScrollStrategy::Nearest);
    }

    fn show_toast(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.toast_gen += 1;
        let generation = self.toast_gen;
        self.toast = Some(text.into());
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(TOAST_MS))
                .await;
            this.update(cx, |this, cx| {
                if this.toast_gen == generation {
                    this.toast = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn close_modals(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette = None;
        self.compose = None;
        self._modal_sub = None;
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    fn mark(&mut self, state: TriageState, window: &mut Window, cx: &mut Context<Self>) {
        let n = self.triage.apply(&mut self.mailbox, state);
        if n > 0 {
            let msg = match state {
                TriageState::Inbox => format!("Moved {n} to inbox · u to undo"),
                s => format!("Marked {n} {} · u to undo", s.label().to_lowercase()),
            };
            self.show_toast(msg, window, cx);
        }
        self.scroll_to_cursor();
        cx.notify();
    }

    fn mark_sender(&mut self, state: TriageState, window: &mut Window, cx: &mut Context<Self>) {
        let name = self
            .triage
            .cursor(&self.mailbox)
            .and_then(|id| self.mailbox.get(id))
            .map(|m| m.from_name.clone());
        let n = self.triage.apply_to_sender(&mut self.mailbox, state);
        if let (n @ 1.., Some(name)) = (n, name) {
            let msg = format!(
                "Marked {n} from {name} {} · u to undo",
                state.label().to_lowercase()
            );
            self.show_toast(msg, window, cx);
        }
        self.scroll_to_cursor();
        cx.notify();
    }

    fn show_view(&mut self, view: TriageState, cx: &mut Context<Self>) {
        self.triage.switch_view(view);
        self.opened = None;
        self.scroll_to_cursor();
        cx.notify();
    }

    fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let palette = cx.new(|cx| CommandPalette::new(window, cx));
        self._modal_sub = Some(cx.subscribe_in(
            &palette,
            window,
            |this, _, event: &PaletteEvent, window, cx| {
                this.close_modals(window, cx);
                if let PaletteEvent::Run(action) = event {
                    window.dispatch_action(action.boxed_clone(), cx);
                }
            },
        ));
        window.focus(&palette.focus_handle(cx), cx);
        self.palette = Some(palette);
        cx.notify();
    }

    fn open_compose(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(msg) = self
            .triage
            .cursor(&self.mailbox)
            .and_then(|id| self.mailbox.get(id))
            .cloned()
        else {
            return;
        };
        self.opened = Some(msg.id);
        let compose = cx.new(|cx| ComposeReply::new(&msg, window, cx));
        self._modal_sub = Some(cx.subscribe_in(
            &compose,
            window,
            |this, _, event: &ComposeEvent, window, cx| {
                match event {
                    ComposeEvent::Send { in_reply_to, body } => {
                        this.mailbox.send_reply(*in_reply_to, body.clone());
                        this.close_modals(window, cx);
                        this.show_toast(
                            "Reply sent · moved to waiting · u to undo".into(),
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

    // ---- rendering helpers ----

    fn clock_label(received: &str, newest: &str) -> String {
        const MONTHS: [&str; 12] = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];
        let date = received.get(..10).unwrap_or(received);
        if date == newest.get(..10).unwrap_or(newest) {
            return received.get(11..16).unwrap_or("").to_string();
        }
        let month = date
            .get(5..7)
            .and_then(|m| m.parse::<usize>().ok())
            .and_then(|m| MONTHS.get(m.wrapping_sub(1)))
            .unwrap_or(&"?");
        let day = date.get(8..10).and_then(|d| d.parse::<u32>().ok()).unwrap_or(0);
        format!("{month} {day}")
    }

    fn newest(&self) -> String {
        self.mailbox
            .messages()
            .iter()
            .map(|m| m.received.as_str())
            .max()
            .unwrap_or("")
            .to_string()
    }

    fn render_row(&self, msg: &Message, ix: usize, newest: &str, cx: &App) -> Stateful<Div> {
        let t = cx.theme();
        let is_cursor = ix == self.triage.cursor_index();
        let selected = self.triage.is_selected(msg.id);
        let date = Self::clock_label(&msg.received, newest);
        div()
            .id(("row", msg.id as usize))
            .h(px(ROW_H))
            .w_full()
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .text_size(px(13.))
            .border_l_2()
            .border_color(if is_cursor { t.primary } else { t.transparent })
            .when(selected, |d| d.bg(t.primary.opacity(0.16)))
            .when(is_cursor && !selected, |d| d.bg(t.list_active))
            .child(
                div()
                    .w(px(10.))
                    .text_color(t.primary)
                    .child(if selected { "●" } else { "" }),
            )
            .child(
                div()
                    .w(px(120.))
                    .flex_none()
                    .truncate()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(msg.from_name.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .truncate()
                    .text_color(t.muted_foreground)
                    .child(msg.subject.clone()),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(px(11.))
                    .text_color(t.muted_foreground)
                    .child(date),
            )
    }

    fn render_list(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme();
        let view = self.triage.view;
        let count = self.mailbox.count(view);
        let selected = self.triage.selected().len();
        let header = div()
            .h(px(28.))
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .px_3()
            .text_size(px(11.))
            .text_color(t.muted_foreground)
            .child(format!("{} · {count}", view.label().to_uppercase()))
            .when(selected > 0, |d| {
                d.child(
                    div()
                        .text_color(t.primary)
                        .child(format!("{selected} selected")),
                )
            });
        let body = if count == 0 {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .child(EmptyState::new(view))
                .into_any_element()
        } else {
            uniform_list(
                "messages",
                count,
                cx.processor(|this, range: std::ops::Range<usize>, _window, cx| {
                    let ids = this.mailbox.ids_in(this.triage.view);
                    let newest = this.newest();
                    let mut rows = Vec::with_capacity(range.len());
                    for ix in range {
                        if let Some(msg) = ids.get(ix).and_then(|id| this.mailbox.get(*id)) {
                            rows.push(this.render_row(msg, ix, &newest, cx));
                        }
                    }
                    rows
                }),
            )
            .track_scroll(&self.list_scroll)
            .flex_1()
            .into_any_element()
        };
        div()
            .w(px(460.))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .border_r_1()
            .border_color(t.border)
            .child(header)
            .child(body)
            .into_any_element()
    }

    fn render_reader(&self, cx: &App) -> AnyElement {
        let t = cx.theme();
        let pane = div().flex_1().h_full().min_w_0().flex().flex_col().px_5().py_4();
        let Some(msg) = self.opened.and_then(|id| self.mailbox.get(id)) else {
            return pane
                .items_center()
                .justify_center()
                .text_size(px(13.))
                .text_color(t.muted_foreground)
                .child("enter to open")
                .into_any_element();
        };
        let state = self.mailbox.state_of(msg.id).unwrap_or_default();
        let newest = self.newest();
        let mut thread: Vec<&Message> = self
            .mailbox
            .messages()
            .iter()
            .filter(|m| m.thread_id == msg.thread_id)
            .collect();
        thread.sort_by(|a, b| a.received.cmp(&b.received));
        let thread_len = thread.len();
        pane.gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .text_size(px(16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(msg.subject.clone()),
                    )
                    .child(Tag::secondary().outline().child(state.label())),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(t.muted_foreground)
                    .child(format!(
                        "{} <{}> → {} · {}",
                        msg.from_name,
                        msg.from_email,
                        msg.to,
                        Self::clock_label(&msg.received, &newest)
                    )),
            )
            .child(
                div()
                    .id("reader-body")
                    .flex_1()
                    .overflow_y_scroll()
                    .text_size(px(13.))
                    .line_height(relative(1.5))
                    .child(msg.body.clone()),
            )
            .when(thread_len > 1, |d| {
                d.child(
                    div()
                        .flex_none()
                        .pt_2()
                        .border_t_1()
                        .border_color(t.border)
                        .flex()
                        .flex_col()
                        .text_size(px(12.))
                        .child(
                            div()
                                .pb_1()
                                .text_size(px(11.))
                                .text_color(t.muted_foreground)
                                .child(format!("THREAD · {thread_len}")),
                        )
                        .children(thread.into_iter().map(|m| {
                            let here = m.id == msg.id;
                            div()
                                .h(px(22.))
                                .flex()
                                .items_center()
                                .gap_2()
                                .text_color(if here { t.foreground } else { t.muted_foreground })
                                .child(
                                    div()
                                        .w(px(110.))
                                        .flex_none()
                                        .truncate()
                                        .child(m.from_name.clone()),
                                )
                                .child(div().flex_1().truncate().child(m.body.replace('\n', " ")))
                                .child(
                                    div()
                                        .flex_none()
                                        .text_size(px(11.))
                                        .child(Self::clock_label(&m.received, &newest)),
                                )
                        })),
                )
            })
            .into_any_element()
    }
}

impl Focusable for MailApp {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for MailApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let counts = TriageState::ALL.map(|s| (s, self.mailbox.count(s)));
        let hint = if self.compose.is_some() {
            HintMode::Compose
        } else {
            match self.triage.selected().len() {
                0 => HintMode::List,
                n => HintMode::Selection(n),
            }
        };
        let list = self.render_list(cx);
        let reader = match &self.compose {
            Some(compose) => div()
                .flex_1()
                .h_full()
                .min_w_0()
                .child(compose.clone())
                .into_any_element(),
            None => self.render_reader(cx),
        };
        let t = cx.theme();
        let (bg, fg, border, muted, primary, sidebar) = (
            t.background,
            t.foreground,
            t.border,
            t.muted_foreground,
            t.primary,
            t.sidebar,
        );

        div()
            .id("mail-app")
            .track_focus(&self.focus_handle)
            .when(!self.modal_open(), |d| d.key_context(MAIL_CONTEXT))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(bg)
            .text_color(fg)
            .text_size(px(13.))
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| {
                this.triage.move_cursor(&this.mailbox, 1);
                this.scroll_to_cursor();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &SelectPrev, _, cx| {
                this.triage.move_cursor(&this.mailbox, -1);
                this.scroll_to_cursor();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ExtendNext, _, cx| {
                this.triage.extend(&this.mailbox, 1);
                this.scroll_to_cursor();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ExtendPrev, _, cx| {
                this.triage.extend(&this.mailbox, -1);
                this.scroll_to_cursor();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleSelect, _, cx| {
                this.triage.toggle_select(&this.mailbox);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ClearSelection, window, cx| {
                if this.help {
                    this.help = false;
                } else if this.modal_open() {
                    this.close_modals(window, cx);
                } else {
                    this.triage.clear_selection();
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &OpenMessage, _, cx| {
                if let Some(id) = this.triage.cursor(&this.mailbox) {
                    this.opened = Some(id);
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &MarkDone, w, cx| this.mark(TriageState::Done, w, cx)))
            .on_action(cx.listener(|this, _: &MarkWaiting, w, cx| this.mark(TriageState::Waiting, w, cx)))
            .on_action(cx.listener(|this, _: &MarkLater, w, cx| this.mark(TriageState::Later, w, cx)))
            .on_action(cx.listener(|this, _: &MoveToInbox, w, cx| this.mark(TriageState::Inbox, w, cx)))
            .on_action(cx.listener(|this, _: &SenderDone, w, cx| this.mark_sender(TriageState::Done, w, cx)))
            .on_action(cx.listener(|this, _: &SenderWaiting, w, cx| this.mark_sender(TriageState::Waiting, w, cx)))
            .on_action(cx.listener(|this, _: &SenderLater, w, cx| this.mark_sender(TriageState::Later, w, cx)))
            .on_action(cx.listener(|this, _: &SenderInbox, w, cx| this.mark_sender(TriageState::Inbox, w, cx)))
            .on_action(cx.listener(|this, _: &Undo, window, cx| {
                if this.mailbox.undo() {
                    this.show_toast("Undone".into(), window, cx);
                }
                this.scroll_to_cursor();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleCommandPalette, window, cx| {
                if this.palette.is_some() {
                    this.close_modals(window, cx);
                } else if this.compose.is_none() {
                    this.open_palette(window, cx);
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
            .on_action(cx.listener(|this, _: &ToggleHelp, _, cx| {
                this.help = !this.help;
                cx.notify();
            }))
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
                            .child(ViewTabs::new(self.triage.view, counts)),
                    )
                    .child(list)
                    .child(reader),
            )
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
                                .px_3()
                                .py_1()
                                .rounded_md()
                                .bg(primary)
                                .text_color(bg)
                                .text_size(px(12.))
                                .font_weight(FontWeight::MEDIUM)
                                .child(text),
                        ),
                )
            })
            .when(self.help, |d| {
                d.child(
                    div()
                        .absolute()
                        .inset_0()
                        .occlude()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(bg.opacity(0.85))
                        .text_color(muted)
                        .child(HelpOverlay::new()),
                )
            })
            .when_some(self.palette.clone(), |d, palette| {
                d.child(
                    div()
                        .absolute()
                        .inset_0()
                        .occlude()
                        .flex()
                        .justify_center()
                        .pt(px(80.))
                        .child(palette),
                )
            })
    }
}
