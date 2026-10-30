//! Recipient fields as chips: committed addresses plus a live input that turns what you type
//! into chips as you go, with a floating autocomplete under the input.
//!
//! The record stores comma-separated strings (the format `Message::to` uses), so the chips
//! are a view over those fields: committing a chip rewrites the field, and removing one
//! rewrites it again. Comma, semicolon, Tab and Enter commit; Backspace on an empty input
//! selects the last chip and a second one removes it; a paste splits into one chip per
//! address. Blurred, the field is one line of the chips that fit, a `+N` chip and a dashed
//! add button; focused, every chip wraps and the input follows the last one.
//! The pure parts (matching, splitting, fitting) live in [`crate::recipient_suggest`].

use crate::recipient_suggest::{self as suggest, Directory, Suggestion};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::rc::Rc;

/// Key context of a recipient input, so its own chords reach it.
pub const RECIPIENTS_CONTEXT: &str = "ComposeRecipients";

/// Autocomplete rows shown at most (the new-address row comes on top of these).
const SUGGESTION_ROWS: usize = 5;

/// What an edit of a recipient field tells its owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecipientEvent {
    /// The field's committed addresses changed (a chip added or removed).
    Changed { field: Field, value: String },
    /// The user typed `cc:` or `bcc:`: the owner shows that field, focuses it and hands it
    /// `rest`, whatever was typed after the prefix.
    Reveal { field: Field, rest: String },
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
    /// Committed addresses, in the order they were added, as `Name <addr>` or `addr`.
    chips: Vec<String>,
    /// A held, sending or unconfirmed draft may not be edited, so the chips and the input
    /// stop accepting changes rather than silently dropping them.
    pub(crate) disabled: bool,
    /// Who can be suggested, and whose address is new. Empty until the owner provides it.
    directory: Rc<Directory>,
    suggestions: Vec<Suggestion>,
    /// The highlighted autocomplete row; `suggestions.len()` is the new-address row.
    active: usize,
    /// Whether the autocomplete is open (Esc closes it until the next keystroke).
    popup_open: bool,
    /// The chip Backspace has selected and the next Backspace removes.
    selected: Option<usize>,
    /// The width the field was last laid out at, so a blurred line knows how many chips fit.
    width: Option<f32>,
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
            directory: Rc::default(),
            suggestions: Vec::new(),
            active: 0,
            popup_open: false,
            selected: None,
            width: None,
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
                } => {
                    this.commit(window, cx);
                }
                InputEvent::Change => {
                    let typed = input.read(cx).value().to_string();
                    this.on_typed(&typed, window, cx);
                }
                InputEvent::Focus | InputEvent::Blur => {
                    this.selected = None;
                    this.popup_open = false;
                    cx.notify();
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

    /// Who the autocomplete suggests and whose address the NEW badge flags.
    pub fn set_directory(&mut self, directory: Rc<Directory>, cx: &mut Context<Self>) {
        self.directory = directory;
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

    /// Replace the field's addresses from a record value.
    pub fn set_value(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.value() == text {
            return;
        }
        self.set_field_text(text, window, cx);
        cx.notify();
    }

    fn set_field_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.chips = crate::outgoing::split_addresses(text);
        self.selected = None;
        self.clear_suggestions();
        self.input.update(cx, |i, cx| i.set_value("", window, cx));
    }

    /// Put `text` in the input as if typed, so it is matched, split and committed the same.
    pub fn set_typed(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.input
            .update(cx, |i, cx| i.set_value(text.to_owned(), window, cx));
    }

    /// Commit `chip` (`Name <addr>` or `addr`) unless the address is already there, and tell
    /// the owner. Returns whether it was added.
    pub fn add(&mut self, chip: &str, cx: &mut Context<Self>) -> bool {
        let key = suggest::email_of(chip);
        if self.disabled || self.chips.iter().any(|c| suggest::email_of(c) == key) {
            return false;
        }
        self.chips.push(chip.trim().to_owned());
        self.changed(cx);
        true
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        let value = self.value();
        cx.emit(RecipientEvent::Changed {
            field: self.field,
            value,
        });
        cx.notify();
    }

    fn clear_suggestions(&mut self) {
        self.suggestions.clear();
        self.active = 0;
        self.popup_open = false;
    }

    /// React to the input's text changing: `cc:` hands over, separators and pastes commit,
    /// anything else refreshes the autocomplete.
    fn on_typed(&mut self, typed: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = None;
        if self.disabled {
            return;
        }
        if let Some((name, rest)) = suggest::field_prefix(typed) {
            let target = if name == "bcc" { Field::Bcc } else { Field::Cc };
            if target == self.field {
                self.set_typed(rest, window, cx);
            } else {
                cx.emit(RecipientEvent::Reveal {
                    field: target,
                    rest: rest.to_owned(),
                });
                self.set_typed("", window, cx);
            }
            return;
        }
        let (parts, tail) = suggest::split_input(typed);
        if parts.is_empty() {
            self.refresh(typed);
            cx.notify();
            return;
        }
        // A separator was typed or a list was pasted. A tail that is a whole address after
        // other addresses can only be pasted, so it commits too.
        let mut left: Vec<String> = Vec::new();
        for part in parts.into_iter().chain(Some(tail.clone()).filter(|t| t.contains('@'))) {
            if looks_like_address(&part) {
                self.add(&part, cx);
            } else {
                left.push(part);
            }
        }
        if !tail.contains('@') {
            left.push(tail);
        }
        // Space-joined, never comma-joined: that would split again and loop.
        let left = left.join(" ").trim().to_owned();
        if left != typed {
            self.set_typed(&left, window, cx);
        }
        self.refresh(&left);
        cx.notify();
    }

    fn refresh(&mut self, typed: &str) {
        let typed = typed.trim();
        self.suggestions = self.directory.suggest(typed, &self.chips, SUGGESTION_ROWS);
        self.popup_open = !typed.is_empty();
        // An exact address picks its own row; a whole new address defaults to the
        // new-address row so Enter commits what was typed; otherwise the best match.
        let key = typed.to_lowercase();
        let whole = typed.contains('@') && looks_like_address(typed);
        self.active = match self.suggestions.iter().position(|s| s.address.to_lowercase() == key) {
            Some(ix) => ix,
            None if whole => self.suggestions.len(),
            None => 0,
        };
    }

    /// The text in the input, without a trailing separator.
    fn typed(&self, cx: &App) -> String {
        let typed = self.input.read(cx).value().to_string();
        typed.trim().trim_end_matches([',', ';']).trim().to_owned()
    }

    /// Commit the highlighted autocomplete row, or the typed text itself. Returns whether
    /// a chip was committed.
    fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let typed = self.typed(cx);
        let chip = match self.suggestions.get(self.active) {
            Some(s) if self.popup_open => s.chip(),
            _ if looks_like_address(&typed) => typed,
            _ => return false,
        };
        self.accept(&chip, window, cx);
        true
    }

    fn accept(&mut self, chip: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        self.add(chip, cx);
        self.clear_suggestions();
        self.input.update(cx, |i, cx| i.set_value("", window, cx));
        cx.notify();
    }

    /// Pick autocomplete row `ix`; the last row is the typed text as a new address.
    fn pick(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let chip = match self.suggestions.get(ix) {
            Some(s) => s.chip(),
            None => self.typed(cx),
        };
        if chip.is_empty() {
            return;
        }
        self.accept(&chip, window, cx);
        self.focus(window, cx);
    }

    /// Drop chip `ix`, and tell the owner the field changed.
    fn remove(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.disabled || ix >= self.chips.len() {
            return;
        }
        self.chips.remove(ix);
        self.selected = None;
        self.changed(cx);
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.input.read(cx).focus_handle(cx), cx);
    }

    /// Keys the input would otherwise swallow, seen first: autocomplete navigation, Tab
    /// committing, and Backspace selecting then removing the last chip.
    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &event.keystroke;
        if k.modifiers.platform || k.modifiers.control || k.modifiers.alt || self.disabled {
            return;
        }
        let rows = self.suggestions.len() + 1;
        let open = self.popup_open && !self.typed(cx).is_empty();
        match k.key.as_str() {
            "down" | "up" if open => {
                let step = if k.key == "down" { 1 } else { rows - 1 };
                self.active = (self.active + step) % rows;
            }
            "escape" if open => self.popup_open = false,
            "tab" if !k.modifiers.shift && !self.typed(cx).is_empty() => {
                if !self.commit(window, cx) {
                    return;
                }
            }
            "backspace" if self.typed(cx).is_empty() && !self.chips.is_empty() => {
                match self.selected {
                    Some(ix) => self.remove(ix, cx),
                    None => self.selected = Some(self.chips.len() - 1),
                }
            }
            _ => {
                self.selected = None;
                return;
            }
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn render_chips(&self, focused: bool, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let theme = cx.theme();
        let (field_bg, border, fg, ok, primary, dark) = (
            theme.secondary,
            theme.border,
            theme.foreground,
            theme.success,
            theme.primary,
            theme.is_dark(),
        );
        let limit = if focused { self.chips.len() } else { self.visible() };
        self.chips
            .iter()
            .enumerate()
            .take(limit)
            .map(|(ix, chip)| {
                // `ElementId` takes at most a two-part tuple, so the field goes into the
                // second half: a chip is found by `(field, chip index)`.
                let key = ix * 4 + self.field as usize;
                let (name, address) = suggest::parts_of(chip);
                let label = name.clone().unwrap_or_else(|| address.clone());
                let tip = SharedString::from(match &name {
                    Some(n) => format!("{n} <{address}>"),
                    None => address.clone(),
                });
                // A chip is a token, never a field: hovering shows the `x` that removes it,
                // and a click only puts the caret back in the input beside it.
                let group = SharedString::from(format!("compose-chip-{key}"));
                div()
                    .flex()
                    .flex_none()
                    .relative()
                    .max_w(px(220.))
                    .h(px(24.))
                    .pl(px(4.))
                    .pr(px(7.))
                    .gap(px(5.))
                    .items_center()
                    .rounded(px(6.))
                    .bg(field_bg)
                    .border_1()
                    .border_color(if self.selected == Some(ix) { primary } else { border })
                    .text_size(px(13.))
                    .text_color(fg)
                    .id(("compose-chip", key))
                    .test_support()
                    .group(group.clone())
                    .child(avatar(&label, &address, dark))
                    .child(div().min_w_0().truncate().child(label.clone()))
                    .when(self.directory.is_new(chip), |d| {
                        d.child(
                            div()
                                .flex_none()
                                .px(px(3.))
                                .rounded(px(3.))
                                .border_1()
                                .border_color(ok)
                                .text_color(ok)
                                .text_size(px(8.))
                                .font_weight(FontWeight::BOLD)
                                .child("NEW"),
                        )
                    })
                    .child(
                        // Overlaid on the chip's tail so revealing it never changes the
                        // chip's width; hidden until the chip is hovered.
                        div()
                            .id(("compose-chip-remove", key))
                            .test_support()
                            .absolute()
                            .top(px(4.))
                            .right(px(3.))
                            .size(px(16.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.))
                            .bg(field_bg)
                            .opacity(0.)
                            .group_hover(group, |s| s.opacity(1.))
                            .hover(|s| s.bg(theme.secondary_hover))
                            .cursor_pointer()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.remove(ix, cx)
                                }),
                            )
                            .child(
                                Icon::new(gpui_kit::assets::IconName::X)
                                    .with_size(px(12.))
                                    .text_color(fg),
                            ),
                    )
                    .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                    .into_any_element()
            })
            .collect()
    }

    /// How many chips the blurred line shows: those that fit beside `+N` and the add button.
    fn visible(&self) -> usize {
        let Some(avail) = self.width else {
            return self.chips.len();
        };
        let widths: Vec<f32> = self
            .chips
            .iter()
            .map(|c| suggest::chip_width(&suggest::label_of(c), self.directory.is_new(c)))
            .collect();
        suggest::visible_count(&widths, avail, 4., 36., 20.)
    }

    /// The floating autocomplete: contacts and history, then the typed text as a new address.
    fn render_popup(&self, typed: &str, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (muted, fg, hover, border) = (
            theme.muted_foreground,
            theme.foreground,
            theme.secondary_hover,
            theme.border,
        );
        let dark = theme.is_dark();
        let mono = crate::app::ui::mono_font(cx);
        let row = |ix: usize| {
            div()
                .id(("compose-suggest", ix))
                .test_support()
                .flex()
                .items_center()
                .gap(px(8.))
                .h(px(28.))
                .px(px(8.))
                .rounded(px(5.))
                .text_size(px(12.))
                .text_color(fg)
                .when(ix == self.active, |d| d.bg(hover))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.pick(ix, window, cx)
                    }),
                )
        };
        let mut menu = div()
            .absolute()
            .top_full()
            .left_0()
            .mt(px(4.))
            // A fixed width: clamped to the field, a Cc popup squeezed into its narrow clause
            // and wrapped its header over three lines.
            .w(px(300.))
            .p(px(4.))
            .rounded(px(8.))
            .border_1()
            .border_color(border)
            .bg(cx.theme().popover)
            .shadow_lg()
            .occlude()
            .child(
                div()
                    .px(px(8.))
                    .pt(px(4.))
                    .pb(px(2.))
                    .text_size(px(9.))
                    .font_family(mono.clone())
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(muted)
                    .child("RECIPIENTS · CONTACTS & HISTORY"),
            );
        for (ix, s) in self.suggestions.iter().enumerate() {
            let name = if s.name.is_empty() { &s.address } else { &s.name };
            menu = menu.child(
                row(ix)
                    .child(avatar(name, &s.address, dark))
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(name.clone()))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(muted)
                            .child(s.address.clone()),
                    )
                    .child(
                        div()
                            .ml_auto()
                            .font_family(mono.clone())
                            .text_size(px(10.))
                            .text_color(muted)
                            .child(s.threads.to_string()),
                    ),
            );
        }
        if looks_like_address(typed) {
            let last = self.suggestions.len();
            menu = menu
                .when(last > 0, |m| {
                    m.child(div().h(px(1.)).mx(px(6.)).my(px(4.)).bg(border))
                })
                .child(
                    row(last)
                        .child(Icon::new(gpui_kit::assets::IconName::Plus).with_size(px(13.)).text_color(muted))
                        .child(div().font_weight(FontWeight::MEDIUM).child(typed.to_owned()))
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(muted)
                                .child("send to this new address"),
                        )
                        .child(
                            div()
                                .ml_auto()
                                .font_family(mono)
                                .text_size(px(10.))
                                .text_color(muted)
                                .child("↵"),
                        ),
                );
        }
        deferred(menu).with_priority(1).into_any_element()
    }
}

