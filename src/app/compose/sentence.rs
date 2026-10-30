//! The header as one line of prose: `To [people] Cc Bcc  about [subject]  from [account]
//! then [what happens after send]`. Each clause is a verb plus a token; unset optional
//! clauses are ghost words, and clauses wrap whole when the line runs out.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::DropdownMenu as _;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::rc::Rc;

use super::Composer;
use super::recipients::Field;
use crate::app::actions::ComposeAccount;
use crate::app::icons;
use crate::outgoing::DraftKind;

/// Per-clause verb: 13px prose, readable but not as strong as the tokens.
fn verb(text: &'static str, color: Hsla) -> Div {
    div()
        .text_size(px(13.))
        .line_height(px(20.))
        .text_color(color)
        .child(text)
}

fn dim_on<E: Styled>(on: bool, el: E) -> E {
    if on { el.opacity(0.55) } else { el }
}

/// A clause: a verb and its token, kept whole when the line wraps.
fn clause() -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .min_w_0()
        .max_w_full()
}

impl Composer {
    /// The header: the sentence, the focus tint, and the dropped-recipient note riding its
    /// bottom-right as an overlay.
    pub(super) fn render_headers(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Stateful<Div> {
        let frozen = self.frozen();
        let to_focused = self.to.read(cx).focus_handle(cx).contains_focused(window, cx);
        let subject_focused = self.subject.read(cx).focus_handle(cx).is_focused(window);
        let t = cx.theme();
        let (muted, prose, border, field, strong) =
            (t.muted_foreground, t.secondary_foreground, t.border, t.secondary, t.foreground);
        let primary = t.primary;
        // While the To field is being edited, everything else steps back.
        let dim = |el: Div| if to_focused { el.opacity(0.55) } else { el };
        let ghost = |id: &'static str, label: &'static str, f: Field, cx: &mut Context<Self>| {
            div()
                .id(id)
                .test_support()
                .cursor_pointer()
                .text_size(px(13.))
                .line_height(px(20.))
                .text_color(muted)
                .border_b_1()
                .border_dashed()
                .border_color(muted.opacity(0.45))
                .px(px(2.))
                .hover(move |s| s.text_color(strong))
                .child(label)
                .when(!frozen, |d| {
                    d.on_click(cx.listener(move |this, _, window, cx| {
                        this.reveal_field(f, window, cx)
                    }))
                })
        };

        let reply = !matches!(self.draft.kind, DraftKind::New | DraftKind::Forward);
        let to = clause()
            .id("compose-to")
            .test_support()
            .child(
                // Empty space in the clause puts the caret in the To input.
                div()
                    .flex()
                    .flex_grow_1()
                    .items_center()
                    .gap(px(6.))
                    .min_w_0()
                    .cursor_text()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| this.focus_recipients(window, cx)),
                    )
                    .child(verb(if reply { "Reply to" } else { "To" }, prose))
                    .child(div().flex_grow_1().min_w_0().child(self.to.clone())),
            )
            .when(!self.cc_shown, |d| {
                d.child(ghost("compose-show-cc", "Cc", Field::Cc, cx))
            })
            .when(!self.bcc_shown, |d| {
                d.child(ghost("compose-show-bcc", "Bcc", Field::Bcc, cx))
            });

        // The token is as wide as its text (or the placeholder), measured, not estimated.
        let text = self.subject.read(cx).value();
        let text: &str = if text.is_empty() { "Subject" } else { text.as_ref() };
        let mut run = window.text_style().to_run(text.len());
        if reply {
            run.font.weight = FontWeight::MEDIUM;
        }
        let text_w = window
            .text_system()
            .layout_line(text, px(13.), &[run], None)
            .width;
        let subject = dim(clause().child(verb("about", prose)).child(
            div()
                .id("compose-subject")
                .test_support()
                .h(px(24.))
                .px(px(7.))
                .min_w_0()
                .max_w_full()
                .flex()
                .items_center()
                .overflow_hidden()
                .rounded(px(6.))
                .border_1()
                .border_color(if subject_focused { primary } else { border })
                .bg(field)
                .child(
                    // `xsmall` keeps the field at text height with 4px side padding, so the
                    // measured width plus that padding shows the whole subject.
                    Input::new(&self.subject)
                        .appearance(false)
                        .xsmall()
                        .w(text_w + px(10.))
                        .max_w_full()
                        .text_size(px(13.))
                        .text_color(strong)
                        .when(reply, |i| i.font_weight(FontWeight::MEDIUM))
                        .disabled(frozen),
                ),
        ));

