//! Reader disclosure state: which thread messages, recipient lists and quoted blocks are open,
//! and which messages are shown in Reader mode. Pure view state: none of it is an undo step.

use super::*;

impl MailApp {
    pub(super) fn thread_of(&self, id: MessageId) -> Option<u32> {
        self.mailbox.get(id).map(|m| m.thread_id)
    }

    /// Expand or collapse one earlier/later message of the opened thread.
    pub(super) fn toggle_reader_expanded(&mut self, id: MessageId, cx: &mut Context<Self>) {
        if let Some(thread) = self.thread_of(id) {
            self.reader.toggle_expanded(thread, id);
            self.tabs.pin(thread);
            cx.notify();
        }
    }

    /// Expand every other message of the opened thread, or collapse them all when they are
    /// already all expanded.
    pub(super) fn toggle_thread_expansion(&mut self, cx: &mut Context<Self>) {
        let Some(opened) = self.opened().and_then(|id| self.mailbox.get(id)) else {
            return;
        };
        let thread = opened.thread_id;
        let others = crate::reading::thread_others(self.mailbox.messages(), opened);
        if others.is_empty() {
            return;
        }
        self.reader.toggle_all(thread, &others);
        self.tabs.pin(thread);
        cx.notify();
    }

    /// Switch one message between its original HTML and Reader mode. Plain-text messages have
    /// nothing to switch.
    pub(super) fn toggle_reader_mode(&mut self, id: MessageId, cx: &mut Context<Self>) {
        let Some(msg) = self.mailbox.get(id) else {
            return;
        };
        if msg.html.is_none() {
            return;
        }
        let thread = msg.thread_id;
        self.reader.toggle_plain(thread, id);
        cx.notify();
    }

    /// Show or hide the To/Cc/Bcc rows under a message header.
    pub(super) fn toggle_recipients(&mut self, id: MessageId, cx: &mut Context<Self>) {
        if let Some(thread) = self.thread_of(id) {
            self.reader.toggle_recipients(thread, id);
            cx.notify();
        }
    }

    /// Show or hide the quoted history under a message body.
    pub(super) fn toggle_quoted(&mut self, id: MessageId, cx: &mut Context<Self>) {
        if let Some(thread) = self.thread_of(id) {
            self.reader.toggle_quoted(thread, id);
            cx.notify();
        }
    }

    /// `v`: Reader mode for the opened message.
    pub(super) fn toggle_opened_reader_mode(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.opened() {
            self.toggle_reader_mode(id, cx);
        }
    }

    /// Whether `id` is expanded in its thread's reader.
    pub fn reader_expanded(&self, id: MessageId) -> bool {
        self.thread_of(id).is_some_and(|t| self.reader.is_expanded(t, id))
    }

    /// Whether `id` is shown in Reader mode.
    pub fn reader_plain(&self, id: MessageId) -> bool {
        self.thread_of(id).is_some_and(|t| self.reader.plain(t, id))
    }

    /// Whether `id`'s recipient rows are open.
    pub fn reader_recipients_open(&self, id: MessageId) -> bool {
        self.thread_of(id).is_some_and(|t| self.reader.recipients_open(t, id))
    }

    /// Whether `id`'s quoted history is open.
    pub fn reader_quoted_open(&self, id: MessageId) -> bool {
        self.thread_of(id).is_some_and(|t| self.reader.quoted_open(t, id))
    }
}
