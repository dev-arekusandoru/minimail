//! Pure triage logic: messages, the exactly-one-state invariant, undo, and a
//! cursor/selection model over a single view.
//!
//! No UI types live here, so all of it is headlessly testable.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use serde::{Deserialize, Serialize};

use crate::clock::{DAY, HOUR, MINUTE, Timestamp};
use crate::contacts::{ContactSource, ContactStore};
use crate::judge::{AnswerValue, Kind, QuestionKey, Suggestion};

mod replies;
mod states;
mod suggestions;
mod timing;
mod triage;
mod undo;
mod visibility;

pub use triage::Triage;

pub type MessageId = u32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum TriageState {
    #[default]
    Inbox,
    Waiting,
    Later,
    Done,
}

impl TriageState {
    pub const ALL: [TriageState; 4] = [
        TriageState::Inbox,
        TriageState::Waiting,
        TriageState::Later,
        TriageState::Done,
    ];

    pub fn label(self) -> &'static str {
        match self {
            TriageState::Inbox => "Inbox",
            TriageState::Waiting => "Waiting",
            TriageState::Later => "Later",
            TriageState::Done => "Done",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Message {
    pub id: MessageId,
    pub thread_id: u32,
    pub from_name: String,
    pub from_email: String,
    pub to: String,
    pub subject: String,
    pub body: String,
    /// RFC3339 timestamp, e.g. `2026-09-14T08:12:00Z`.
    pub received: String,
    #[serde(default)]
    pub state: TriageState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    pub in_reply_to: MessageId,
    pub body: String,
}

/// A reply held back for `OUTBOX_DELAY` seconds so it can still be recalled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outgoing {
    pub reply: Reply,
    pub due: Timestamp,
    seq: u64,
}

/// Seconds a reply waits in the outbox before `tick` sends it.
pub const OUTBOX_DELAY: Timestamp = 10;
/// Waiting mail with no newer reply resurfaces after this long.
pub const WAITING_TIMEOUT: Timestamp = 3 * DAY;

/// Badge attached to a message (accepted AI labels and resurfacing notes).
#[derive(Clone, Debug, PartialEq)]
pub enum Tag {
    NoReply,
    NeedsReply,
    Spam,
    Urgent(u8),
    Kind(Kind),
}

/// What `tick` changed. All-empty on an idempotent repeat.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TickReport {
    /// Waiting messages returned to the Inbox as `Tag::NoReply`.
    pub resurfaced: Vec<MessageId>,
    /// Snoozed messages woken into the Inbox.
    pub woken: Vec<MessageId>,
    /// Replies moved from the outbox to `sent()`.
    pub flushed: usize,
}

/// Per-message metadata beside the triage state.
#[derive(Clone, Debug, Default, PartialEq)]
struct Meta {
    waiting_since: Option<Timestamp>,
    snoozed_until: Option<Timestamp>,
    tags: Vec<Tag>,
}

/// One inverse operation; a user action's `UndoStep` is a list of these.
#[derive(Clone, Debug)]
enum Change {
    Msg(MessageId, TriageState, Meta),
    Muted(u32, bool),
    /// Sender allowed in the Screener: the email, whether it was known before,
    /// and whether the address book created a contact for it.
    Known(String, bool, bool),
    Blocked(String, bool),
    /// Sender was appended to `unsubscribed`; undo removes it.
    Unsubscribed(String),
    /// Whole pending-suggestion list before the action.
    Pending(Vec<Suggestion>),
    /// Outbox reply with this seq was queued; undo removes it if still there.
    Queued(u64),
    /// A reply was pushed straight to `sent`.
    SentPush,
}

#[derive(Clone, Debug)]
struct UndoStep {
    changes: Vec<Change>,
}

pub struct Mailbox {
    messages: Vec<Message>,
    /// Message id -> index into `messages`, for O(1) lookup.
    index: HashMap<MessageId, usize>,
    /// Every id ordered newest-first by `received`; `ids_in` filters this.
    newest_first: Vec<MessageId>,
    undo: Vec<UndoStep>,
    sent: Vec<Reply>,
    outbox: Vec<Outgoing>,
    next_seq: u64,
    meta: HashMap<MessageId, Meta>,
    /// Lowercased emails that skip the Screener.
    known: HashSet<String>,
    /// The address book; `None` for a mailbox built without one.
    contacts: Option<Rc<ContactStore>>,
    blocked: HashSet<String>,
    unsubscribed: Vec<String>,
    muted: HashSet<u32>,
    pending: Vec<Suggestion>,
}

