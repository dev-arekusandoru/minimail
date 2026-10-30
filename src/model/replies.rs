use super::*;
use crate::outgoing::SendIntent;

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
            original_meta.follow_up_after = None;
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

    /// Leave `target`'s thread awaiting a reply: it resurfaces after `days` unless one
    /// arrives (`tick`). Not an undo step — it follows a send, which cannot be undone.
    pub fn await_reply(&mut self, target: MessageId, days: u32, now: Timestamp) {
        if self.get(target).is_none() {
            return;
        }
        let meta = self.meta.entry(target).or_default();
        meta.awaiting_since = Some(now);
        meta.follow_up_after = Some(i64::from(days) * DAY);
        if !meta.tags.contains(&Tag::AwaitingReply) {
            meta.tags.push(Tag::AwaitingReply);
        }
    }

    /// What a confirmed send does to the thread (`target`: the original it answered, or the
    /// sent message itself for a new thread). Archive is one undo step.
    pub fn apply_send_intent(&mut self, target: MessageId, intent: SendIntent, now: Timestamp) {
        match intent {
            SendIntent::Archive => {
                self.file_after_reply(target, TriageState::Archived);
            }
            SendIntent::RemindIfNoReply { days } => self.await_reply(target, days, now),
            SendIntent::JustSend | SendIntent::SendLater { .. } => {}
        }
    }
}
