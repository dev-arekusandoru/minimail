use super::*;

impl Mailbox {
    /// Muted thread, blocked sender or unsubscribed sender.
    pub(super) fn is_hidden_msg(&self, m: &Message) -> bool {
        let email = lower(&m.from_email);
        self.muted.contains(&m.thread_id)
            || self.blocked.contains(&email)
            || self.unsubscribed.iter().any(|u| lower(u) == email)
    }

    pub(super) fn is_screened_msg(&self, m: &Message) -> bool {
        !self.is_hidden_msg(m) && !self.known.contains(&lower(&m.from_email))
    }

    pub(super) fn is_visible_msg(&self, m: &Message) -> bool {
        !self.is_hidden_msg(m) && self.known.contains(&lower(&m.from_email))
    }

    /// Muted / blocked / unsubscribed (not merely unscreened).
    pub fn is_hidden(&self, id: MessageId) -> bool {
        self.get(id).is_some_and(|m| self.is_hidden_msg(m))
    }

    /// In the Screener: unscreened sender, not otherwise hidden.
    pub fn is_screened(&self, id: MessageId) -> bool {
        self.get(id).is_some_and(|m| self.is_screened_msg(m))
    }

    /// Visible messages in `state`, newest first.
    pub fn ids_in(&self, state: TriageState) -> Vec<MessageId> {
        self.newest_first
            .iter()
            .copied()
            .filter(|id| {
                self.get(*id)
                    .is_some_and(|m| m.state == state && self.is_visible_msg(m))
            })
            .collect()
    }

    /// Visible messages in `state`.
    pub fn count(&self, state: TriageState) -> usize {
        self.messages
            .iter()
            .filter(|m| m.state == state && self.is_visible_msg(m))
            .count()
    }

    /// Messages from unscreened senders, newest first.
    pub fn screener_ids(&self) -> Vec<MessageId> {
        self.newest_first
            .iter()
            .copied()
            .filter(|id| self.is_screened(*id))
            .collect()
    }

    pub fn hidden_count(&self) -> usize {
        self.messages
            .iter()
            .filter(|m| self.is_hidden_msg(m))
            .count()
    }

    /// Let `email` through the Screener and, when there is an address book,
    /// remember the sender in it so the decision survives a restart. False if
    /// the sender is already known.
    pub fn allow_sender(&mut self, email: &str) -> bool {
        let e = lower(email);
        if !self.known.insert(e.clone()) {
            return false;
        }
        // Undo removes exactly what this call added: a contact the Screener
        // created disappears with it, an existing one only loses the address.
        let created = self
            .contacts
            .as_ref()
            .and_then(|store| {
                store
                    .upsert_from_email(&e, self.sender_name(&e), ContactSource::Screener)
                    .ok()
            })
            .is_some_and(|(_, created)| created);
        self.push_undo(vec![Change::Known(e, false, created)]);
        true
    }

    /// Display name of the newest message from `email`, for the address book.
    fn sender_name(&self, email: &str) -> Option<&str> {
        self.newest_first
            .iter()
            .filter_map(|id| self.get(*id))
            .find(|m| lower(&m.from_email) == email)
            .map(|m| m.from_name.as_str())
    }

    /// Hide every message from `email` (not deleted). False if already blocked.
    pub fn block_sender(&mut self, email: &str) -> bool {
        let e = lower(email);
        if !self.blocked.insert(e.clone()) {
            return false;
        }
        self.push_undo(vec![Change::Blocked(e, false)]);
        true
    }

    pub fn mute_thread(&mut self, thread_id: u32) -> bool {
        if !self.muted.insert(thread_id) {
            return false;
        }
        self.push_undo(vec![Change::Muted(thread_id, false)]);
        true
    }

    pub fn is_muted(&self, thread_id: u32) -> bool {
        self.muted.contains(&thread_id)
    }

    /// Simulated unsubscribe: hide the sender and remember it.
    pub fn unsubscribe(&mut self, email: &str) -> bool {
        let e = lower(email);
        if self.unsubscribed.iter().any(|u| lower(u) == e) {
            return false;
        }
        self.unsubscribed.push(email.to_string());
        self.push_undo(vec![Change::Unsubscribed(email.to_string())]);
        true
    }

    pub fn unsubscribed(&self) -> &[String] {
        &self.unsubscribed
    }
}
