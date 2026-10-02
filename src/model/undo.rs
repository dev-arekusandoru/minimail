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

    /// Run `f` and fold every undo step it pushes into one, so a compound action
    /// (create a folder, then move mail into it) undoes in a single step.
    pub fn grouped<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        let mark = self.undo.len();
        let out = f(self);
        let pushed: Vec<UndoStep> = self.undo.split_off(mark);
        if !pushed.is_empty() {
            let changes = pushed.into_iter().flat_map(|step| step.changes).collect();
            self.undo.push(UndoStep { changes });
        }
        out
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
                Change::Known(e, was, created) => {
                    set_membership(&mut self.known, e.clone(), was);
                    if !was
                        && created
                        && let Some(store) = &self.contacts
                    {
                        let _ = store.forget_address(&e);
                    }
                }
                Change::Muted(t, was) => set_membership(&mut self.muted, t, was),
                Change::Blocked(e, Some(at)) => {
                    self.blocked.insert(e, at);
                }
                Change::Blocked(e, None) => {
                    self.blocked.remove(&e);
                }
                Change::Unsubscribed(e) => self.unsubscribed.retain(|u| *u != e),
                Change::FolderPush(id) => self.folders.retain(|f| f.id != id),
                Change::Pending(p) => self.pending = p,
            }
        }
    }
}
