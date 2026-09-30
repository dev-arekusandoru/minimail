//! Reader tab operations: opening messages into the preview tab, pinning, switching, closing,
//! and dropping preview tabs whose thread left the list. Pure view state (`crate::tabs`): none
//! of it is an undo step. A triage session bypasses the tabs entirely (see `opened`).

use super::*;

impl MailApp {
    /// Open `id` in its thread's tab: the preview tab unless the thread already has one.
    /// `enter` (the `enter` key or a double-click) on the message the preview already shows
    /// pins it instead.
    pub(super) fn open_message(&mut self, id: MessageId, enter: bool) {
        if self.in_session() {
            return;
        }
        let Some(thread) = self.thread_of(id) else { return };
        let pin = enter && self.tabs.tab_for(thread).is_some_and(|t| !t.pinned && t.msg == id);
        if let Some(replaced) = self.tabs.open(thread, id) {
            self.forget_thread(replaced);
        }
        if pin {
            self.tabs.pin(thread);
        }
    }

    /// Open `id` and make its tab permanent (replying to it).
    pub(super) fn pin_message(&mut self, id: MessageId) {
        if self.in_session() {
            return;
        }
        let Some(thread) = self.thread_of(id) else { return };
        if let Some(replaced) = self.tabs.open_pinned(thread, id) {
            self.forget_thread(replaced);
        }
    }

    /// Make the tab of `id`'s thread permanent, if it has one.
    pub(super) fn pin_thread_of(&mut self, id: MessageId) {
        if let Some(thread) = self.thread_of(id) {
            self.tabs.pin(thread);
        }
    }

    /// A click inside the active tab's content pins it.
    pub(super) fn pin_active_tab(&mut self, cx: &mut Context<Self>) {
        if self.tabs.active().is_some_and(|t| !t.pinned) {
            self.tabs.pin_active();
            cx.notify();
        }
    }

    /// Everything the reader keeps for a thread that no longer has a tab.
    fn forget_thread(&mut self, thread: u32) {
        self.reader.forget(thread);
        self.drop_find(thread);
        self.reader_panes.borrow_mut().remove(&thread);
    }

    /// The list cursor follows the active tab's message when the list has it. Selection is
    /// untouched.
    fn follow_active_tab(&mut self) {
        if let Some(id) = self.tabs.opened() {
            self.focus_message(id, false);
            self.scroll_to_cursor();
        }
    }

    /// Click on tab `ix`: show it; a double-click pins it.
    pub(super) fn click_tab(&mut self, ix: usize, clicks: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.activate(ix) {
            if clicks >= 2 {
                self.tabs.pin_active();
            }
            self.follow_active_tab();
        }
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    pub(super) fn close_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(thread) = self.tabs.close(ix) {
            self.forget_thread(thread);
            self.follow_active_tab();
        }
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    /// `cmd-w`.
    pub(super) fn close_active_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs_locked() {
            return;
        }
        if let Some(ix) = self.tabs.active_index() {
            self.close_tab(ix, window, cx);
        }
    }

    /// `ctrl-tab` / `ctrl-shift-tab`: step to the next or previous tab, wrapping.
    pub(super) fn cycle_tab(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs_locked() {
            return;
        }
        if self.tabs.cycle(delta) {
            self.follow_active_tab();
            // The tab we left may have owned the focused find input.
            window.focus(&self.focus_handle, cx);
            cx.notify();
        }
    }

    /// Tab shortcuts do nothing behind a modal or menu, or while a session hides the tabs.
    pub(super) fn tabs_locked(&self) -> bool {
        self.modal_open() || self.menu_open() || self.in_session() || self.session_end.is_some()
    }

    /// Close preview tabs whose thread has no message in the list any more (triaged away, or
    /// the view changed); pinned tabs stay. Runs every frame but costs nothing without a
    /// preview, and never during a session, which hides the tabs.
    pub(super) fn reconcile_tabs(&mut self) {
        if !self.tabs.has_preview() || self.in_session() || self.session_end.is_some() {
            return;
        }
        let shown: HashSet<u32> =
            self.visible_ids().into_iter().filter_map(|id| self.thread_of(id)).collect();
        for thread in self.tabs.retain_previews(|t| shown.contains(&t)) {
            self.forget_thread(thread);
        }
    }
}
