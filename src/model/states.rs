use super::*;

impl Mailbox {
    /// Set `state` on every id in `ids`. One undo entry per call; unknown ids
    /// and ids already in `state` are skipped. Returns the number changed.
    /// Records no time metadata; see `set_state_at`.
    pub fn set_state(&mut self, ids: &[MessageId], state: TriageState) -> usize {
        assert_ne!(state, TriageState::Snoozed, "use snooze");
        let mut changes = Vec::new();
        for id in ids {
            let Some(&i) = self.index.get(id) else {
                continue;
            };
            if self.messages[i].state == state {
                continue;
            }
            changes.push(self.snapshot(*id));
            self.messages[i].state = state;
            let meta = self.meta.entry(*id).or_default();
            meta.snoozed_until = None;
            meta.awaiting_since = None;
            meta.tags
                .retain(|tag| !matches!(tag, Tag::Reminder | Tag::FollowUp | Tag::PossibleSpam));
        }
        let changed = changes.len();
        self.push_undo(changes);
        changed
    }

    pub fn set_state_for_sender(
        &mut self,
        email: &str,
        only_inbox: bool,
        state: TriageState,
    ) -> usize {
        let ids: Vec<_> = self
            .messages
            .iter()
            .filter(|m| {
                m.from_email.eq_ignore_ascii_case(email)
                    && (!only_inbox || m.state == TriageState::Inbox)
            })
            .map(|m| m.id)
            .collect();
        self.set_state(&ids, state)
    }

    pub fn mark_spam(&mut self, ids: &[MessageId], block: bool) -> usize {
        let mut changes = Vec::new();
        let mut senders = HashSet::new();
        for id in ids {
            let Some(m) = self.get(*id).cloned() else {
                continue;
            };
            if block {
                senders.insert(lower(&m.from_email));
            }
            if m.state != TriageState::Deleted {
                changes.push(self.snapshot(*id));
                if let Some(&i) = self.index.get(id) {
                    self.messages[i].state = TriageState::Deleted;
                }
                let meta = self.meta.entry(*id).or_default();
                meta.snoozed_until = None;
                meta.awaiting_since = None;
                meta.tags
                    .retain(|t| !matches!(t, Tag::Reminder | Tag::FollowUp | Tag::PossibleSpam));
            }
        }
        for email in senders {
            if self.blocked.insert(email.clone()) {
                changes.push(Change::Blocked(email, false));
            }
        }
        let changed = changes
            .iter()
            .filter(|c| matches!(c, Change::Msg(..)))
            .count();
        self.push_undo(changes);
        changed
    }
}
