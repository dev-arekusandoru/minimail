use super::*;

impl Mailbox {
    /// Record a reply to `to` and move the original to `Waiting`, sent
    /// immediately. Exactly one undo entry, which also retracts the sent
    /// reply. A no-op (no entry, no sent reply) if `to` is unknown.
    pub fn send_reply(&mut self, to: MessageId, body: String) {
        let Some(mut changes) = self.enter_waiting(to, None) else {
            return;
        };
        self.sent.push(Reply {
            in_reply_to: to,
            body,
        });
        changes.push(Change::SentPush);
        self.push_undo(changes);
    }

    /// Queue a reply in the outbox (due in `OUTBOX_DELAY`s) and move the
    /// original to Waiting now. One undo entry.
    pub fn send_reply_at(&mut self, to: MessageId, body: String, now: Timestamp) {
        let Some(mut changes) = self.enter_waiting(to, Some(now)) else {
            return;
        };
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

    /// Move `to` to Waiting; returns the undo changes, `None` if unknown id.
    pub(super) fn enter_waiting(&mut self, to: MessageId, now: Option<Timestamp>) -> Option<Vec<Change>> {
        let &i = self.index.get(&to)?;
        let changes = vec![self.snapshot(to)];
        self.messages[i].state = TriageState::Waiting;
        let meta = self.meta.entry(to).or_default();
        meta.waiting_since = now;
        meta.snoozed_until = None;
        meta.tags.retain(|t| *t != Tag::NoReply);
        Some(changes)
    }

    pub fn sent(&self) -> &[Reply] {
        &self.sent
    }

    pub fn outbox(&self) -> &[Outgoing] {
        &self.outbox
    }

    /// Pull the newest reply back out of the outbox, undoing the send that
    /// queued it (original's prior state restored). `None` if the outbox is
    /// empty.
    pub fn recall_last(&mut self, _now: Timestamp) -> Option<Reply> {
        let seq = self.outbox.last()?.seq;
        let pos = self
            .undo
            .iter()
            .rposition(|s| s.changes.iter().any(|c| matches!(c, Change::Queued(q) if *q == seq)))?;
        let step = self.undo.remove(pos);
        let reply = self.outbox.last().map(|o| o.reply.clone());
        self.revert(step);
        reply
    }
}
