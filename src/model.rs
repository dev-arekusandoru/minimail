//! Pure triage logic: messages, the exactly-one-state invariant, undo, and a
//! cursor/selection model over a single view.
//!
//! No UI types live here, so all of it is headlessly testable.

use std::collections::HashMap;

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

/// One reversible mutation: the states it overwrote, plus the sent reply it
/// appended (if any).
#[derive(Clone, Debug)]
struct UndoStep {
    states: Vec<(MessageId, TriageState)>,
    reply: Option<Reply>,
}

pub struct Mailbox {
    messages: Vec<Message>,
    /// Message id -> index into `messages`, for O(1) lookup.
    index: HashMap<MessageId, usize>,
    /// Every id ordered newest-first by `received`; `ids_in` filters this.
    newest_first: Vec<MessageId>,
    undo: Vec<UndoStep>,
    sent: Vec<Reply>,
}

impl Mailbox {
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        let messages: Vec<Message> = serde_json::from_str(json)?;
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
        Ok(Self {
            messages,
            index,
            newest_first,
            undo: Vec::new(),
            sent: Vec::new(),
        })
    }

    pub fn load_default() -> Self {
        Self::from_json(include_str!("../fixtures/mailbox.json"))
            .expect("fixtures/mailbox.json parses")
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

    /// Ids currently in `state`, newest first.
    pub fn ids_in(&self, state: TriageState) -> Vec<MessageId> {
        self.newest_first
            .iter()
            .copied()
            .filter(|id| self.state_of(*id) == Some(state))
            .collect()
    }

    pub fn count(&self, state: TriageState) -> usize {
        self.messages.iter().filter(|m| m.state == state).count()
    }

    /// Set `state` on every id in `ids`. One undo entry per call; unknown ids
    /// and ids already in `state` are skipped. Returns the number changed.
    pub fn set_state(&mut self, ids: &[MessageId], state: TriageState) -> usize {
        let mut states = Vec::new();
        for id in ids {
            let Some(&i) = self.index.get(id) else {
                continue;
            };
            if self.messages[i].state == state {
                continue;
            }
            states.push((*id, self.messages[i].state));
            self.messages[i].state = state;
        }
        if states.is_empty() {
            return 0;
        }
        let changed = states.len();
        self.undo.push(UndoStep {
            states,
            reply: None,
        });
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

    /// Undo the last mutation. Returns false when there is nothing to undo.
    pub fn undo(&mut self) -> bool {
        let Some(step) = self.undo.pop() else {
            return false;
        };
        for (id, state) in step.states {
            if let Some(&i) = self.index.get(&id) {
                self.messages[i].state = state;
            }
        }
        if step.reply.is_some() {
            self.sent.pop();
        }
        true
    }

    /// Record a reply to `to` and move the original to `Waiting`. Exactly one
    /// undo entry, which also retracts the sent reply. A no-op (no entry, no
    /// sent reply) if `to` is unknown.
    pub fn send_reply(&mut self, to: MessageId, body: String) {
        let Some(&i) = self.index.get(&to) else {
            return;
        };
        let reply = Reply {
            in_reply_to: to,
            body,
        };
        let mut states = Vec::new();
        if self.messages[i].state != TriageState::Waiting {
            states.push((to, self.messages[i].state));
            self.messages[i].state = TriageState::Waiting;
        }
        self.sent.push(reply.clone());
        self.undo.push(UndoStep {
            states,
            reply: Some(reply),
        });
    }

    pub fn sent(&self) -> &[Reply] {
        &self.sent
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
