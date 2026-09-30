use super::*;

impl Mailbox {
    pub fn tags(&self, id: MessageId) -> &[Tag] {
        self.meta.get(&id).map_or(&[], |m| &m.tags)
    }

    pub fn waiting_since(&self, id: MessageId) -> Option<Timestamp> {
        self.meta.get(&id).and_then(|m| m.waiting_since)
    }

    pub fn snoozed_until(&self, id: MessageId) -> Option<Timestamp> {
        self.meta.get(&id).and_then(|m| m.snoozed_until)
    }

    /// Move `ids` to Later until `until`. One undo entry; returns the number
    /// changed (already snoozed until the same time counts as unchanged).
    pub fn snooze(&mut self, ids: &[MessageId], until: Timestamp, _now: Timestamp) -> usize {
        let mut changes = Vec::new();
        for id in ids {
            let Some(&i) = self.index.get(id) else {
                continue;
            };
            if self.messages[i].state == TriageState::Later
                && self.snoozed_until(*id) == Some(until)
            {
                continue;
            }
            changes.push(self.snapshot(*id));
            self.messages[i].state = TriageState::Later;
            let meta = self.meta.entry(*id).or_default();
            meta.waiting_since = None;
            meta.snoozed_until = Some(until);
            meta.tags.retain(|t| *t != Tag::NoReply);
        }
        let changed = changes.len();
        self.push_undo(changes);
        changed
    }

    /// Advance time-driven state: resurface stale Waiting mail, wake due
    /// snoozes, flush due outbox replies. Idempotent; pushes no undo steps.
    pub fn tick(&mut self, now: Timestamp) -> TickReport {
        let mut report = TickReport::default();
        let mut woken = Vec::new();
        let mut stale = Vec::new();
        for m in &self.messages {
            let meta = self.meta.get(&m.id);
            match m.state {
                TriageState::Later => {
                    if meta.and_then(|x| x.snoozed_until).is_some_and(|t| t <= now) {
                        woken.push(m.id);
                    }
                }
                TriageState::Waiting => {
                    if let Some(since) = meta.and_then(|x| x.waiting_since)
                        && now - since >= WAITING_TIMEOUT
                        && !self.has_newer_reply(m, since)
                    {
                        stale.push(m.id);
                    }
                }
                _ => {}
            }
        }
        for id in woken {
            self.set_raw(id, TriageState::Inbox);
            report.woken.push(id);
        }
        for id in stale {
            self.set_raw(id, TriageState::Inbox);
            self.meta.entry(id).or_default().tags.push(Tag::NoReply);
            report.resurfaced.push(id);
        }
        let (due, keep): (Vec<_>, Vec<_>) =
            std::mem::take(&mut self.outbox).into_iter().partition(|o| o.due <= now);
        self.outbox = keep;
        report.flushed = due.len();
        self.sent.extend(due.into_iter().map(|o| o.reply));
        report
    }

    /// A later message in the same thread, received after `since`.
    pub(super) fn has_newer_reply(&self, waiting: &Message, since: Timestamp) -> bool {
        self.messages.iter().any(|m| {
            m.id != waiting.id
                && m.thread_id == waiting.thread_id
                && parse_rfc3339(&m.received).is_some_and(|t| t > since)
        })
    }

    /// State change from `tick`: clears time metadata, no undo.
    pub(super) fn set_raw(&mut self, id: MessageId, state: TriageState) {
        if let Some(&i) = self.index.get(&id) {
            self.messages[i].state = state;
        }
        let meta = self.meta.entry(id).or_default();
        meta.waiting_since = None;
        meta.snoozed_until = None;
    }

    /// Tonight 18:00, Tomorrow 08:00, next Monday 08:00 (all UTC). Tonight
    /// rolls to tomorrow 18:00 once 18:00 has passed.
    pub fn snooze_presets(&self, now: Timestamp) -> [(&'static str, Timestamp); 3] {
        let today = now.div_euclid(DAY) * DAY;
        let tonight = if now < today + 18 * HOUR {
            today + 18 * HOUR
        } else {
            today + DAY + 18 * HOUR
        };
        // 1970-01-01 was a Thursday; Monday = 0.
        let weekday = (now.div_euclid(DAY) + 3).rem_euclid(7);
        let monday = today + (7 - weekday) * DAY + 8 * HOUR;
        [
            ("Tonight", tonight),
            ("Tomorrow", today + DAY + 8 * HOUR),
            ("Monday", monday),
        ]
    }
}
