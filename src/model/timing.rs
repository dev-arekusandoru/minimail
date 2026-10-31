use super::*;

impl Mailbox {
    pub fn tags(&self, id: MessageId) -> &[Tag] {
        self.meta.get(&id).map_or(&[], |m| &m.tags)
    }

    pub fn awaiting_since(&self, id: MessageId) -> Option<Timestamp> {
        self.meta.get(&id).and_then(|m| m.awaiting_since)
    }

    /// This message's reminder in the shape the cache stores it.
    pub fn reminder(&self, id: MessageId) -> Reminder {
        let meta = self.meta.get(&id);
        Reminder {
            awaiting_since: meta.and_then(|m| m.awaiting_since),
            follow_up_after: meta.and_then(|m| m.follow_up_after),
            followed_up: self.tags(id).contains(&Tag::FollowUp),
        }
    }

    /// Put reminders read back from the cache into `Meta`.
    pub fn restore_reminders(&mut self, reminders: Vec<(MessageId, Reminder)>) {
        for (id, reminder) in reminders {
            if self.get(id).is_none() {
                continue;
            }
            let meta = self.meta.entry(id).or_default();
            meta.awaiting_since = reminder.awaiting_since;
            meta.follow_up_after = reminder.follow_up_after;
            if reminder.awaiting_since.is_some() && !meta.tags.contains(&Tag::AwaitingReply) {
                meta.tags.push(Tag::AwaitingReply);
            }
            if reminder.followed_up && !meta.tags.contains(&Tag::FollowUp) {
                meta.tags.push(Tag::FollowUp);
            }
        }
    }

    pub fn snoozed_until(&self, id: MessageId) -> Option<Timestamp> {
        self.meta.get(&id).and_then(|m| m.snoozed_until)
    }

    pub fn snooze(&mut self, ids: &[MessageId], until: Timestamp, _now: Timestamp) -> usize {
        let mut changes = Vec::new();
        for id in ids {
            let Some(&i) = self.index.get(id) else {
                continue;
            };
            if self.messages[i].state == TriageState::Snoozed
                && self.snoozed_until(*id) == Some(until)
            {
                continue;
            }
            changes.push(self.snapshot(*id));
            self.messages[i].state = TriageState::Snoozed;
            let meta = self.meta.entry(*id).or_default();
            meta.awaiting_since = None;
            meta.snoozed_until = Some(until);
            meta.tags
                .retain(|t| !matches!(t, Tag::Reminder | Tag::FollowUp | Tag::PossibleSpam));
        }
        let changed = changes.len();
        self.push_undo(changes);
        changed
    }

    pub fn tick(&mut self, now: Timestamp) -> TickReport {
        let mut report = TickReport::default();
        let mut wake = Vec::new();
        let mut follow = Vec::new();
        let mut answered = Vec::new();
        for m in &self.messages {
            let meta = self.meta.get(&m.id);
            if m.state == TriageState::Snoozed
                && meta.and_then(|x| x.snoozed_until).is_some_and(|t| t <= now)
            {
                wake.push(m.id);
            }
            if let Some(since) = meta.and_then(|x| x.awaiting_since) {
                if self.has_newer_reply(m, since) {
                    answered.push(m.id);
                } else if now
                    >= since
                        + meta
                            .and_then(|x| x.follow_up_after)
                            .unwrap_or(self.follow_up_timeout)
                {
                    follow.push(m.id);
                }
            }
        }
        for id in answered {
            let meta = self.meta.entry(id).or_default();
            meta.awaiting_since = None;
            meta.follow_up_after = None;
            meta.tags.retain(|t| *t != Tag::AwaitingReply);
        }
        for id in wake {
            self.set_raw(id, TriageState::Inbox);
            let tags = &mut self.meta.entry(id).or_default().tags;
            if !tags.contains(&Tag::Reminder) {
                tags.push(Tag::Reminder);
            }
            report.woken.push(id);
        }
        for id in follow {
            self.set_raw(id, TriageState::Inbox);
            let tags = &mut self.meta.entry(id).or_default().tags;
            tags.retain(|t| *t != Tag::AwaitingReply);
            if !tags.contains(&Tag::FollowUp) {
                tags.push(Tag::FollowUp);
            }
            report.followed_up.push(id);
        }
        report
    }
    /// A later message in the same thread, received after `since`.
    pub(super) fn has_newer_reply(&self, waiting: &Message, since: Timestamp) -> bool {
        self.messages.iter().any(|m| {
            m.id != waiting.id
                && !m.outgoing
                && m.thread_id == waiting.thread_id
                && parse_rfc3339(&m.received).is_some_and(|t| t > since)
        })
    }

    pub(super) fn set_raw(&mut self, id: MessageId, state: TriageState) {
        if let Some(&i) = self.index.get(&id) {
            self.messages[i].state = state;
        }
        let meta = self.meta.entry(id).or_default();
        meta.awaiting_since = None;
        meta.follow_up_after = None;
        meta.snoozed_until = None;
    }

    /// Tonight 18:00, Tomorrow 08:00, next Monday 08:00, each at that time on the user's
    /// own clock. Tonight rolls to tomorrow 18:00 once 18:00 has passed.
    pub fn snooze_presets(now: &Now) -> [(&'static str, Timestamp); 3] {
        // Today's local midnight as wall-clock seconds: everything below is written on the
        // user's clock and converted back to an instant at the end.
        let midnight = now.parts(now.at()).0 * DAY;
        let tonight_at = midnight + 18 * HOUR;
        let tonight = now.instant(if now.local_secs(now.at()) < tonight_at {
            tonight_at
        } else {
            tonight_at + DAY
        });
        let tomorrow = now.instant(midnight + DAY + 8 * HOUR);
        // Monday is weekday 1 (0 is Sunday).
        let monday =
            now.instant(midnight + now.days_until_weekday(now.at(), 1, 8 * HOUR) + 8 * HOUR);
        [
            ("Tonight", tonight),
            ("Tomorrow", tomorrow),
            ("Monday", monday),
        ]
    }
}
