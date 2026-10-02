//! Recipient fields as editable chips: a list of committed addresses plus a live input
//! that turns what you type into chips as you go.
//!
//! The record stores comma-separated strings (the format `Message::to` uses), so the chips
//! are a view over those fields: committing a chip rewrites the field, and removing one
//! rewrites it again. Enter, comma and semicolon all commit, which is what a mail client
//! does, and Backspace on an empty input takes back the last chip.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::*;

/// Key context of a recipient input, so its own chords reach it.
pub const RECIPIENTS_CONTEXT: &str = "ComposeRecipients";

/// What ended an edit of a recipient field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecipientEvent {
    /// The field's committed addresses changed (a chip added, removed or replaced).
    Changed { field: Field, value: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Field {
    To,
    Cc,
    Bcc,
}

impl Field {
    pub fn label(self) -> &'static str {
        match self {
            Self::To => "To",
            Self::Cc => "Cc",
            Self::Bcc => "Bcc",
        }
    }

    pub const ALL: [Field; 3] = [Field::To, Field::Cc, Field::Bcc];
}

/// One recipient field: committed chips and the input that fills them.
pub struct Recipients {
    field: Field,
    input: Entity<InputState>,
    /// Committed addresses, in the order they were added, as typed.
    chips: Vec<String>,
    /// A held, sending or unconfirmed draft may not be edited, so the chips and the input
    /// stop accepting changes rather than silently dropping them.
    pub(crate) disabled: bool,
    _input_sub: Option<Subscription>,
}

impl Recipients {
    pub fn new(
        field: Field,
        initial: &str,
        placeholder: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let mut this = Self {
            field,
            input,
            chips: Vec::new(),
            disabled: false,
            _input_sub: None,
        };
        this.set_field_text(initial, window, cx);
        // `subscribe_in` rather than `subscribe`: committing a chip rewrites the input's own
        // value, which the kit's input state only accepts with a window in hand.
        this._input_sub = Some(cx.subscribe_in(
            &this.input,
            window,
            |this, input, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter {
                    secondary: false, ..
                } => this.commit(window, cx),
                InputEvent::Change => {
                    // Typing a separator is a commit, the way every mail client behaves.
                    let typed = input.read(cx).value().to_string();
                    if typed.ends_with([',', ';']) {
                        this.commit(window, cx);
                    }
                }
                _ => {}
            },
        ));
        this
    }

    /// Freeze or unfreeze the field.
    pub fn set_disabled(&mut self, disabled: bool, cx: &mut Context<Self>) {
        self.disabled = disabled;
        cx.notify();
    }

    pub fn field(&self) -> Field {
        self.field
    }

    /// The committed addresses as the record stores them.
    pub fn value(&self) -> String {
        self.chips.join(", ")
    }

    pub fn chips(&self) -> &[String] {
        &self.chips
    }

    /// Replace the field's addresses from a record value, keeping any half-typed
    /// address as the input's contents.
    pub fn set_value(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.value() == text {
            return;
        }
        self.set_field_text(text, window, cx);
        cx.notify();
    }

    fn set_field_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.chips = crate::outgoing::split_addresses(text);
        let tail = text.trim_end();
        let typed = tail
            .rsplit(',')
            .next()
            .unwrap_or_default()
            .trim_start()
            .to_owned();
        self.input
            .update(cx, |i, cx| i.set_value(typed, window, cx));
    }

    fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        let typed = self.input.read(cx).value().to_string();
        let typed = typed.trim().trim_end_matches([',', ';']).trim().to_owned();
        if typed.is_empty() {
            return;
        }
        if !looks_like_address(&typed) {
            return;
        }
        let key = typed.to_lowercase();
        let added = !self.chips.iter().any(|c| c.to_lowercase() == key);
        if added {
            self.chips.push(typed);
        }
        self.input.update(cx, |i, cx| i.set_value("", window, cx));
        if added {
            let value = self.value();
            cx.emit(RecipientEvent::Changed {
                field: self.field,
                value,
            });
        }
        cx.notify();
    }

    /// Drop chip `ix`, and tell the owner the field changed.
    fn remove(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.disabled || ix >= self.chips.len() {
            return;
        }
        self.chips.remove(ix);
        let value = self.value();
        cx.emit(RecipientEvent::Changed {
            field: self.field,
            value,
        });
        cx.notify();
    }

    /// Put chip `ix` back in the input so it can be corrected.
    fn edit(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        let Some(chip) = self.chips.get(ix).cloned() else {
            return;
        };
        self.chips.remove(ix);
        self.input.update(cx, |i, cx| i.set_value(chip, window, cx));
        cx.notify();
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.input.read(cx).focus_handle(cx), cx);
    }

    fn render_chips(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        // The chip's own remove and edit handlers need the context mutably, so the colours
        // are copied out rather than borrowed from the theme across the whole map.
        let accent = cx.theme().accent.opacity(0.12);
        let border = cx.theme().border;
        self.chips
            .iter()
            .enumerate()
            .map(|(ix, chip)| {
                // `ElementId` takes at most a two-part tuple, so the field goes into the
                // second half: a chip is found by `(field, chip index)`.
                let key = ix * 4 + self.field as usize;
                let label = chip.clone();
                div()
                    .flex()
                    .flex_none()
                    .max_w(px(220.))
                    .h(px(20.))
                    .pl_2()
                    .pr_1()
                    .gap_1()
                    .items_center()
                    .rounded_sm()
                    .bg(accent)
                    .border_1()
                    .border_color(border)
                    .id(("compose-chip", key))
                    .test_support()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_size(px(11.))
                            .child(label),
                    )
                    .child(
                        Button::new(("compose-chip-remove", key))
                            .icon(gpui_kit::assets::IconName::Close)
                            .ghost()
                            .xsmall()
                            .w(px(14.))
                            .h(px(14.))
                            .px_0()
                            .on_click(cx.listener(move |this, _, _, cx| this.remove(ix, cx))),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| this.edit(ix, window, cx)),
                    )
                    .into_any_element()
            })
            .collect()
    }
}

