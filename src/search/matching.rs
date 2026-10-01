//! Evaluating a [`Query`] against a message.

use super::{Combinator, Field, Group, Query, dates};
use crate::clock::Timestamp;
use crate::judge::Kind;
use crate::model::{Mailbox, Message, Tag, TriageState};

/// Lowercased haystacks for one message, computed once per `matches` call.
struct Haystack<'a> {
    m: &'a Message,
    mailbox: &'a Mailbox,
    state: TriageState,
    now: Timestamp,
    name: String,
    email: String,
    subject: String,
    body: Option<String>,
}

impl Haystack<'_> {
    fn date(&self) -> &str {
        self.m.received.get(..10).unwrap_or(&self.m.received)
    }

    fn value(&self, field: Field, v: &str) -> bool {
        let tags = self.mailbox.tags(self.m.id);
        let m = self.m;
        match field {
            Field::From => self.name.contains(v) || self.email.contains(v),
            Field::To => m.to.to_lowercase().contains(v),
            Field::Cc => m.cc.to_lowercase().contains(v),
            Field::Bcc => m.bcc.to_lowercase().contains(v),
            Field::Subject => self.subject.contains(v),
            Field::Body => self.body.as_deref().unwrap_or_default().contains(v),
            Field::Before => dates::resolve(v, self.now).is_some_and(|d| self.date() < d.as_str()),
            Field::After => dates::resolve(v, self.now).is_some_and(|d| self.date() >= d.as_str()),
            Field::On => dates::resolve(v, self.now).is_some_and(|d| self.date() == d),
            Field::Is => match v {
                "inbox" => self.state == TriageState::Inbox,
                "snoozed" => self.state == TriageState::Snoozed,
                "archived" => self.state == TriageState::Archived,
                "deleted" => self.state == TriageState::Deleted,
                "filed" => matches!(self.state, TriageState::Filed(_)),
                "sent" => m.outgoing && self.state != TriageState::Deleted,
                "new" => self.mailbox.is_new_sender(m.id),
                _ => false,
            },
            Field::In => match v {
                "inbox" => !m.outgoing && self.state == TriageState::Inbox,
                "snoozed" => self.state == TriageState::Snoozed,
                "sent" => m.outgoing && self.state != TriageState::Deleted,
                "archived" => self.state == TriageState::Archived,
                "deleted" => self.state == TriageState::Deleted,
                folder => self.mailbox.folders(&m.account).iter().any(|f| {
                    self.state == TriageState::Filed(f.id) && f.name.to_lowercase() == folder
                }),
            },
            Field::Tag => match v {
                "needs-reply" => tags.contains(&Tag::NeedsReply),
                "awaiting" => tags.contains(&Tag::AwaitingReply),
                "follow-up" => tags.contains(&Tag::FollowUp),
                "reminder" => tags.contains(&Tag::Reminder),
                "spam" => tags.contains(&Tag::PossibleSpam),
                "urgent" => tags.iter().any(|t| matches!(t, Tag::Urgent(_))),
                _ => false,
            },
            Field::Kind => Kind::ALL
                .iter()
                .find(|k| k.label() == v)
                .is_some_and(|k| tags.contains(&Tag::Kind(*k))),
            Field::Account => m.account.to_lowercase() == v,
        }
    }

    fn group(&self, g: &Group) -> bool {
        let hit = |v: &String| self.value(g.field, v);
        match g.combinator {
            Combinator::And => g.values.iter().all(hit),
            Combinator::Or => g.values.iter().any(hit),
        }
    }
}

impl Query {
    /// Whether `m` satisfies every group and every free-text term. Relative dates resolve
    /// against `now`; triage state, tags, folders and new-sender status come from `mailbox`.
    pub fn matches(&self, m: &Message, mailbox: &Mailbox, now: Timestamp) -> bool {
        let Some(state) = mailbox.state_of(m.id) else {
            return false;
        };
        let needs_body = !self.text.is_empty() || self.groups.iter().any(|g| g.field == Field::Body);
        let h = Haystack {
            m,
            mailbox,
            state,
            now,
            name: m.from_name.to_lowercase(),
            email: m.from_email.to_lowercase(),
            subject: m.subject.to_lowercase(),
            body: needs_body.then(|| crate::reading::reader_text(m).to_lowercase()),
        };
        self.groups.iter().all(|g| h.group(g))
            && self.text.iter().all(|t| {
                h.subject.contains(t)
                    || h.body.as_deref().unwrap_or_default().contains(t)
                    || h.name.contains(t)
                    || h.email.contains(t)
            })
    }
}
