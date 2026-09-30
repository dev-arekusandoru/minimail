//! Search query parsing and matching for the palette.
//!
//! Syntax (terms are ANDed, everything case-insensitive): `from:x`, `subject:x`,
//! `is:inbox|snoozed|archived|filed|deleted|sent|new`,
//! `tag:needs-reply|awaiting|follow-up|reminder|spam|urgent`, dates, quoted phrases,
//! and free text over subject + body + sender. A leading `/` is ignored.

use crate::model::{Mailbox, Message, Tag, TriageState};

const KEYS: [&str; 6] = ["from", "subject", "is", "tag", "before", "after"];

#[derive(Clone, Debug, PartialEq, Eq)]
enum Is {
    State(TriageState),
    Filed,
    Sent,
    New,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TagQuery {
    NeedsReply,
    Awaiting,
    FollowUp,
    Reminder,
    Spam,
    Urgent,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    from: Vec<String>,
    subject: Vec<String>,
    is: Vec<Is>,
    tags: Vec<TagQuery>,
    before: Vec<String>,
    after: Vec<String>,
    text: Vec<String>,
}

fn tokenize(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut any = false;
    for c in input.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any {
                    tokens.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            c => {
                cur.push(c);
                any = true;
            }
        }
    }
    if any {
        tokens.push(cur);
    }
    tokens
}

fn valid_date(s: &str) -> bool {
    if s.len() != 10 || s.as_bytes()[4] != b'-' || s.as_bytes()[7] != b'-' {
        return false;
    }
    let (Ok(year), Ok(month), Ok(day)) = (
        s[..4].parse::<u32>(),
        s[5..7].parse::<u32>(),
        s[8..10].parse::<u32>(),
    ) else {
        return false;
    };
    year > 0 && (1..=12).contains(&month) && (1..=31).contains(&day)
}

fn is_key(token: &str) -> Option<(&'static str, &str)> {
    let (k, v) = token.split_once(':')?;
    let k = KEYS.iter().find(|key| key.eq_ignore_ascii_case(k))?;
    Some((k, v))
}

impl Query {
    pub fn parse(input: &str) -> Query {
        let input = input.trim_start();
        let input = input.strip_prefix('/').unwrap_or(input);
        let mut q = Query::default();
        for token in tokenize(input) {
            let lower = token.to_lowercase();
            let Some((key, value)) = is_key(&lower).filter(|(_, v)| !v.is_empty()) else {
                q.text.push(lower);
                continue;
            };
            let value = value.to_string();
            match key {
                "from" => q.from.push(value),
                "subject" => q.subject.push(value),
                "is" => match value.as_str() {
                    "inbox" => q.is.push(Is::State(TriageState::Inbox)),
                    "snoozed" => q.is.push(Is::State(TriageState::Snoozed)),
                    "archived" => q.is.push(Is::State(TriageState::Archived)),
                    "filed" => q.is.push(Is::Filed),
                    "deleted" => q.is.push(Is::State(TriageState::Deleted)),
                    "sent" => q.is.push(Is::Sent),
                    "new" => q.is.push(Is::New),
                    _ => q.text.push(lower),
                },
                "tag" => match value.as_str() {
                    "needs-reply" => q.tags.push(TagQuery::NeedsReply),
                    "awaiting" => q.tags.push(TagQuery::Awaiting),
                    "follow-up" => q.tags.push(TagQuery::FollowUp),
                    "reminder" => q.tags.push(TagQuery::Reminder),
                    "spam" => q.tags.push(TagQuery::Spam),
                    "urgent" => q.tags.push(TagQuery::Urgent),
                    _ => q.text.push(lower),
                },
                "before" if valid_date(&value) => q.before.push(value),
                "after" if valid_date(&value) => q.after.push(value),
                _ => q.text.push(lower),
            }
        }
        q
    }

    pub fn is_search(input: &str) -> bool {
        let input = input.trim_start();
        input.starts_with('/')
            || tokenize(input)
                .iter()
                .any(|t| is_key(t).is_some_and(|(_, v)| !v.is_empty()))
    }

    /// Match using mailbox state, tags, new-sender status, and outgoing status.
    pub fn matches(&self, m: &Message, mailbox: &Mailbox) -> bool {
        let Some(state) = mailbox.state_of(m.id) else {
            return false;
        };
        let new_sender = mailbox.is_new_sender(m.id);
        let tags = mailbox.tags(m.id);
        let name = m.from_name.to_lowercase();
        let email = m.from_email.to_lowercase();
        let subject = m.subject.to_lowercase();
        let body = m.body.to_lowercase();
        let date = m.received.get(..10).unwrap_or(&m.received);
        self.from
            .iter()
            .all(|f| name.contains(f) || email.contains(f))
            && self.subject.iter().all(|s| subject.contains(s))
            && self.is.iter().all(|i| match i {
                Is::State(s) => *s == state,
                Is::Filed => matches!(state, TriageState::Filed(_)),
                Is::Sent => m.outgoing && state != TriageState::Deleted,
                Is::New => new_sender,
            })
            && self.tags.iter().all(|t| match t {
                TagQuery::NeedsReply => tags.contains(&Tag::NeedsReply),
                TagQuery::Awaiting => tags.contains(&Tag::AwaitingReply),
                TagQuery::FollowUp => tags.contains(&Tag::FollowUp),
                TagQuery::Reminder => tags.contains(&Tag::Reminder),
                TagQuery::Spam => tags.contains(&Tag::PossibleSpam),
                TagQuery::Urgent => tags.iter().any(|x| matches!(x, Tag::Urgent(_))),
            })
            && self.before.iter().all(|d| date < d.as_str())
            && self.after.iter().all(|d| date >= d.as_str())
            && self.text.iter().all(|t| {
                subject.contains(t) || body.contains(t) || name.contains(t) || email.contains(t)
            })
    }

    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        parts.extend(self.from.iter().map(|v| format!("from:{v}")));
        parts.extend(self.subject.iter().map(|v| format!("subject:{v}")));
        parts.extend(self.is.iter().map(|i| match i {
            Is::State(s) => format!("is:{}", s.label().to_lowercase()),
            Is::Filed => "is:filed".into(),
            Is::Sent => "is:sent".into(),
            Is::New => "is:new".into(),
        }));
        parts.extend(self.tags.iter().map(|t| {
            format!(
                "tag:{}",
                match t {
                    TagQuery::NeedsReply => "needs-reply",
                    TagQuery::Awaiting => "awaiting",
                    TagQuery::FollowUp => "follow-up",
                    TagQuery::Reminder => "reminder",
                    TagQuery::Spam => "spam",
                    TagQuery::Urgent => "urgent",
                }
            )
        }));
        parts.extend(self.after.iter().map(|v| format!("after:{v}")));
        parts.extend(self.before.iter().map(|v| format!("before:{v}")));
        parts.extend(self.text.iter().map(|v| format!("\"{v}\"")));
        parts.join(" ")
    }
}