        let from = self.account_token(frozen, cx).map(|token| {
            dim(clause().child(verb("from", prose)).child(token))
        });

        let mut sentence = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_x(px(12.))
            .gap_y(px(7.))
            .child(to);
        for (id, label, entity, shown) in [
            ("compose-cc", "Cc", &self.cc, self.cc_shown),
            ("compose-bcc", "Bcc", &self.bcc, self.bcc_shown),
        ] {
            if shown {
                sentence = sentence.child(dim_on(to_focused,
                    clause()
                        .id(id)
                        .test_support()
                        .child(verb(label, prose))
                        .child(div().flex_grow_1().min_w_0().child(entity.clone())),
                ));
            }
        }
        sentence = sentence.child(subject).children(from);
        let intent = dim(clause().child(self.render_intent_clause(cx)));
        let sentence = sentence.child(intent);
        let note = self.render_dropped_note(cx);

        let t = cx.theme();
        div()
            .id("compose-header")
            .relative()
            .flex_none()
            .px(px(10.))
            .pt(px(10.))
            .pb(px(9.))
            .border_b(px(1.))
            .border_color(t.border)
            .when(to_focused, |d| d.bg(t.secondary.opacity(0.4)))
            .child(sentence)
            .children(note)
    }

    /// The sending account as a token with a menu of the mailbox's accounts. With one
    /// account there is nothing to choose, so the clause is not shown at all. While the
    /// message is held or in flight the account stays visible but cannot be changed.
    fn account_token(&self, frozen: bool, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.accounts.len() < 2 {
            return None;
        }
        let current = self.accounts.iter().find(|a| a.id == self.draft.account);
        let label = current.map_or_else(|| self.draft.account.clone(), |a| a.label.clone());
        let t = cx.theme();
        let icon = current.map_or_else(
            || Icon::new(gpui_kit::assets::IconName::Mail).with_size(px(13.)).text_color(t.muted_foreground),
            |a| icons::account_icon(a.icon, &a.color, t, 13.),
        );
        let app = cx.weak_entity();
        Some(
            Button::new("compose-account")
                .ghost()
                .icon(icon)
                .label(label)
                .child(Icon::new(gpui_kit::assets::IconName::ChevronDown).size_3())
                .h(px(24.))
                .px(px(7.))
                .rounded(px(6.))
                .border_1()
                .border_color(t.border)
                .bg(t.secondary)
                .text_size(px(13.))
                .text_color(t.foreground)
                .tooltip("Send this message from a different account")
                .disabled(frozen)
                .dropdown_menu_with_anchor(Anchor::BottomLeft, move |menu, window, cx| {
                    let Some(this) = app.upgrade() else {
                        return menu;
                    };
                    let accounts = this.read(cx).accounts.clone();
                    let t = cx.theme();
                    // A chosen row dispatches what the action would, so the composer hears
                    // it through the same handler as any other key or click.
                    let owner = app.clone();
                    let run: crate::app::menu::MenuRun = Rc::new(move |action, window, cx| {
                        owner
                            .update(cx, |_, cx| window.dispatch_action(action, cx))
                            .ok();
                    });
                    let items: Vec<crate::app::menu::MenuItem> = accounts
                        .into_iter()
                        .map(|a| {
                            let id = a.id;
                            crate::app::menu::MenuItem::action_fn(a.label, move || {
                                Box::new(ComposeAccount { account: id.clone() })
                                    as Box<dyn Action>
                            })
                            .icon(icons::account_icon(a.icon, &a.color, t, 14.))
                        })
                        .collect();
                    crate::app::menu::populate(menu, &items, &run, window, cx)
                })
                .into_any_element(),
        )
    }


}
