use super::*;

impl MailApp {
    pub(super) fn close_help(&mut self, cx: &mut Context<Self>) {
        self.help = false;
        cx.notify();
    }

    /// Scrolls the help body by `lines` text lines (positive = down), clamped to its extent.
    pub(super) fn scroll_help_lines(&mut self, lines: f32, cx: &mut Context<Self>) {
        self.scroll_help_by(lines * 28., cx);
    }

    /// Scrolls the help body by `pages` viewport heights (positive = down).
    pub(super) fn scroll_help(&mut self, pages: f32, cx: &mut Context<Self>) {
        if !self.help {
            return;
        }
        let page = f32::from(self.help_scroll.bounds().size.height) * 0.9;
        self.scroll_help_by(pages * page, cx);
    }

    pub(super) fn scroll_help_by(&mut self, dy: f32, cx: &mut Context<Self>) {
        let max = f32::from(self.help_scroll.max_offset().y);
        let y = (f32::from(self.help_scroll.offset().y) - dy).clamp(-max, 0.);
        self.help_scroll.set_offset(point(px(0.), px(y)));
        cx.notify();
    }

    /// Help body viewport bounds and how far it can scroll (for headless tests).
    pub fn help_metrics(&self) -> (Bounds<Pixels>, f32) {
        (self.help_scroll.bounds(), f32::from(self.help_scroll.max_offset().y))
    }
}
