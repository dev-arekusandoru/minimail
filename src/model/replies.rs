use super::*;

impl Mailbox {
    /// Record provider-confirmed delivery. Unlike filing, delivery cannot be undone.
    pub fn record_sent(
        &mut self,
        mut message: Message,
        original: Option<MessageId>,
        awaiting_reply: bool,
        now: Timestamp,
    ) -> MessageId {
        message.id = self
            .messages
            .iter()
            .map(|m| m.id)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .expect("message id space exhausted");
        message.outgoing = true;
        message.read = true;
        message.partial = false;
        let mut meta = Meta::default();
        if let Some(original) = original.and_then(|id| self.get(id)).cloned() {
            message.thread_id = original.thread_id;
            message.state = original.state;
            if original.state == TriageState::Snoozed {
                meta.snoozed_until = self.snoozed_until(original.id);
                message.snooze = original.snooze.clone();
            }
            for m in &self.messages {
                if m.thread_id == original.thread_id {
                    self.meta
                        .entry(m.id)
                        .or_default()
                        .tags
                        .retain(|tag| *tag != Tag::NeedsReply);
                }
            }
            let original_meta = self.meta.entry(original.id).or_default();
            original_meta.tags.retain(|tag| *tag != Tag::AwaitingReply);
            original_meta.awaiting_since = awaiting_reply.then_some(now);
            if awaiting_reply {
                original_meta.tags.push(Tag::AwaitingReply);
            }
        } else if message.state == TriageState::Snoozed {
            // A newly composed message has no existing wake time to inherit.
            message.state = TriageState::Archived;
            message.snooze = None;
        }
        if message.state != TriageState::Snoozed {
            message.snooze = None;
        }
        let id = message.id;
        self.index.insert(id, self.messages.len());
        self.newest_first.insert(0, id);
        self.meta.insert(id, meta);
        self.messages.push(message);
        id
    }

    /// File the original and confirmed outgoing messages together in one undo step.
    pub fn file_after_reply(&mut self, original: MessageId, state: TriageState) -> usize {
        assert_ne!(state, TriageState::Snoozed, "use snooze");
        let Some(thread) = self.get(original).map(|m| m.thread_id) else {
            return 0;
        };
        let ids: Vec<_> = self
            .messages
            .iter()
            .filter(|m| m.id == original || (m.outgoing && m.thread_id == thread))
            .filter(|m| m.state != state)
            .map(|m| m.id)
            .collect();
        let changes = ids.iter().map(|id| self.snapshot(*id)).collect();
        for id in &ids {
            self.set_raw(*id, state);
            self.meta
                .entry(*id)
                .or_default()
                .tags
                .retain(|tag| !matches!(tag, Tag::Reminder | Tag::FollowUp | Tag::PossibleSpam));
        }
        self.push_undo(changes);
        ids.len()
    }
}
