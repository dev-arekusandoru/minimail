//! Reply composer: read-only To/Subject header plus a multiline body.

use crate::app::actions::{CancelCompose, COMPOSE_CONTEXT, SendReply};
use crate::app::chrome::{HintBar, HintMode};
use crate::model::{Message, MessageId};
use gpui_kit::{
    component::{
        input::{InputEvent, Textarea, TextareaState},
        label::Label,
    },
    *,
};

use crate::theme::{self, Theme};

pub enum ComposeEvent {
    Send { in_reply_to: MessageId, body: String },
    Cancel,
}

pub struct ComposeReply {
    in_reply_to: MessageId,
    to: String,
    subject: String,
    body: Entity<TextareaState>,
}

impl ComposeReply {
    pub fn new(msg: &Message, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let body = cx.new(|cx| TextareaState::new(window, cx).placeholder("Write your reply…"));
        let subject = if msg.subject.to_lowercase().starts_with("re:") {
            msg.subject.clone()
        } else {
            format!("Re: {}", msg.subject)
        };
        // The kit Textarea binds cmd-enter ("secondary-enter") itself, inserting a
        // newline and emitting PressEnter; treat that as "send" and drop the newline.
        cx.subscribe(&body, |this, body, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { secondary: true, .. }) {
                let mut text = body.read(cx).value().to_string();
                if text.ends_with('\n') {
                    text.pop();
                }
                cx.emit(ComposeEvent::Send {
                    in_reply_to: this.in_reply_to,
                    body: text,
                });
            }
        })
        .detach();
        Self {
            in_reply_to: msg.id,
            to: format!("{} <{}>", msg.from_name, msg.from_email),
            subject,
            body,
        }
    }

    pub fn set_body(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.body
            .update(cx, |body, cx| body.set_value(text, window, cx));
    }

    pub fn body(&self, cx: &App) -> String {
        self.body.read(cx).value().to_string()
    }
}

impl Focusable for ComposeReply {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.body.focus_handle(cx)
    }
}

impl EventEmitter<ComposeEvent> for ComposeReply {}

fn header(t: &Theme, name: &'static str, value: String) -> impl IntoElement {
    div()
        .flex()
        .gap_2()
        .text_sm()
        .child(div().w(px(56.)).text_color(t.text_muted).child(name))
        .child(div().text_color(t.text).child(Label::new(value)))
}

impl Render for ComposeReply {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::active(cx);
        div()
            .key_context(COMPOSE_CONTEXT)
            .on_action(cx.listener(|this, _: &SendReply, _, cx| {
                let body = this.body.read(cx).value().to_string();
                cx.emit(ComposeEvent::Send {
                    in_reply_to: this.in_reply_to,
                    body,
                });
            }))
            .on_action(cx.listener(|_, _: &CancelCompose, _, cx| cx.emit(ComposeEvent::Cancel)))
            .size_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_4()
            .bg(t.background)
            .child(header(&t, "To", self.to.clone()))
            .child(header(&t, "Subject", self.subject.clone()))
            .child(gpui_kit::component::separator::Separator::horizontal())
            .child(div().flex_1().child(Textarea::new(&self.body).h_full()))
            .child(HintBar::new(HintMode::Compose))
    }
}
