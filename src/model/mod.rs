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
mod remote;
mod triage;
mod undo;
mod visibility;

pub use triage::Triage;

pub type MessageId = u32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum TriageState {
    #[default]
    Inbox,
    Snoozed,
    Archived,
    Filed(FolderId),
    Deleted,
}

impl TriageState {
    pub fn label(self) -> &'static str {
        match self {
            TriageState::Inbox => "Inbox",
            TriageState::Snoozed => "Snoozed",
            TriageState::Archived => "Archived",
            TriageState::Filed(_) => "Filed",
            TriageState::Deleted => "Deleted",
        }
    }
}
impl TriageState {
    pub const ALL: [TriageState; 5] = [
        Self::Inbox,
        Self::Snoozed,
        Self::Archived,
        Self::Filed(0),
        Self::Deleted,
    ];
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
    pub received: String,
    #[serde(default)]
    pub state: TriageState,
    #[serde(default = "personal_account")]
    pub account: AccountId,
    #[serde(default)]
    pub outgoing: bool,
    #[serde(default)]
    pub snooze: Option<String>,
    /// Carbon-copy recipients, same comma-separated format as `to`.
    #[serde(default)]
    pub cc: String,
    #[serde(default)]
    pub bcc: String,
    /// HTML alternative. `body` is the text part (empty for HTML-only mail).
    #[serde(default)]
    pub html: Option<String>,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    /// Opened (here, or on the server for synced mail). Persisted for synced accounts.
    #[serde(default)]
    pub read: bool,
    /// Only headers and the server snippet are known (`body` holds the snippet);
    /// the full body is downloaded when the message is opened.
    #[serde(default)]
    pub partial: bool,
}

impl Message {
    /// Whole seconds since the Unix epoch for `received`, or `None` if unparseable.
    pub fn received_at(&self) -> Option<Timestamp> {
        parse_rfc3339(&self.received)
    }
}

/// A file attached to a message; `size` is in bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    pub name: String,
    pub size: u64,
}

