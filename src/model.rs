//! Pure triage logic: messages, the exactly-one-state invariant, undo, and a
//! cursor/selection model over a single view.
//!
//! No UI types live here, so all of it is headlessly testable.

use std::collections::{HashMap, HashSet};

use crate::clock::{DAY, HOUR, MINUTE, Timestamp};
use crate::judge::{AnswerValue, Kind, QuestionKey, Suggestion};

use serde::{Deserialize, Serialize};

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
    Known(String, bool),
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

    /// Load messages plus a JSON array of known sender emails; other senders
    /// land in the Screener.
    pub fn from_json_with_contacts(json: &str, contacts: &str) -> Result<Self, serde_json::Error> {
        let contacts: Vec<String> = serde_json::from_str(contacts)?;
        let known = contacts.iter().map(|c| lower(c)).collect();
        Ok(Self::build(serde_json::from_str(json)?, known))
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
            unsubscribed: Vec::new(),
            muted: HashSet::new(),
            pending: Vec::new(),
        }
    }

    pub fn load_default() -> Self {
        Self::from_json_with_contacts(
            include_str!("../fixtures/mailbox.json"),
            include_str!("../fixtures/contacts.json"),
        )
        .expect("fixtures parse")
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

    // ---------------------------------------------------------- visibility

    /// Muted thread, blocked sender or unsubscribed sender.
    fn is_hidden_msg(&self, m: &Message) -> bool {
        let email = lower(&m.from_email);
        self.muted.contains(&m.thread_id)
            || self.blocked.contains(&email)
            || self.unsubscribed.iter().any(|u| lower(u) == email)
    }

    fn is_screened_msg(&self, m: &Message) -> bool {
        !self.is_hidden_msg(m) && !self.known.contains(&lower(&m.from_email))
    }

    fn is_visible_msg(&self, m: &Message) -> bool {
        !self.is_hidden_msg(m) && self.known.contains(&lower(&m.from_email))
    }

    /// Muted / blocked / unsubscribed (not merely unscreened).
    pub fn is_hidden(&self, id: MessageId) -> bool {
        self.get(id).is_some_and(|m| self.is_hidden_msg(m))
    }

    /// In the Screener: unscreened sender, not otherwise hidden.
    pub fn is_screened(&self, id: MessageId) -> bool {
        self.get(id).is_some_and(|m| self.is_screened_msg(m))
    }

    /// Visible messages in `state`, newest first.
    pub fn ids_in(&self, state: TriageState) -> Vec<MessageId> {
        self.newest_first
            .iter()
            .copied()
            .filter(|id| {
                self.get(*id)
                    .is_some_and(|m| m.state == state && self.is_visible_msg(m))
            })
            .collect()
    }

    /// Visible messages in `state`.
    pub fn count(&self, state: TriageState) -> usize {
        self.messages
            .iter()
            .filter(|m| m.state == state && self.is_visible_msg(m))
            .count()
    }

    /// Messages from unscreened senders, newest first.
    pub fn screener_ids(&self) -> Vec<MessageId> {
        self.newest_first
            .iter()
            .copied()
            .filter(|id| self.is_screened(*id))
            .collect()
    }

    pub fn hidden_count(&self) -> usize {
        self.messages
            .iter()
            .filter(|m| self.is_hidden_msg(m))
            .count()
    }

    /// Let `email` through the Screener. False if already known.
    pub fn allow_sender(&mut self, email: &str) -> bool {
        let e = lower(email);
        if !self.known.insert(e.clone()) {
            return false;
        }
        self.push_undo(vec![Change::Known(e, false)]);
        true
    }

    /// Hide every message from `email` (not deleted). False if already blocked.
    pub fn block_sender(&mut self, email: &str) -> bool {
        let e = lower(email);
        if !self.blocked.insert(e.clone()) {
            return false;
        }
        self.push_undo(vec![Change::Blocked(e, false)]);
        true
    }

    pub fn mute_thread(&mut self, thread_id: u32) -> bool {
        if !self.muted.insert(thread_id) {
            return false;
        }
        self.push_undo(vec![Change::Muted(thread_id, false)]);
        true
    }

    pub fn is_muted(&self, thread_id: u32) -> bool {
        self.muted.contains(&thread_id)
    }

    /// Simulated unsubscribe: hide the sender and remember it.
    pub fn unsubscribe(&mut self, email: &str) -> bool {
        let e = lower(email);
        if self.unsubscribed.iter().any(|u| lower(u) == e) {
            return false;
        }
        self.unsubscribed.push(email.to_string());
        self.push_undo(vec![Change::Unsubscribed(email.to_string())]);
        true
    }

    pub fn unsubscribed(&self) -> &[String] {
        &self.unsubscribed
    }

    // -------------------------------------------------------------- states

    /// Set `state` on every id in `ids`. One undo entry per call; unknown ids
    /// and ids already in `state` are skipped. Returns the number changed.
    /// Records no time metadata; see `set_state_at`.
    pub fn set_state(&mut self, ids: &[MessageId], state: TriageState) -> usize {
        self.set_state_inner(ids, state, None)
    }

    /// Like `set_state`, but stamps `waiting_since = now` on messages entering
    /// Waiting so `tick` can resurface them.
    pub fn set_state_at(&mut self, ids: &[MessageId], state: TriageState, now: Timestamp) -> usize {
        self.set_state_inner(ids, state, Some(now))
    }

    fn set_state_inner(
        &mut self,
        ids: &[MessageId],
        state: TriageState,
        now: Option<Timestamp>,
    ) -> usize {
        let mut changes = Vec::new();
        for id in ids {
            let Some(&i) = self.index.get(id) else {
                continue;
            };
            if self.messages[i].state == state {
                continue;
            }
            changes.push(self.snapshot(*id));
            self.messages[i].state = state;
            let meta = self.meta.entry(*id).or_default();
            meta.waiting_since = if state == TriageState::Waiting { now } else { None };
            meta.snoozed_until = None;
            meta.tags.retain(|t| *t != Tag::NoReply);
        }
        let changed = changes.len();
        self.push_undo(changes);
        changed
    }

    /// Set `state` on every message from `from_email`, whatever state it is in
    /// now. One undo entry per call; returns the number changed.
    pub fn set_state_for_sender(&mut self, from_email: &str, state: TriageState) -> usize {
        let ids: Vec<MessageId> = self
            .messages
            .iter()
            .filter(|m| m.from_email.eq_ignore_ascii_case(from_email))
            .map(|m| m.id)
            .collect();
        self.set_state(&ids, state)
    }

    // ------------------------------------------------------ time metadata

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
    fn has_newer_reply(&self, waiting: &Message, since: Timestamp) -> bool {
        self.messages.iter().any(|m| {
            m.id != waiting.id
                && m.thread_id == waiting.thread_id
                && parse_rfc3339(&m.received).is_some_and(|t| t > since)
        })
    }

    /// State change from `tick`: clears time metadata, no undo.
    fn set_raw(&mut self, id: MessageId, state: TriageState) {
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

    // ------------------------------------------------------------ replies

    /// Record a reply to `to` and move the original to `Waiting`, sent
    /// immediately. Exactly one undo entry, which also retracts the sent
    /// reply. A no-op (no entry, no sent reply) if `to` is unknown.
    pub fn send_reply(&mut self, to: MessageId, body: String) {
        let Some(mut changes) = self.enter_waiting(to, None) else {
            return;
        };
        self.sent.push(Reply {
            in_reply_to: to,
            body,
        });
        changes.push(Change::SentPush);
        self.push_undo(changes);
    }

    /// Queue a reply in the outbox (due in `OUTBOX_DELAY`s) and move the
    /// original to Waiting now. One undo entry.
    pub fn send_reply_at(&mut self, to: MessageId, body: String, now: Timestamp) {
        let Some(mut changes) = self.enter_waiting(to, Some(now)) else {
            return;
        };
        let seq = self.next_seq;
        self.next_seq += 1;
        self.outbox.push(Outgoing {
            reply: Reply {
                in_reply_to: to,
                body,
            },
            due: now + OUTBOX_DELAY,
            seq,
        });
        changes.push(Change::Queued(seq));
        self.push_undo(changes);
    }

    /// Move `to` to Waiting; returns the undo changes, `None` if unknown id.
    fn enter_waiting(&mut self, to: MessageId, now: Option<Timestamp>) -> Option<Vec<Change>> {
        let &i = self.index.get(&to)?;
        let changes = vec![self.snapshot(to)];
        self.messages[i].state = TriageState::Waiting;
        let meta = self.meta.entry(to).or_default();
        meta.waiting_since = now;
        meta.snoozed_until = None;
        meta.tags.retain(|t| *t != Tag::NoReply);
        Some(changes)
    }

    pub fn sent(&self) -> &[Reply] {
        &self.sent
    }

    pub fn outbox(&self) -> &[Outgoing] {
        &self.outbox
    }

    /// Pull the newest reply back out of the outbox, undoing the send that
    /// queued it (original's prior state restored). `None` if the outbox is
    /// empty.
    pub fn recall_last(&mut self, _now: Timestamp) -> Option<Reply> {
        let seq = self.outbox.last()?.seq;
        let pos = self
            .undo
            .iter()
            .rposition(|s| s.changes.iter().any(|c| matches!(c, Change::Queued(q) if *q == seq)))?;
        let step = self.undo.remove(pos);
        let reply = self.outbox.last().map(|o| o.reply.clone());
        self.revert(step);
        reply
    }

    // -------------------------------------------------------- suggestions

    /// Store suggestions awaiting review; a newer one replaces an older
    /// pending suggestion for the same message and question.
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

    /// Apply every pending suggestion for `id`. One undo entry; returns the
    /// number accepted.
    pub fn accept_suggestions(&mut self, id: MessageId) -> usize {
        let (mine, rest): (Vec<_>, Vec<_>) =
            self.pending.iter().cloned().partition(|s| s.message == id);
        if mine.is_empty() {
            return 0;
        }
        let mut changes = vec![Change::Pending(self.pending.clone()), self.snapshot(id)];
        self.pending = rest;
        for s in &mine {
            self.apply(s);
        }
        changes.reverse();
        self.push_undo(changes);
        mine.len()
    }

    /// Drop every pending suggestion for `id`. One undo entry; returns the
    /// number dropped.
    pub fn reject_suggestions(&mut self, id: MessageId) -> usize {
        let before = self.pending.len();
        let snapshot = self.pending.clone();
        self.pending.retain(|s| s.message != id);
        let dropped = before - self.pending.len();
        if dropped > 0 {
            self.push_undo(vec![Change::Pending(snapshot)]);
        }
        dropped
    }

    /// Apply one suggestion immediately (System 1 auto mode). One undo entry.
    /// False if it was a no-op or the message is unknown.
    pub fn apply_auto(&mut self, s: Suggestion) -> bool {
        if !self.index.contains_key(&s.message) {
            return false;
        }
        let before = (
            self.snapshot(s.message),
            Change::Pending(self.pending.clone()),
        );
        self.pending
            .retain(|p| !(p.message == s.message && p.key == s.key));
        self.apply(&s);
        let changed = match (&before.0, self.snapshot(s.message)) {
            (Change::Msg(_, st, meta), Change::Msg(_, st2, meta2)) => {
                *st != st2 || *meta != meta2
            }
            _ => false,
        };
        if changed {
            self.push_undo(vec![before.1, before.0]);
        } else {
            // Nothing applied; still restore the pending list on failure.
            if let Change::Pending(p) = before.1 {
                self.pending = p;
            }
        }
        changed
    }

    /// Apply a suggestion's answer to its message (no undo bookkeeping).
    fn apply(&mut self, s: &Suggestion) {
        let id = s.message;
        let Some(&i) = self.index.get(&id) else {
            return;
        };
        match (s.key, &s.answer.value) {
            (QuestionKey::SuggestedState, AnswerValue::Choice(n)) => {
                if let Some(state) = TriageState::ALL.get(*n).copied() {
                    self.apply_state(i, state);
                }
            }
            (QuestionKey::Spam, AnswerValue::Bool(true)) => {
                self.add_tag(id, Tag::Spam);
                self.apply_state(i, TriageState::Done);
            }
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

    fn apply_state(&mut self, i: usize, state: TriageState) {
        if self.messages[i].state != state {
            self.messages[i].state = state;
            let meta = self.meta.entry(self.messages[i].id).or_default();
            meta.waiting_since = None;
            meta.snoozed_until = None;
        }
    }

    fn add_tag(&mut self, id: MessageId, tag: Tag) {
        let tags = &mut self.meta.entry(id).or_default().tags;
        if !tags.contains(&tag) {
            tags.push(tag);
        }
    }

    // --------------------------------------------------------------- undo

    fn snapshot(&self, id: MessageId) -> Change {
        Change::Msg(
            id,
            self.state_of(id).unwrap_or_default(),
            self.meta.get(&id).cloned().unwrap_or_default(),
        )
    }

    /// Push one undo step; empty change lists are ignored.
    fn push_undo(&mut self, changes: Vec<Change>) {
        if !changes.is_empty() {
            self.undo.push(UndoStep { changes });
        }
    }

    /// Undo the last mutation. Returns false when there is nothing to undo.
    pub fn undo(&mut self) -> bool {
        let Some(step) = self.undo.pop() else {
            return false;
        };
        self.revert(step);
        true
    }

    /// Apply a step's inverse changes, newest first.
    fn revert(&mut self, step: UndoStep) {
        for change in step.changes.into_iter().rev() {
            match change {
                Change::Msg(id, state, meta) => {
                    if let Some(&i) = self.index.get(&id) {
                        self.messages[i].state = state;
                    }
                    if meta == Meta::default() {
                        self.meta.remove(&id);
                    } else {
                        self.meta.insert(id, meta);
                    }
                }
                Change::Muted(t, was) => set_membership(&mut self.muted, t, was),
                Change::Known(e, was) => set_membership(&mut self.known, e, was),
                Change::Blocked(e, was) => set_membership(&mut self.blocked, e, was),
                Change::Unsubscribed(e) => self.unsubscribed.retain(|u| *u != e),
                Change::Pending(p) => self.pending = p,
                Change::Queued(seq) => self.outbox.retain(|o| o.seq != seq),
                Change::SentPush => {
                    self.sent.pop();
                }
            }
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

/// Cursor + multi-selection over one view.
pub struct Triage {
    pub view: TriageState,
    cursor: usize,
    selected: Vec<MessageId>,
    /// Selection origin, set to the cursor on the first `extend` after a
    /// selection reset.
    anchor: Option<usize>,
}

impl Triage {
    pub fn new(view: TriageState) -> Self {
        Self {
            view,
            cursor: 0,
            selected: Vec::new(),
            anchor: None,
        }
    }

    /// Raw cursor position, unclamped.
    pub fn cursor_index(&self) -> usize {
        self.cursor
    }

    /// Message under the cursor, clamped to the current view.
    pub fn cursor(&self, mb: &Mailbox) -> Option<MessageId> {
        let ids = mb.ids_in(self.view);
        ids.get(clamp_index(self.cursor, ids.len())).copied()
    }

    /// Move the cursor by `delta`, clamped to the view. Selection untouched.
    pub fn move_cursor(&mut self, mb: &Mailbox, delta: isize) {
        let len = mb.ids_in(self.view).len();
        self.cursor = shift(self.cursor, delta, len);
    }

    /// Range-select from the anchor (the cursor at the first extend) to the
    /// moved cursor, inclusive.
    pub fn extend(&mut self, mb: &Mailbox, delta: isize) {
        let ids = mb.ids_in(self.view);
        if ids.is_empty() {
            self.cursor = 0;
            self.clear_selection();
            return;
        }
        let anchor = *self.anchor.get_or_insert(clamp_index(self.cursor, ids.len()));
        self.cursor = shift(self.cursor, delta, ids.len());
        let (lo, hi) = if anchor <= self.cursor {
            (anchor, self.cursor)
        } else {
            (self.cursor, anchor)
        };
        self.selected = ids[lo..=hi].to_vec();
    }

    /// Toggle the message under the cursor in the selection.
    pub fn toggle_select(&mut self, mb: &Mailbox) {
        let ids = mb.ids_in(self.view);
        let Some(id) = ids.get(clamp_index(self.cursor, ids.len())).copied() else {
            return;
        };
        if self.selected.contains(&id) {
            self.selected.retain(|s| *s != id);
        } else {
            self.selected.push(id);
        }
        // Keep the selection in view order (newest first) for stable rendering.
        self.selected.sort_by_key(|s| {
            ids.iter()
                .position(|c| c == s)
                .unwrap_or(usize::MAX)
        });
    }

    pub fn clear_selection(&mut self) {
        self.selected.clear();
        self.anchor = None;
    }

    pub fn selected(&self) -> Vec<MessageId> {
        self.selected.clone()
    }

    pub fn is_selected(&self, id: MessageId) -> bool {
        self.selected.contains(&id)
    }

    /// The selection if non-empty, otherwise the message under the cursor.
    pub fn targets(&self, mb: &Mailbox) -> Vec<MessageId> {
        if !self.selected.is_empty() {
            return self.selected.clone();
        }
        let ids = mb.ids_in(self.view);
        ids.get(clamp_index(self.cursor, ids.len()))
            .map_or_else(Vec::new, |id| vec![*id])
    }

    /// Move `targets()` to `state`, then clear the selection and re-clamp the
    /// cursor. Returns the number of messages changed.
    pub fn apply(&mut self, mb: &mut Mailbox, state: TriageState) -> usize {
        let changed = mb.set_state(&self.targets(mb), state);
        self.clear_selection();
        self.clamp(mb);
        changed
    }

    /// Move every message from the cursor's sender to `state`, then re-clamp
    /// the cursor. Returns the number of messages changed.
    pub fn apply_to_sender(&mut self, mb: &mut Mailbox, state: TriageState) -> usize {
        let Some(sender) = self
            .cursor(mb)
            .and_then(|id| mb.get(id))
            .map(|m| m.from_email.clone())
        else {
            return 0;
        };
        let changed = mb.set_state_for_sender(&sender, state);
        self.clamp(mb);
        changed
    }

    /// Switch views; resets the cursor and the selection.
    pub fn switch_view(&mut self, view: TriageState) {
        self.view = view;
        self.cursor = 0;
        self.clear_selection();
    }

    fn clamp(&mut self, mb: &Mailbox) {
        self.cursor = clamp_index(self.cursor, mb.ids_in(self.view).len());
    }
}

fn clamp_index(index: usize, len: usize) -> usize {
    index.min(len.saturating_sub(1))
}

fn shift(cursor: usize, delta: isize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    (cursor as isize + delta).clamp(0, len as isize - 1) as usize
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
