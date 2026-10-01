//! Composer for Reply, Reply all and Forward: editable To/Cc, read-only Subject, multiline body.
use gpui_kit::component::ActiveTheme as _;

use crate::app::actions::{CancelCompose, COMPOSE_CONTEXT, SendReply};
use crate::app::chrome::HintBar;
use crate::hints::{HintContext, HintMode};
use crate::app::ui::button;
use crate::draft::{DraftKind, draft};
use crate::model::{Message, MessageId};
use gpui_kit::{
    component::{
        input::{Input, InputEvent, InputState, Textarea, TextareaState},
    },
    *,
};

pub enum ComposeEvent {
    Send { in_reply_to: MessageId, kind: DraftKind, body: String },
    Cancel,
}

pub struct ComposeReply {
    in_reply_to: MessageId,
    kind: DraftKind,
    to: Entity<InputState>,
    cc: Entity<InputState>,
    subject: String,
    body: Entity<TextareaState>,
}

impl ComposeReply {
    /// `own_email` is the receiving account's address (excluded from Reply all).
    pub fn new(
        kind: DraftKind,
        msg: &Message,
        own_email: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let d = draft(kind, msg, own_email);
        let placeholder = if kind == DraftKind::Forward { "Add a note…" } else { "Write your reply…" };
        let body = cx.new(|cx| TextareaState::new(window, cx).placeholder(placeholder));
        if !d.body.is_empty() {
            body.update(cx, |b, cx| b.set_value(d.body.as_str(), window, cx));
        }
        let to = cx.new(|cx| {
            let mut i = InputState::new(window, cx).placeholder("Recipients");
            i.set_value(d.to.as_str(), window, cx);
            i
        });
        let cc = cx.new(|cx| {
            let mut i = InputState::new(window, cx).placeholder("Cc");
            i.set_value(d.cc.as_str(), window, cx);
            i
        });
        // The kit Textarea binds cmd-enter ("secondary-enter") itself, inserting a
        // newline and emitting PressEnter; treat that as "send" and drop the newline.
        cx.subscribe(&body, move |this, body, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { secondary: true, .. }) {
                let mut text = body.read(cx).value().to_string();
                if text.ends_with('\n') {
                    text.pop();
                }
                cx.emit(ComposeEvent::Send {
                    in_reply_to: this.in_reply_to,
                    kind: this.kind,
                    body: text,
                });
            }
        })
        .detach();
        Self {
            in_reply_to: msg.id,
            kind,
            to,
            cc,
            subject: d.subject,
            body,
        }
    }

    pub fn set_body(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.body
            .update(cx, |body, cx| body.set_value(text, window, cx));
    }

    pub fn kind(&self) -> DraftKind {
        self.kind
    }

    pub fn to(&self, cx: &App) -> String {
        self.to.read(cx).value().to_string()
    }

    pub fn cc(&self, cx: &App) -> String {
        self.cc.read(cx).value().to_string()
    }

    pub fn subject(&self) -> &str {
        &self.subject
    }

    pub fn body(&self, cx: &App) -> String {
        self.body.read(cx).value().to_string()
    }

    /// `cmd-enter` / the Send button: hand the draft to the owner.
    fn send(&self, cx: &mut Context<Self>) {
        let body = self.body.read(cx).value().to_string();
        cx.emit(ComposeEvent::Send {
            in_reply_to: self.in_reply_to,
            kind: self.kind,
            body,
        });
    }
}

impl Focusable for ComposeReply {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        // A forward starts with no recipient: type it first.
        if self.kind == DraftKind::Forward {
            self.to.focus_handle(cx)
        } else {
            self.body.focus_handle(cx)
        }
    }
}

impl EventEmitter<ComposeEvent> for ComposeReply {}

/// One header row: fixed-width label, then the editable field or plain text.
fn row(label: &'static str, field: impl IntoElement, cx: &App) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(div().w(px(56.)).text_sm().text_color(cx.theme().muted_foreground).child(label))
        .child(div().flex_1().min_w_0().child(field))
}

impl Render for ComposeReply {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        div()
            .key_context(COMPOSE_CONTEXT)
            .on_action(cx.listener(|this, _: &SendReply, _, cx| this.send(cx)))
            .on_action(cx.listener(|_, _: &CancelCompose, _, cx| cx.emit(ComposeEvent::Cancel)))
            .size_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_4()
            .bg(t.background)
            .child(row("To", Input::new(&self.to), cx))
            .child(row("Cc", Input::new(&self.cc), cx))
            .child(row("Subject", self.subject.clone(), cx))
            .child(gpui_kit::component::separator::Separator::horizontal())
            .child(div().flex_1().child(Textarea::new(&self.body).h_full()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap_2()
                    .child(
                        button("compose-cancel", "Cancel", "Discard the draft", "escape", cx)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(ComposeEvent::Cancel))),
                    )
                    .child(
                        button("compose-send", "Send", "Send (held 10s; undo recalls it)", "cmd-enter", cx)
                            .bg(t.primary)
                            .text_color(t.primary_foreground)
                            .on_click(cx.listener(|this, _, _, cx| this.send(cx))),
                    ),
            )
            .child(HintBar::new(HintContext::new(HintMode::Compose)))
    }
}