fn personal_account() -> AccountId {
    "personal".to_owned()
}
pub type AccountId = String;
pub type FolderId = u32;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum ProviderKind {
    #[default]
    Mock,
    Gmail,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub id: AccountId,
    pub name: String,
    pub email: String,
    pub color: String,
    /// Lucide icon key (see `account_style::ICONS`); `None` in configs from before icons.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default)]
    pub provider: ProviderKind,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Folder {
    pub id: FolderId,
    pub account: AccountId,
    pub name: String,
    pub parent: Option<FolderId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub enum Location {
    #[default]
    AllInboxes,
    Inbox(AccountId),
    Snoozed(AccountId),
    Sent(AccountId),
    Archive(AccountId),
    Trash(AccountId),
    Folder(FolderId),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Chip {
    #[default]
    All,
    NeedsReply,
    FollowUp,
    Urgent,
    NewSenders,
    PossibleSpam,
}
impl Chip {
    pub const ALL: [Chip; 6] = [
        Self::All,
        Self::NeedsReply,
        Self::FollowUp,
        Self::Urgent,
        Self::NewSenders,
        Self::PossibleSpam,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::NeedsReply => "Needs Reply",
            Self::FollowUp => "Follow Up",
            Self::Urgent => "Urgent",
            Self::NewSenders => "New Senders",
            Self::PossibleSpam => "Possible Spam",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TagFilter {
    NeedsReply,
    AwaitingReply,
    FollowUp,
    Reminder,
    NewSender,
    PossibleSpam,
    Urgent,
}
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Filter {
    pub tags: Vec<TagFilter>,
    pub kind: Option<Kind>,
    pub account: Option<AccountId>,
}
#[derive(Clone, Debug, PartialEq, Default)]
pub struct View {
    pub location: Location,
    pub chip: Chip,
    pub filter: Filter,
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
pub const DEFAULT_FOLLOW_UP_TIMEOUT: Timestamp = 3 * DAY;

/// Badge attached to a message (accepted AI labels and resurfacing notes).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Tag {
    NeedsReply,
    AwaitingReply,
    FollowUp,
    Reminder,
    PossibleSpam,
    Urgent(u8),
    Kind(Kind),
}

/// What `tick` changed. All-empty on an idempotent repeat.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TickReport {
    pub woken: Vec<MessageId>,
    pub followed_up: Vec<MessageId>,
    pub flushed: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Meta {
    awaiting_since: Option<Timestamp>,
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
    /// Post-send filing recorded for a queued reply: the seq, its previous value,
    /// and the state a reply materialised after the filing should go back to.
    PostSend(u64, Option<TriageState>, TriageState),
    SentPush,
    FolderPush(FolderId),
    Materialised(MessageId),
}

#[derive(Clone, Debug)]
struct UndoStep {
    changes: Vec<Change>,
}

fn fixture_accounts() -> (Vec<Account>, Vec<Folder>) {
    let data: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../../fixtures/accounts.json")).expect("account fixture");
    let accounts = data
        .iter()
        .map(|v| serde_json::from_value(v.clone()).expect("account"))
        .collect();
    let folders = data
        .iter()
        .flat_map(|v| {
            let account = v["id"].as_str().unwrap_or_default().to_owned();
            v["folders"]
                .as_array()
                .into_iter()
                .flatten()
                .map(move |f| Folder {
                    id: f["id"].as_u64().unwrap_or_default() as FolderId,
                    account: account.clone(),
                    name: f["name"].as_str().unwrap_or_default().to_owned(),
                    parent: f["parent"].as_u64().map(|n| n as FolderId),
                })
                .collect::<Vec<_>>()
        })
        .collect();
    (accounts, folders)
}

pub struct Mailbox {
    messages: Vec<Message>,
    index: HashMap<MessageId, usize>,
    newest_first: Vec<MessageId>,
    undo: Vec<UndoStep>,
    sent: Vec<Reply>,
    outbox: Vec<Outgoing>,
    next_seq: u64,
    meta: HashMap<MessageId, Meta>,
    known: HashSet<String>,
    contacts: Option<Rc<ContactStore>>,
    blocked: HashSet<String>,
    unsubscribed: Vec<String>,
    muted: HashSet<u32>,
    pending: Vec<Suggestion>,
    accounts: Vec<Account>,
    folders: Vec<Folder>,
    follow_up_timeout: Timestamp,
    /// State a still-queued reply should materialise with (post-send filing), by outbox seq.
    post_send: HashMap<u64, TriageState>,
    /// Replies the outbox materialised, by the outbox entry they came from.
    sent_ids: HashMap<u64, MessageId>,
}

impl Mailbox {
    /// Load messages; without a contact store, senders are not assumed known.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        let (accounts, folders) = fixture_accounts();
        Ok(Self::build(
            serde_json::from_str(json)?,
            HashSet::new(),
            accounts,
            folders,
        ))
    }

    /// Load messages plus the known addresses in `store`.
    pub fn from_json_with_contacts(
        json: &str,
        store: Rc<ContactStore>,
    ) -> Result<Self, serde_json::Error> {
        let known = store.known_addresses().unwrap_or_default();
        let (accounts, folders) = fixture_accounts();
        let mut mb = Self::build(serde_json::from_str(json)?, known, accounts, folders);
        mb.contacts = Some(store);
        Ok(mb)
    }

    fn build(
        mut messages: Vec<Message>,
        known: HashSet<String>,
        accounts: Vec<Account>,
        folders: Vec<Folder>,
    ) -> Self {
        for message in &mut messages {
            if message.state == TriageState::Snoozed
                && message.snooze.as_deref().and_then(parse_rfc3339).is_none()
            {
                message.state = TriageState::Inbox;
            }
        }
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
        let meta = messages
            .iter()
            .filter_map(|m| {
                (m.state == TriageState::Snoozed)
                    .then(|| {
                        m.snooze.as_deref().and_then(parse_rfc3339).map(|until| {
                            (
                                m.id,
                                Meta {
                                    snoozed_until: Some(until),
                                    ..Meta::default()
                                },
                            )
                        })
                    })
                    .flatten()
            })
            .collect();
        Self {
            messages,
            index,
            newest_first,
            undo: Vec::new(),
            sent: Vec::new(),
            outbox: Vec::new(),
            next_seq: 0,
            meta,
            known,
            blocked: HashSet::new(),
            contacts: None,
            unsubscribed: Vec::new(),
            muted: HashSet::new(),
            pending: Vec::new(),
            accounts,
            folders,
            follow_up_timeout: DEFAULT_FOLLOW_UP_TIMEOUT,
            post_send: HashMap::new(),
            sent_ids: HashMap::new(),
        }
    }

    /// Build from pre-loaded parts (e.g. the sync cache) plus an address book.
    pub fn from_parts(
        messages: Vec<Message>,
        accounts: Vec<Account>,
        folders: Vec<Folder>,
        store: Rc<ContactStore>,
    ) -> Self {
        let known = store.known_addresses().unwrap_or_default();
        let mut mb = Self::build(messages, known, accounts, folders);
        mb.contacts = Some(store);
        mb
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

    /// Opened in this app, or read on the server for synced mail.
    pub fn is_read(&self, id: MessageId) -> bool {
        self.get(id).is_some_and(|m| m.read)
    }

    /// Mark `id` read. Not an undo step: reading is not a triage action.
    pub fn mark_read(&mut self, id: MessageId) {
        if let Some(&i) = self.index.get(&id) {
            self.messages[i].read = true;
        }
    }

    /// Store a downloaded body and clear `partial`. Not an undo step.
    pub fn set_body(&mut self, id: MessageId, body: String, html: Option<String>, attachments: Vec<Attachment>) {
        if let Some(&i) = self.index.get(&id) {
            let m = &mut self.messages[i];
            m.body = body;
            m.html = html;
            m.attachments = attachments;
            m.partial = false;
        }
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
pub fn parse_rfc3339(s: &str) -> Option<i64> {
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
/// Inverse of [`parse_rfc3339`]: `2026-09-29T07:41:00Z`.
pub fn format_rfc3339(ts: Timestamp) -> String {
    let days = ts.div_euclid(86_400);
    let secs = ts.rem_euclid(86_400);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_round_trips() {
        let leap = parse_rfc3339("2028-02-29T23:59:59Z").unwrap();
        for t in [0, 1_790_000_000, leap] {
            assert_eq!(parse_rfc3339(&format_rfc3339(t)), Some(t));
        }
        assert_eq!(format_rfc3339(leap), "2028-02-29T23:59:59Z");
    }

    #[test]
    fn rfc3339_handles_offsets() {
        let utc = parse_rfc3339("2026-09-01T00:00:00Z").unwrap();
        assert_eq!(utc, parse_rfc3339("2026-09-01T02:00:00+02:00").unwrap());
        assert_eq!(utc, parse_rfc3339("2026-08-31T22:00:00-02:00").unwrap());
        assert!(parse_rfc3339("yesterday").is_none());
    }
}
