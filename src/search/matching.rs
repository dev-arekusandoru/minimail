//! Evaluating a [`Query`] against a message.

use std::cell::OnceCell;

use super::{Combinator, Field, Group, Query, dates};
use crate::judge::Kind;
use crate::model::{Mailbox, Message, Tag, TriageState};
use crate::tz::Now;

/// Lowercased haystacks for one message, each computed on first use: counting a location
/// for every sidebar row would otherwise lowercase every message's text on every frame.
struct Haystack<'a> {
    m: &'a Message,
    mailbox: &'a Mailbox,
    state: TriageState,
    now: &'a Now,
    name: OnceCell<String>,
    email: OnceCell<String>,
    subject: OnceCell<String>,
    body: OnceCell<String>,
}

impl Haystack<'_> {
    fn name(&self) -> &str {
        self.name.get_or_init(|| self.m.from_name.to_lowercase())
    }

    fn email(&self) -> &str {
        self.email.get_or_init(|| self.m.from_email.to_lowercase())
    }

    fn subject(&self) -> &str {
        self.subject.get_or_init(|| self.m.subject.to_lowercase())
    }

    fn body(&self) -> &str {
        self.body.get_or_init(|| crate::reading::reader_text(self.m).to_lowercase())
    }

    /// The message's calendar day on the user's clock, `yyyy-mm-dd`. An unreadable
    /// `received` falls back to the leading ten characters, as written.
    fn date(&self) -> String {
        match self.m.received_at() {
            Some(ts) => self.now.day(ts),
            None => self.m.received.get(..10).unwrap_or(&self.m.received).to_owned(),
        }
    }

    fn value(&self, field: Field, v: &str) -> bool {
        let tags = self.mailbox.tags(self.m.id);
        let m = self.m;
        match field {
            Field::From => self.name().contains(v) || self.email().contains(v),
            Field::To => m.to.to_lowercase().contains(v),
            Field::Cc => m.cc.to_lowercase().contains(v),
            Field::Bcc => m.bcc.to_lowercase().contains(v),
            Field::Subject => self.subject().contains(v),
            Field::Body => self.body().contains(v),
            Field::Before => dates::resolve(v, self.now).is_some_and(|d| self.date() < d),
            Field::After => dates::resolve(v, self.now).is_some_and(|d| self.date() >= d),
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
                folder => match self.state {
                    TriageState::Filed(id) => self
                        .mailbox
                        .folder(id)
                        .is_some_and(|f| f.name.to_lowercase() == folder || self.mailbox.folder_path(id).to_lowercase() == folder),
                    _ => false,
                },
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
    /// against `now`, and a message's day is its day on that same clock; triage state, tags,
    /// folders and new-sender status come from `mailbox`.
    pub fn matches(&self, m: &Message, mailbox: &Mailbox, now: &Now) -> bool {
        let Some(state) = mailbox.state_of(m.id) else {
            return false;
        };
        let h = Haystack {
            m,
            mailbox,
            state,
            now,
            name: OnceCell::new(),
            email: OnceCell::new(),
            subject: OnceCell::new(),
            body: OnceCell::new(),
        };
        self.groups.iter().all(|g| h.group(g))
            && self.text.iter().all(|t| {
                h.subject().contains(t) || h.name().contains(t) || h.email().contains(t) || h.body().contains(t)
            })
    }
}
