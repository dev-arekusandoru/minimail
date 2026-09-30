use super::*;

impl Mailbox {
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
    pub fn accept_suggestions(&mut self, id: MessageId, now: Timestamp) -> usize {
        let before = self.pending.clone();
        let (mine, rest): (Vec<_>, Vec<_>) = before.iter().cloned().partition(|s| s.message == id);
        if mine.is_empty() {
            return 0;
        }
        self.pending = rest;
        let mut changes = vec![Change::Pending(before), self.snapshot(id)];
        for s in &mine {
            self.apply(s, now);
        }
        changes.reverse();
        self.push_undo(changes);
        mine.len()
    }
    pub fn reject_suggestions(&mut self, id: MessageId) -> usize {
        let before = self.pending.clone();
        let rest: Vec<_> = before.iter().filter(|s| s.message != id).cloned().collect();
        if before.len() == rest.len() {
            return 0;
        }
        self.push_undo(vec![Change::Pending(before)]);
        let dropped = self.pending.len() - rest.len();
        self.pending = rest;
        dropped
    }
    pub fn apply_auto(&mut self, s: Suggestion, now: Timestamp) -> bool {
        if !self.index.contains_key(&s.message) {
            return false;
        }
        let old = self.snapshot(s.message);
        let before = self.meta.get(&s.message).cloned().unwrap_or_default();
        self.apply(&s, now);
        let changed = before != self.meta.get(&s.message).cloned().unwrap_or_default();
        if changed {
            self.push_undo(vec![old]);
        }
        changed
    }
    pub(super) fn apply(&mut self, s: &Suggestion, _now: Timestamp) {
        let id = s.message;
        if !self.index.contains_key(&id) {
            return;
        }
        match (s.key, &s.answer.value) {
            (QuestionKey::Spam, AnswerValue::Bool(true)) => self.add_tag(id, Tag::PossibleSpam),
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
    pub(super) fn add_tag(&mut self, id: MessageId, tag: Tag) {
        let tags = &mut self.meta.entry(id).or_default().tags;
        if !tags.contains(&tag) {
            tags.push(tag);
        }
    }
}
