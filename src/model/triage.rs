use super::*;

/// Cursor + multi-selection over the list a [`Query`] selects.
pub struct Triage {
    pub query: Query,
    cursor: usize,
    selected: Vec<MessageId>,
    anchor: Option<usize>,
}

impl Triage {
    pub fn new(query: Query) -> Self {
        Self {
            query,
            cursor: 0,
            selected: Vec::new(),
            anchor: None,
        }
    }
    fn ids(&self, mb: &Mailbox, now: Timestamp) -> Vec<MessageId> {
        mb.ids_matching(&self.query, now)
    }
    pub fn cursor_index(&self) -> usize {
        self.cursor
    }
    pub fn cursor(&self, mb: &Mailbox, now: Timestamp) -> Option<MessageId> {
        let ids = self.ids(mb, now);
        ids.get(clamp_index(self.cursor, ids.len())).copied()
    }
    pub fn move_cursor(&mut self, mb: &Mailbox, now: Timestamp, delta: isize) {
        self.cursor = shift(self.cursor, delta, self.ids(mb, now).len());
    }
    pub fn extend(&mut self, mb: &Mailbox, now: Timestamp, delta: isize) {
        let ids = self.ids(mb, now);
        if ids.is_empty() {
            self.cursor = 0;
            self.clear_selection();
            return;
        }
        let anchor = *self
            .anchor
            .get_or_insert(clamp_index(self.cursor, ids.len()));
        self.cursor = shift(self.cursor, delta, ids.len());
        let (lo, hi) = if anchor <= self.cursor {
            (anchor, self.cursor)
        } else {
            (self.cursor, anchor)
        };
        self.selected = ids[lo..=hi].to_vec();
    }
    pub fn toggle_select(&mut self, mb: &Mailbox, now: Timestamp) {
        let ids = self.ids(mb, now);
        let Some(id) = ids.get(clamp_index(self.cursor, ids.len())).copied() else {
            return;
        };
        if self.selected.contains(&id) {
            self.selected.retain(|s| *s != id);
        } else {
            self.selected.push(id);
        }
        self.selected
            .sort_by_key(|s| ids.iter().position(|c| c == s).unwrap_or(usize::MAX));
    }
    pub fn set_cursor(&mut self, mb: &Mailbox, now: Timestamp, index: usize) {
        self.cursor = clamp_index(index, self.ids(mb, now).len());
    }
    pub fn set_selection(&mut self, ids: Vec<MessageId>) {
        self.selected = ids;
        if self.selected.is_empty() {
            self.anchor = None;
        }
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
    pub fn targets(&self, mb: &Mailbox, now: Timestamp) -> Vec<MessageId> {
        if !self.selected.is_empty() {
            return self.selected.clone();
        }
        let ids = self.ids(mb, now);
        ids.get(clamp_index(self.cursor, ids.len()))
            .map_or_else(Vec::new, |id| vec![*id])
    }
    pub fn apply(&mut self, mb: &mut Mailbox, now: Timestamp, state: TriageState) -> usize {
        let ids = self.targets(mb, now);
        let changed = mb.set_state(&ids, state);
        self.clear_selection();
        self.clamp(mb, now);
        changed
    }
    /// Move every message from the cursor's sender that shares the cursor
    /// message's state to `state`, then re-clamp the cursor.
    pub fn apply_to_sender(&mut self, mb: &mut Mailbox, now: Timestamp, state: TriageState) -> usize {
        let Some((sender, from)) = self
            .cursor(mb, now)
            .and_then(|id| mb.get(id))
            .map(|m| (m.from_email.clone(), m.state))
        else {
            return 0;
        };
        let changed = mb.set_state_for_sender(&sender, from, state);
        self.clamp(mb, now);
        changed
    }
    /// Point the list at `query`: the cursor goes to the top and the selection clears.
    pub fn set_query(&mut self, query: Query) {
        self.query = query;
        self.cursor = 0;
        self.clear_selection();
    }
    fn clamp(&mut self, mb: &Mailbox, now: Timestamp) {
        self.cursor = clamp_index(self.cursor, self.ids(mb, now).len());
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
