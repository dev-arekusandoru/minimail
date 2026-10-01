use super::*;
use crate::app::chrome::help_width;

impl MailApp {
    /// `?`: open the shortcut panel in a kit dialog, or close it if it is already open.
    pub(super) fn toggle_help(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.help.is_some() {
            self.close_modals(window, cx);
            return;
        }
        if self.modal_open() {
            return;
        }
        let scroll = self.help_scroll.clone();
        let panel = cx.new(|cx| HelpPanel::new(scroll, cx));
        self._modal_sub = Some(cx.subscribe_in(
            &panel,
            window,
            |this, _, _: &HelpEvent, window, cx| this.close_modals(window, cx),
        ));
        self.help = Some(panel.clone());
        self.host_in_dialog(panel.clone(), help_width, window, cx);
        window.focus(&panel.focus_handle(cx), cx);
        cx.notify();
    }

    /// Help body viewport bounds and how far it can scroll (for headless tests).
    pub fn help_metrics(&self) -> (Bounds<Pixels>, f32) {
        (self.help_scroll.bounds(), f32::from(self.help_scroll.max_offset().y))
    }
}