/// Unresolved names remain editable; addressed chips use the sender's validation policy.
pub fn looks_like_address(candidate: &str) -> bool {
    let trimmed = candidate.trim();
    if trimmed.is_empty() {
        return false;
    }
    if !trimmed.contains('@') {
        return !trimmed.contains(char::is_whitespace);
    }
    let recipients = crate::reading::parse_recipients(trimmed);
    matches!(recipients.as_slice(), [recipient] if crate::outgoing::draft::valid_email(&recipient.email))
}

/// The chips plus their input, wrapped so they reflow across lines.
impl Render for Recipients {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let chips = self.render_chips(cx);
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1()
            .min_h(px(22.))
            .py_0()
            .key_context(RECIPIENTS_CONTEXT)
            .children(chips)
            .child(
                Input::new(&self.input)
                    .h(px(20.))
                    .bordered(false)
                    .flex_1()
                    .min_w(px(80.))
                    .disabled(self.disabled),
            )
    }
}

impl Focusable for Recipients {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.read(cx).focus_handle(cx).clone()
    }
}

impl EventEmitter<RecipientEvent> for Recipients {}

#[cfg(test)]
mod tests {
    use super::looks_like_address;

    #[test]
    fn addresses_need_a_domain_but_bare_names_are_allowed() {
        assert!(looks_like_address("a@b.test"));
        assert!(looks_like_address("Someone <a@b.test>"));
        assert!(looks_like_address("someone"));
        assert!(!looks_like_address("a@b"), "a domain needs a dot");
        assert!(!looks_like_address("@b.test"));
        assert!(!looks_like_address("two words"));
        assert!(!looks_like_address(""));
    }
}
