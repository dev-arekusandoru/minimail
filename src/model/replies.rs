use super::*;

impl Mailbox {
    pub fn send_reply(&mut self, to: MessageId, body: String) {
        self.send_reply_at(to, body, false, 0);
    }
    pub fn send_reply_at(
        &mut self,
        to: MessageId,
        body: String,
        awaiting_reply: bool,
        now: Timestamp,
    ) {
        let Some(&i) = self.index.get(&to) else {
            return;
        };
        let original = self.messages[i].clone();
        let mut changes = vec![self.snapshot(to)];
        for m in &self.messages {
            if m.thread_id == original.thread_id
                && self
                    .meta
                    .get(&m.id)
                    .is_some_and(|meta| meta.tags.contains(&Tag::NeedsReply))
            {
                changes.push(self.snapshot(m.id));
                self.meta
                    .entry(m.id)
                    .or_default()
                    .tags
                    .retain(|t| *t != Tag::NeedsReply);
            }
        }
        let meta = self.meta.entry(to).or_default();
        meta.tags.retain(|t| *t != Tag::AwaitingReply);
        meta.awaiting_since = None;
        if awaiting_reply {
            meta.tags.push(Tag::AwaitingReply);
            meta.awaiting_since = Some(now);
        }
        let seq = self.next_seq;
        self.next_seq += 1;
        self.outbox.push(Outgoing {
            reply: Reply {
                in_reply_to: to,
                body,
            },
            due: now + OUTBOX_DELAY,
            seq,
        });
        changes.push(Change::Queued(seq));
        self.push_undo(changes);
    }
    pub fn file_after_reply(&mut self, original: MessageId, state: TriageState) -> usize {
        assert_ne!(state, TriageState::Snoozed, "use snooze");
        let Some(m) = self.get(original).cloned() else {
            return 0;
        };
        let ids: Vec<_> = self
            .messages
            .iter()
            .filter(|x| x.id == original || (x.outgoing && x.thread_id == m.thread_id))
            .map(|x| x.id)
            .collect();
        let mut changes = Vec::new();
        for id in &ids {
            if let Some(&i) = self.index.get(id)
                && self.messages[i].state != state
            {
                changes.push(self.snapshot(*id));
                self.messages[i].state = state;
                let meta = self.meta.entry(*id).or_default();
                meta.snoozed_until = None;
                meta.awaiting_since = None;
                meta.tags.retain(|t| {
                    !matches!(t, Tag::Reminder | Tag::FollowUp | Tag::PossibleSpam)
                });
            }
        }
        let changed = changes.len();
        self.push_undo(changes);
        changed
    }
    pub fn sent(&self) -> &[Reply] {
        &self.sent
    }
    pub fn outbox(&self) -> &[Outgoing] {
        &self.outbox
    }
    pub fn recall_last(&mut self, _now: Timestamp) -> Option<Reply> {
        let seq = self.outbox.last()?.seq;
        let pos = self.undo.iter().rposition(|s| {
            s.changes
                .iter()
                .any(|c| matches!(c, Change::Queued(q) if *q == seq))
        })?;
        let step = self.undo.remove(pos);
        let reply = self.outbox.last().map(|o| o.reply.clone());
        self.revert(step);
        reply
    }
}
