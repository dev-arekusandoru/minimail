use super::*;

impl Mailbox {
    pub(super) fn snapshot(&self, id: MessageId) -> Change {
        Change::Msg(
            id,
            self.state_of(id).unwrap_or_default(),
            self.meta.get(&id).cloned().unwrap_or_default(),
        )
    }

    /// Push one undo step; empty change lists are ignored.
    pub(super) fn push_undo(&mut self, changes: Vec<Change>) {
        if !changes.is_empty() {
            self.undo.push(UndoStep { changes });
        }
    }

    /// Undo the last mutation. Returns false when there is nothing to undo.
    pub fn undo(&mut self) -> bool {
        let Some(step) = self.undo.pop() else {
            return false;
        };
        self.revert(step);
        true
    }

    /// Apply a step's inverse changes, newest first.
    pub(super) fn revert(&mut self, step: UndoStep) {
        for change in step.changes.into_iter().rev() {
            match change {
                Change::Msg(id, state, meta) => {
                    if let Some(&i) = self.index.get(&id) {
                        self.messages[i].state = state;
                    }
                    if meta == Meta::default() {
                        self.meta.remove(&id);
                    } else {
                        self.meta.insert(id, meta);
                    }
                }
                Change::Muted(t, was) => set_membership(&mut self.muted, t, was),
                Change::Known(e, was) => set_membership(&mut self.known, e, was),
                Change::Blocked(e, was) => set_membership(&mut self.blocked, e, was),
                Change::Unsubscribed(e) => self.unsubscribed.retain(|u| *u != e),
                Change::Pending(p) => self.pending = p,
                Change::Queued(seq) => self.outbox.retain(|o| o.seq != seq),
                Change::SentPush => {
                    self.sent.pop();
                }
            }
        }
    }
}
