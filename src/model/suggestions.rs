use super::*;

impl Mailbox {
    /// Store suggestions awaiting review; a newer one replaces an older
    /// pending suggestion for the same message and question.
    pub fn add_suggestions(&mut self, suggestions: Vec<Suggestion>) {
        for s in suggestions {
            self.pending
                .retain(|p| !(p.message == s.message && p.key == s.key));
            self.pending.push(s);
        }
    }

    pub fn pending(&self, id: MessageId) -> Vec<&Suggestion> {
        self.pending.iter().filter(|s| s.message == id).collect()
    }

    /// Apply every pending suggestion for `id`. One undo entry; returns the
    /// number accepted.
    pub fn accept_suggestions(&mut self, id: MessageId, now: Timestamp) -> usize {
        let (mine, rest): (Vec<_>, Vec<_>) =
            self.pending.iter().cloned().partition(|s| s.message == id);
        if mine.is_empty() {
            return 0;
        }
        let mut changes = vec![Change::Pending(self.pending.clone()), self.snapshot(id)];
        self.pending = rest;
        for s in &mine {
            self.apply(s, now);
        }
        changes.reverse();
        self.push_undo(changes);
        mine.len()
    }

    /// Drop every pending suggestion for `id`. One undo entry; returns the
    /// number dropped.
    pub fn reject_suggestions(&mut self, id: MessageId) -> usize {
        let before = self.pending.len();
        let snapshot = self.pending.clone();
        self.pending.retain(|s| s.message != id);
        let dropped = before - self.pending.len();
        if dropped > 0 {
            self.push_undo(vec![Change::Pending(snapshot)]);
        }
        dropped
    }

    /// Apply one suggestion immediately (System 1 auto mode). One undo entry.
    /// False if it was a no-op or the message is unknown.
    pub fn apply_auto(&mut self, s: Suggestion, now: Timestamp) -> bool {
        if !self.index.contains_key(&s.message) {
            return false;
        }
        let before = (
            self.snapshot(s.message),
            Change::Pending(self.pending.clone()),
        );
        self.pending
            .retain(|p| !(p.message == s.message && p.key == s.key));
        self.apply(&s, now);
        let changed = match (&before.0, self.snapshot(s.message)) {
            (Change::Msg(_, st, meta), Change::Msg(_, st2, meta2)) => {
                *st != st2 || *meta != meta2
            }
            _ => false,
        };
        if changed {
            self.push_undo(vec![before.1, before.0]);
        } else {
            // Nothing applied; still restore the pending list on failure.
            if let Change::Pending(p) = before.1 {
                self.pending = p;
            }
        }
        changed
    }

    /// Apply a suggestion's answer to its message (no undo bookkeeping).
    pub(super) fn apply(&mut self, s: &Suggestion, now: Timestamp) {
        let id = s.message;
        let Some(&i) = self.index.get(&id) else {
            return;
        };
        match (s.key, &s.answer.value) {
            (QuestionKey::SuggestedState, AnswerValue::Choice(n)) => {
                if let Some(state) = TriageState::ALL.get(*n).copied() {
                    self.apply_state(i, state, now);
                }
            }
            (QuestionKey::Spam, AnswerValue::Bool(true)) => {
                self.add_tag(id, Tag::Spam);
                self.apply_state(i, TriageState::Done, now);
            }
            (QuestionKey::NeedsReply, AnswerValue::Bool(true)) => self.add_tag(id, Tag::NeedsReply),
            (QuestionKey::Urgency, AnswerValue::Score(v)) => {
                let level = v.round().clamp(0.0, 255.0) as u8;
                self.meta
                    .entry(id)
                    .or_default()
                    .tags
                    .retain(|t| !matches!(t, Tag::Urgent(_)));
                self.add_tag(id, Tag::Urgent(level));
            }
            (QuestionKey::Kind, AnswerValue::Choice(n)) => {
                if let Some(kind) = Kind::from_index(*n) {
                    self.meta
                        .entry(id)
                        .or_default()
                        .tags
                        .retain(|t| !matches!(t, Tag::Kind(_)));
                    self.add_tag(id, Tag::Kind(kind));
                }
            }
            _ => {}
        }
    }

    pub(super) fn apply_state(&mut self, i: usize, state: TriageState, now: Timestamp) {
        if self.messages[i].state != state {
            self.messages[i].state = state;
            let meta = self.meta.entry(self.messages[i].id).or_default();
            meta.waiting_since = (state == TriageState::Waiting).then_some(now);
            meta.snoozed_until = None;
            meta.tags.retain(|t| *t != Tag::NoReply);
        }
    }

    pub(super) fn add_tag(&mut self, id: MessageId, tag: Tag) {
        let tags = &mut self.meta.entry(id).or_default().tags;
        if !tags.contains(&tag) {
            tags.push(tag);
        }
    }
}
