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
        let _ = self.queue_reply_at(to, body, awaiting_reply, now);
    }
    /// Queue a reply like [`Self::send_reply_at`], returning its outbox sequence (or `None` if
    /// the message is gone). The classifier needs the sequence to know whether *this* send is
    /// still in flight when its answer lands.
    pub fn queue_reply_at(
        &mut self,
        to: MessageId,
        body: String,
        awaiting_reply: bool,
        now: Timestamp,
    ) -> Option<u64> {
        let &i = self.index.get(&to)?;
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
        Some(seq)
    }
    pub fn file_after_reply(&mut self, original: MessageId, state: TriageState) -> usize {
        assert_ne!(state, TriageState::Snoozed, "use snooze");
        let Some(m) = self.get(original).cloned() else {
            return 0;
        };
        // What a reply materialised from the outbox would have had: the original's
        // state before this filing.
        let before = m.state;
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
        // Replies still in the outbox materialise with this state, and the same undo
        // step has to reach them once they are sent.
        let queued: Vec<u64> = self
            .outbox
            .iter()
            .filter(|o| {
                self.get(o.reply.in_reply_to)
                    .is_some_and(|x| x.thread_id == m.thread_id)
            })
            .map(|o| o.seq)
            .collect();
        for seq in queued {
            let was = self.post_send.insert(seq, state);
            changes.push(Change::PostSend(seq, was, before));
        }
        let changed = changes
            .iter()
            .filter(|c| matches!(c, Change::Msg(..)))
            .count();
        self.push_undo(changes);
        changed
    }
    pub fn sent(&self) -> &[Reply] {
        &self.sent
    }
    pub fn outbox(&self) -> &[Outgoing] {
        &self.outbox
    }
    /// Whether the send with this outbox sequence is still queued or sent: false once it was
    /// recalled or undone. The sequence, not the message, is what a reply judge must check.
    pub fn send_in_flight(&self, seq: u64) -> bool {
        self.outbox.iter().any(|o| o.seq == seq) || self.sent_ids.contains_key(&seq)
    }
    /// Tag `original`'s thread *Awaiting Reply* as `send_reply_at(.., true, ..)` would, but
    /// without pushing an undo step: the send that queued it already pushed one (whose snapshot
    /// predates this tag, so undoing the send removes it). Does nothing once that send is gone.
    pub fn mark_awaiting_reply(&mut self, original: MessageId, seq: u64, now: Timestamp) -> bool {
        if !self.send_in_flight(seq) {
            return false;
        }
        let Some(meta) = self.meta.get_mut(&original) else {
            return false;
        };
        if !meta.tags.contains(&Tag::AwaitingReply) {
            meta.tags.push(Tag::AwaitingReply);
        }
        meta.awaiting_since = Some(now);
        true
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
