//! Pure thread grouping and navigation over `Message::thread_id`.
//!
//! Mock data carries a stable `thread_id` per message, so threads are keyed on it
//! directly. Everything here is UI-free and headlessly testable.
//!
//! Grouping is always *per panel*: callers pass only the ids one panel shows, so a
//! thread whose messages sit in different triage states yields one group in each
//! panel, containing just that panel's messages.

use std::collections::HashSet;

use crate::model::{Message, MessageId};

/// The messages of one thread inside one panel, newest first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadGroup {
    pub thread_id: u32,
    pub ids: Vec<MessageId>,
}

impl ThreadGroup {
    /// Newest message of the group.
    pub fn latest(&self) -> MessageId {
        self.ids[0]
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
}

/// One rendered list row when grouping is on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    /// A thread with a single message in this panel: an ordinary message row.
    Single(MessageId),
    /// Collapsed or expanded thread header for two or more messages.
    Header {
        thread_id: u32,
        /// Messages of the thread in this panel, newest first.
        ids: Vec<MessageId>,
        expanded: bool,
    },
    /// A message under an expanded header.
    Child { thread_id: u32, id: MessageId },
}

impl Row {
    /// Message shown by the row and opened by it (the newest one for a header).
    pub fn primary(&self) -> MessageId {
        match self {
            Row::Single(id) | Row::Child { id, .. } => *id,
            Row::Header { ids, .. } => ids[0],
        }
    }

    /// Every message a triage action on this row applies to.
    pub fn ids(&self) -> Vec<MessageId> {
        match self {
            Row::Single(id) | Row::Child { id, .. } => vec![*id],
            Row::Header { ids, .. } => ids.clone(),
        }
    }

    pub fn thread_id(&self) -> Option<u32> {
        match self {
            Row::Single(_) => None,
            Row::Header { thread_id, .. } | Row::Child { thread_id, .. } => Some(*thread_id),
        }
    }
}

/// Group `ids` (already ordered newest first) by thread. Groups are ordered by
/// their newest message; each group's ids keep the input order.
pub fn group<'a>(
    ids: &[MessageId],
    get: impl Fn(MessageId) -> Option<&'a Message>,
) -> Vec<ThreadGroup> {
    let mut groups: Vec<ThreadGroup> = Vec::new();
    for &id in ids {
        let Some(m) = get(id) else { continue };
        match groups.iter_mut().find(|g| g.thread_id == m.thread_id) {
            Some(g) => g.ids.push(id),
            None => groups.push(ThreadGroup {
                thread_id: m.thread_id,
                ids: vec![id],
            }),
        }
    }
    groups
}

/// Flatten groups into list rows; threads in `expanded` list their messages
/// (newest first) under the header.
pub fn rows(groups: &[ThreadGroup], expanded: &HashSet<u32>) -> Vec<Row> {
    let mut out = Vec::new();
    for g in groups {
        if g.len() == 1 {
            out.push(Row::Single(g.ids[0]));
            continue;
        }
        let open = expanded.contains(&g.thread_id);
        out.push(Row::Header {
            thread_id: g.thread_id,
            ids: g.ids.clone(),
            expanded: open,
        });
        if open {
            out.extend(g.ids.iter().map(|&id| Row::Child {
                thread_id: g.thread_id,
                id,
            }));
        }
    }
    out
}

/// Distinct sender names of `ids`, oldest message first.
pub fn participants<'a>(
    ids: &[MessageId],
    get: impl Fn(MessageId) -> Option<&'a Message>,
) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for &id in ids.iter().rev() {
        if let Some(m) = get(id)
            && !names.contains(&m.from_name)
        {
            names.push(m.from_name.clone());
        }
    }
    names
}

/// Every message of a thread (any state), oldest first; ties broken by id.
pub fn thread_order(messages: &[Message], thread_id: u32) -> Vec<MessageId> {
    let mut msgs: Vec<&Message> = messages.iter().filter(|m| m.thread_id == thread_id).collect();
    msgs.sort_by(|a, b| a.received.cmp(&b.received).then(a.id.cmp(&b.id)));
    msgs.into_iter().map(|m| m.id).collect()
}

/// Message `delta` steps from `from` in `order`, or `None` past either end.
pub fn step(order: &[MessageId], from: MessageId, delta: isize) -> Option<MessageId> {
    let at = order.iter().position(|&id| id == from)? as isize;
    usize::try_from(at + delta)
        .ok()
        .and_then(|i| order.get(i))
        .copied()
}