impl Mailbox {
    /// Load messages; every sender counts as known (nothing is screened).
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        let mut mb = Self::build(serde_json::from_str(json)?, HashSet::new());
        mb.known = mb.messages.iter().map(|m| lower(&m.from_email)).collect();
        Ok(mb)
    }

    /// Load messages plus the addresses of `store`; every other sender lands
    /// in the Screener. Later `allow_sender` calls are written back to the
    /// store, so they survive a restart.
    pub fn from_json_with_contacts(
        json: &str,
        store: Rc<ContactStore>,
    ) -> Result<Self, serde_json::Error> {
        let known = store.known_addresses().unwrap_or_default();
        let mut mb = Self::build(serde_json::from_str(json)?, known);
        mb.contacts = Some(store);
        Ok(mb)
    }

    fn build(messages: Vec<Message>, known: HashSet<String>) -> Self {
        let mut index = HashMap::with_capacity(messages.len());
        for (i, m) in messages.iter().enumerate() {
            index.entry(m.id).or_insert(i);
        }
        let mut newest_first: Vec<MessageId> = messages.iter().map(|m| m.id).collect();
        newest_first.sort_by(|a, b| {
            let stamp = |id: &MessageId| {
                index
                    .get(id)
                    .and_then(|i| parse_rfc3339(&messages[*i].received))
            };
            stamp(b).cmp(&stamp(a)).then_with(|| b.cmp(a))
        });
        Self {
            messages,
            index,
            newest_first,
            undo: Vec::new(),
            sent: Vec::new(),
            outbox: Vec::new(),
            next_seq: 0,
            meta: HashMap::new(),
            known,
            blocked: HashSet::new(),
            contacts: None,
            unsubscribed: Vec::new(),
            muted: HashSet::new(),
            pending: Vec::new(),
        }
    }

    /// The mock mailbox against an in-memory copy of the shipped address
    /// book. The app passes its own database via [`Self::load_default_with`].
    pub fn load_default() -> Self {
        Self::load_default_with(Rc::new(
            crate::contacts::open_seeded_in_memory().expect("contacts database"),
        ))
    }

    /// [`Self::load_default`] against an explicit address book.
    pub fn load_default_with(store: Rc<ContactStore>) -> Self {
        Self::from_json_with_contacts(include_str!("../../fixtures/mailbox.json"), store)
            .expect("fixtures parse")
    }

    /// The address book behind the Screener, if this mailbox has one.
    pub fn contacts(&self) -> Option<&ContactStore> {
        self.contacts.as_deref()
    }

    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    pub fn get(&self, id: MessageId) -> Option<&Message> {
        self.index.get(&id).and_then(|i| self.messages.get(*i))
    }

    pub fn state_of(&self, id: MessageId) -> Option<TriageState> {
        self.get(id).map(|m| m.state)
    }
}

/// Parse a custom snooze like "30m", "3h" or "2d" into `now + duration`.
/// `None` for a missing/zero amount, unknown unit, or overflow.
pub fn parse_snooze(input: &str, now: Timestamp) -> Option<Timestamp> {
    let s = input.trim().to_ascii_lowercase();
    let unit = match s.chars().last()? {
        'm' => MINUTE,
        'h' => HOUR,
        'd' => DAY,
        _ => return None,
    };
    let digits = &s[..s.len() - 1];
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: i64 = digits.parse().ok().filter(|n| *n > 0)?;
    now.checked_add(n.checked_mul(unit)?)
}

fn lower(s: &str) -> String {
    s.to_ascii_lowercase()
}

fn set_membership<T: std::hash::Hash + Eq>(set: &mut HashSet<T>, value: T, present: bool) {
    if present {
        set.insert(value);
    } else {
        set.remove(&value);
    }
}

/// Parse an RFC3339 timestamp into whole seconds since the Unix epoch.
/// Returns `None` for anything unreadable; such messages sort last.
fn parse_rfc3339(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let num = |range: std::ops::Range<usize>| -> Option<i64> {
        let part = s.get(range)?;
        if !part.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        part.parse::<i64>().ok()
    };
    if b[4] != b'-' || b[7] != b'-' || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let (year, month, day) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hour, minute, second) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }

    let mut rest = 19;
    if b.get(rest) == Some(&b'.') {
        rest += 1;
        while rest < b.len() && b[rest].is_ascii_digit() {
            rest += 1;
        }
    }
    let offset = match b.get(rest) {
        None | Some(b'Z') | Some(b'z') => 0,
        Some(sign @ (b'+' | b'-')) => {
            let sign = if *sign == b'-' { -1 } else { 1 };
            sign * (num(rest + 1..rest + 3)? * 3600 + num(rest + 4..rest + 6)? * 60)
        }
        _ => return None,
    };

    Some(days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second - offset)
}

/// Days since 1970-01-01 for a proleptic Gregorian date.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = y - i64::from(m <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_handles_offsets() {
        let utc = parse_rfc3339("2026-09-01T00:00:00Z").unwrap();
        assert_eq!(utc, parse_rfc3339("2026-09-01T02:00:00+02:00").unwrap());
        assert_eq!(utc, parse_rfc3339("2026-08-31T22:00:00-02:00").unwrap());
        assert!(parse_rfc3339("yesterday").is_none());
    }
}
