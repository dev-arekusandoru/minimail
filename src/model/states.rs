use super::*;

impl Mailbox {
    /// Set `state` on every id in `ids`. One undo entry per call; unknown ids
    /// and ids already in `state` are skipped. Returns the number changed.
    /// Records no time metadata; see `set_state_at`.
    pub fn set_state(&mut self, ids: &[MessageId], state: TriageState) -> usize {
        self.set_state_inner(ids, state, None)
    }

    /// Like `set_state`, but stamps `waiting_since = now` on messages entering
    /// Waiting so `tick` can resurface them.
    pub fn set_state_at(&mut self, ids: &[MessageId], state: TriageState, now: Timestamp) -> usize {
        self.set_state_inner(ids, state, Some(now))
    }

    pub(super) fn set_state_inner(
        &mut self,
        ids: &[MessageId],
        state: TriageState,
        now: Option<Timestamp>,
    ) -> usize {
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
            meta.waiting_since = if state == TriageState::Waiting { now } else { None };
            meta.snoozed_until = None;
            meta.tags.retain(|t| *t != Tag::NoReply);
        }
        let changed = changes.len();
        self.push_undo(changes);
        changed
    }

    /// Set `state` on every message from `from_email`, whatever state it is in
    /// now. One undo entry per call; returns the number changed.
    pub fn set_state_for_sender(&mut self, from_email: &str, state: TriageState) -> usize {
        let ids: Vec<MessageId> = self
            .messages
            .iter()
            .filter(|m| m.from_email.eq_ignore_ascii_case(from_email))
            .map(|m| m.id)
            .collect();
        self.set_state(&ids, state)
    }
}
