use super::*;

impl Mailbox {
    pub fn tags(&self, id: MessageId) -> &[Tag] {
        self.meta.get(&id).map_or(&[], |m| &m.tags)
    }

    pub fn awaiting_since(&self, id: MessageId) -> Option<Timestamp> {
        self.meta.get(&id).and_then(|m| m.awaiting_since)
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
                } else if now >= since + self.follow_up_timeout {
                    follow.push(m.id);
                }
            }
        }
        for id in answered {
            let meta = self.meta.entry(id).or_default();
            meta.awaiting_since = None;
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
        let (due, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut self.outbox)
            .into_iter()
            .partition(|o| o.due <= now);
        self.outbox = keep;
        report.flushed = due.len();
        for outgoing in due {
            let Some(original) = self.get(outgoing.reply.in_reply_to).cloned() else {
                continue;
            };
            let id = self
                .messages
                .iter()
                .map(|m| m.id)
                .max()
                .unwrap_or(0)
                .saturating_add(1);
            let sent = Message {
                id,
                thread_id: original.thread_id,
                from_name: "You".into(),
                from_email: self
                    .account(&original.account)
                    .map_or_else(|| "you@example.com".into(), |a| a.email.clone()),
                to: original.from_email.clone(),
                subject: original.subject.clone(),
                body: outgoing.reply.body.clone(),
                received: original.received.clone(),
                state: self.post_send.get(&outgoing.seq).copied().unwrap_or(original.state),
                account: original.account.clone(),
                outgoing: true,
                snooze: None,
                cc: String::new(),
                bcc: String::new(),
                html: None,
                attachments: Vec::new(),
                read: true,
                partial: false,
            };
            self.index.insert(id, self.messages.len());
            self.newest_first.insert(0, id);
            self.meta.insert(id, Meta::default());
            self.messages.push(sent);
            let seq = outgoing.seq;
            self.sent_ids.insert(seq, id);
            self.sent.push(outgoing.reply);
            if let Some(step) = self.undo.iter_mut().find(|step| {
                step.changes
                    .iter()
                    .any(|change| matches!(change, Change::Queued(queued) if *queued == seq))
            }) {
                step.changes.push(Change::Materialised(id));
                step.changes.push(Change::SentPush);
            }
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
