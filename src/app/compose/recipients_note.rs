//! The composer's side of the recipient fields: routing their events, the autocomplete's
//! directory, and the reply-all note that says who was dropped and offers to bring them back.

use super::{ComposeEvent, Composer};
use super::recipients::{Field, RecipientEvent};
use crate::outgoing::DraftKind;
use crate::recipient_suggest::{self as suggest, Directory};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::rc::Rc;

impl Composer {
    /// Hand the fields who can be suggested, and the reply-all recipients the thread
    /// started with (`(field, chip)`, empty for anything but a reply-all) so removing one
    /// can be noticed and undone.
    pub fn set_recipient_context(
        &mut self,
        directory: Rc<Directory>,
        baseline: Vec<(Field, String)>,
        cx: &mut Context<Self>,
    ) {
        self.recipient_baseline = baseline;
        for entity in [&self.to, &self.cc, &self.bcc] {
            let directory = directory.clone();
            entity.update(cx, |r, cx| r.set_directory(directory, cx));
        }
        cx.notify();
    }

    /// A recipient field reported something: a changed value goes into the draft; a typed
    /// `cc:`/`bcc:` reveals that field and moves the rest of the typing into it.
    pub(super) fn recipient_event(
        &mut self,
        event: &RecipientEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.frozen() {
            return;
        }
        match event {
            RecipientEvent::Changed { field, value } => {
                let current = match field {
                    Field::To => &mut self.draft.to,
                    Field::Cc => &mut self.draft.cc,
                    Field::Bcc => &mut self.draft.bcc,
                };
                if current != value {
                    current.clone_from(value);
                    cx.emit(ComposeEvent::Edited);
                }
            }
            RecipientEvent::Reveal { field, rest } => {
                self.reveal_field(*field, window, cx);
                let target = match field {
                    Field::Cc => &self.cc,
                    _ => &self.bcc,
                };
                target.update(cx, |r, cx| r.set_typed(rest, window, cx));
            }
        }
    }

    /// Reply-all recipients of the thread that the draft no longer addresses.
    fn dropped_recipients(&self) -> Vec<(Field, String)> {
        if self.draft.kind != DraftKind::ReplyAll {
            return Vec::new();
        }
        let now: Vec<String> = [&self.draft.to, &self.draft.cc, &self.draft.bcc]
            .into_iter()
            .flat_map(|f| crate::outgoing::split_addresses(f))
            .collect();
        let gone = suggest::dropped(
            &self.recipient_baseline.iter().map(|(_, c)| c.clone()).collect::<Vec<_>>(),
            &now,
        );
        self.recipient_baseline
            .iter()
            .filter(|(_, c)| gone.contains(c))
            .cloned()
            .collect()
    }

    /// Put the dropped recipients back in the field each came from.
    fn restore_dropped(&mut self, cx: &mut Context<Self>) {
        for (field, chip) in self.dropped_recipients() {
            let entity = match field {
                Field::To => &self.to,
                Field::Cc => {
                    self.cc_shown = true;
                    &self.cc
                }
                Field::Bcc => {
                    self.bcc_shown = true;
                    &self.bcc
                }
            };
            entity.update(cx, |r, cx| r.add(&chip, cx));
        }
        cx.notify();
    }

    /// `↺ ~~Raj Patel~~ dropped  Restore`, an overlay on the header's bottom-right; several
    /// people collapse into `N people dropped`. `None` when nobody was dropped.
    pub fn render_dropped_note(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let dropped = self.dropped_recipients();
        if dropped.is_empty() || self.frozen() {
            return None;
        }
        let t = cx.theme();
        let (warn, fg, muted, strong) = (t.warning, t.foreground, t.muted_foreground, t.foreground);
        let who = match dropped.as_slice() {
            [(_, one)] => div()
                .line_through()
                .text_color(muted)
                .child(suggest::label_of(one))
                .into_any_element(),
            many => div().child(format!("{} people", many.len())).into_any_element(),
        };
        Some(
            div()
                .id("compose-dropped-note")
                .test_support()
                .absolute()
                .right(px(10.))
                .bottom(px(7.))
                .flex()
                .items_center()
                .gap(px(6.))
                .h(px(24.))
                .pl(px(7.))
                .pr(px(4.))
                .rounded(px(6.))
                .border_1()
                .border_color(warn.opacity(0.35))
                .bg(warn.opacity(0.1))
                .text_size(px(11.))
                .text_color(fg)
                .whitespace_nowrap()
                .child(Icon::new(gpui_kit::assets::IconName::Undo2).with_size(px(12.)).text_color(warn))
                .child(who)
                .child("dropped")
                .child(
                    div()
                        .id("compose-dropped-restore")
                        .test_support()
                        .px(px(3.))
                        .cursor_pointer()
                        .underline()
                        .text_color(strong)
                        .child("Restore")
                        .on_click(cx.listener(|this, _, _, cx| this.restore_dropped(cx))),
                )
                .into_any_element(),
        )
    }
}
