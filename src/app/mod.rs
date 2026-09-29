//! GPUI views for the mail client.
//!
//! `gpui-kit` re-exports GPUI at its root, so `use gpui_kit::*` *is* GPUI.

use gpui_kit::component::button::Button;
use gpui_kit::*;

/// Keymap context declared on the top-level mail view.
pub const KEY_CONTEXT: &str = "MailApp";

// `gpui_kit::actions!` declares unit actions with a namespace, so these land in
// the keymap as `mail::SelectNext`.
gpui_kit::actions!(mail, [SelectNext]);

/// Placeholder view proving the window/Root/action/focus wiring works.
pub struct MailApp {
    focus_handle: FocusHandle,
    /// Incremented by `SelectNext`; asserted by `tests/ui_spike.rs`.
    pub select_next_count: usize,
}

impl MailApp {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        cx.bind_keys([KeyBinding::new("j", SelectNext, Some(KEY_CONTEXT))]);
        Self {
            focus_handle,
            select_next_count: 0,
        }
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus_handle
    }
}

impl Render for MailApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .size_full()
            .on_action(cx.listener(|this, _: &SelectNext, _window, cx| {
                this.select_next_count += 1;
                cx.notify();
            }))
            .child(Button::new("ok").label(format!("j pressed {} times", self.select_next_count)))
    }
}
