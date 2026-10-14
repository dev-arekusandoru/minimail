use super::*;

impl Mailbox {
    pub fn add_suggestions(&mut self, suggestions: Vec<Suggestion>) {
        for s in suggestions {
            self.pending
                .retain(|p| !(p.message == s.message && p.key == s.key));
            self.pending.push(s);
        }
    }
    /// Drop pending suggestions whose question is Off in `policy` (mode changes apply at once).
    pub fn drop_suggestions_off(&mut self, policy: &crate::judge::JudgePolicy) {
        self.pending
            .retain(|s| policy.mode(s.key) != crate::judge::Mode::Off);
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
        let mut changes = vec![Change::Pending(before), self.snapshot(id)];
        changes.reverse();
        // A Folder suggestion files the message via `set_state`; fold that into this one step.
        self.grouped(|mb| {
            mb.pending = rest;
            for s in &mine {
                mb.apply(s, now);
            }
            mb.push_undo(changes);
        });
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
    /// Apply a classifier label without pushing an undo step: background labels are not user
    /// actions, so `u` never undoes one (see AGENTS.md invariants).
    pub fn apply_auto(&mut self, s: Suggestion, now: Timestamp) -> bool {
        if !self.index.contains_key(&s.message) {
            return false;
        }
        let before = self.meta.get(&s.message).cloned().unwrap_or_default();
        self.apply(&s, now);
        before != self.meta.get(&s.message).cloned().unwrap_or_default()
    }
    pub(super) fn apply(&mut self, s: &Suggestion, _now: Timestamp) {
        let id = s.message;
        if !self.index.contains_key(&id) || self.is_overridden(id, s.key) {
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
            // "None"/no folder is a no-op; a real folder files the message like manual filing.
            (QuestionKey::Folder, AnswerValue::Choice(_)) => {
                if let Some(folder) = self.suggested_folder(s) {
                    self.set_state(&[id], TriageState::Filed(folder));
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

    /// Whether the user set `key`'s label by hand: reclassification leaves it alone until
    /// [`Self::reset_label`].
    pub fn is_overridden(&self, id: MessageId, key: QuestionKey) -> bool {
        self.meta.get(&id).is_some_and(|m| m.overrides.contains(&key))
    }

    /// Set `key`'s label from the user: the tag becomes `tag` (or is cleared) and sticks, so a
    /// later classification never overwrites it. One undo step.
    pub fn set_label(&mut self, id: MessageId, key: QuestionKey, tag: Option<Tag>) -> bool {
        if !self.index.contains_key(&id) {
            return false;
        }
        let before = self.meta.get(&id).cloned().unwrap_or_default();
        let meta = self.meta.entry(id).or_default();
        if !meta.overrides.contains(&key) {
            meta.overrides.push(key);
        }
        replace_tag(&mut meta.tags, key, tag);
        self.label_undo(id, before)
    }

    /// Drop the user's override on `key` and put the classifier's `tag` back. One undo step;
    /// false when the key was never set by hand.
    pub fn reset_label(&mut self, id: MessageId, key: QuestionKey, tag: Option<Tag>) -> bool {
        if !self.is_overridden(id, key) {
            return false;
        }
        let before = self.meta.get(&id).cloned().unwrap_or_default();
        let meta = self.meta.entry(id).or_default();
        meta.overrides.retain(|k| *k != key);
        replace_tag(&mut meta.tags, key, tag);
        self.label_undo(id, before)
    }

    /// One undo step for a label change on `id`; false when the tags did not move.
    fn label_undo(&mut self, id: MessageId, before: Meta) -> bool {
        if before == self.meta.get(&id).cloned().unwrap_or_default() {
            return false;
        }
        self.push_undo(vec![Change::Msg(id, self.state_of(id).unwrap_or_default(), before)]);
        true
    }
}

/// Replace `key`'s tag with `tag` (`None` clears it); every other tag is left alone.
fn replace_tag(tags: &mut Vec<Tag>, key: QuestionKey, tag: Option<Tag>) {
    tags.retain(|t| !matches!(
        (key, t),
        (QuestionKey::Spam, Tag::PossibleSpam)
            | (QuestionKey::Urgency, Tag::Urgent(_))
            | (QuestionKey::Kind, Tag::Kind(_))
    ));
    if let Some(tag) = tag {
        tags.push(tag);
    }
}