/// A 16px monogram: the first letter of the name, on a colour picked by the address so the
/// same person always gets the same one.
fn avatar(label: &str, address: &str, dark: bool) -> Div {
    let letter = crate::reading::initials(label, address)
        .chars()
        .next()
        .unwrap_or('?')
        .to_uppercase()
        .to_string();
    let hash = address
        .to_lowercase()
        .bytes()
        .fold(0u64, |h, b| h.wrapping_mul(31).wrapping_add(b as u64));
    let hue = (hash % 12) as f32 / 12.;
    div()
        .flex_none()
        .size(px(16.))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(hsla(hue, 0.40, if dark { 0.68 } else { 0.78 }, 1.))
        .text_color(hsla(0., 0., 0.09, 1.))
        .text_size(px(9.))
        .font_weight(FontWeight::BOLD)
        .child(letter)
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

/// The chips, `+N` and add button (blurred) or the wrapping chips plus input (focused),
/// with the autocomplete floating under them.
impl Render for Recipients {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.input.read(cx).focus_handle(cx).is_focused(window);
        let chips = self.render_chips(focused, cx);
        let hidden = if focused { 0 } else { self.chips.len() - chips.len() };
        let typed = self.typed(cx);
        let (muted, border, field_bg, fg) = {
            let t = cx.theme();
            (t.muted_foreground, t.border, t.secondary, t.foreground)
        };
        let field = self.field as usize;
        let popup = (focused && self.popup_open && !typed.is_empty()
            && (!self.suggestions.is_empty() || looks_like_address(&typed)))
        .then(|| self.render_popup(&typed, cx));
        let view = cx.weak_entity();
        div()
            .relative()
            .flex_1()
            .min_w_0()
            .on_children_prepainted(move |bounds, _, cx| {
                let (Some(width), Some(view)) =
                    (bounds.first().map(|b| f32::from(b.size.width)), view.upgrade())
                else {
                    return;
                };
                view.update(cx, |this, cx| {
                    if this.width != Some(width) {
                        this.width = Some(width);
                        cx.notify();
                    }
                });
            })
            .child(
                div()
                    .flex()
                    .when(focused, |d| d.flex_wrap())
                    .items_center()
                    .gap(px(4.))
                    .w_full()
                    .min_h(px(24.))
                    .key_context(RECIPIENTS_CONTEXT)
                    .capture_key_down(cx.listener(|this, e, window, cx| this.on_key(e, window, cx)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| this.focus(window, cx)),
                    )
                    .children(chips)
                    .when(hidden > 0, |d| {
                        d.child(
                            div()
                                .id(("compose-more", field))
                                .test_support()
                                .flex_none()
                                .h(px(24.))
                                .px(px(8.))
                                .flex()
                                .items_center()
                                .rounded(px(6.))
                                .border_1()
                                .border_color(border)
                                .bg(field_bg)
                                .text_size(px(11.))
                                .text_color(muted)
                                .child(format!("+{hidden}")),
                        )
                    })
                    .when(!focused, |d| {
                        // A quiet glyph at text height: a boxed button beside words reads as a
                        // control, not as part of the sentence.
                        d.child(
                            div()
                                .id(("compose-add", field))
                                .test_support()
                                .flex_none()
                                .size(px(20.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(5.))
                                .text_color(muted)
                                .cursor_pointer()
                                .hover(move |s| s.text_color(fg).bg(field_bg))
                                .tooltip(|window, cx| Tooltip::new("Add recipient").build(window, cx))
                                .child(Icon::new(gpui_kit::assets::IconName::Plus).with_size(px(14.))),
                        )
                    })
                    // Always mounted so it can take focus; collapsed out of sight while blurred.
                    .child(
                        div()
                            // Wide enough for most addresses; it grows into whatever the
                            // chips leave on its line.
                            .when(focused, |d| d.flex_1().min_w(px(200.)).h(px(24.)))
                            .when(!focused, |d| d.absolute().size(px(1.)).opacity(0.))
                            .child(
                                // `small` draws the field at chip height (24px) with the
                                // text on the chips' baseline.
                                Input::new(&self.input)
                                    .small()
                                    .bordered(false)
                                    .text_size(px(13.))
                                    .disabled(self.disabled),
                            ),
                    ),
            )
            .children(popup)
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
