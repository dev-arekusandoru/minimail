use super::*;

/// Cursor + multi-selection over one view.
pub struct Triage {
    pub view: View,
    cursor: usize,
    selected: Vec<MessageId>,
    anchor: Option<usize>,
}

impl Triage {
    pub fn new(view: View) -> Self {
        Self {
            view,
            cursor: 0,
            selected: Vec::new(),
            anchor: None,
        }
    }
    fn ids(&self, mb: &Mailbox) -> Vec<MessageId> {
        mb.ids_in_view(&self.view)
    }
    pub fn cursor_index(&self) -> usize {
        self.cursor
    }
    pub fn cursor(&self, mb: &Mailbox) -> Option<MessageId> {
        let ids = self.ids(mb);
        ids.get(clamp_index(self.cursor, ids.len())).copied()
    }
    pub fn move_cursor(&mut self, mb: &Mailbox, delta: isize) {
        self.cursor = shift(self.cursor, delta, self.ids(mb).len());
    }
    pub fn extend(&mut self, mb: &Mailbox, delta: isize) {
        let ids = self.ids(mb);
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
    pub fn toggle_select(&mut self, mb: &Mailbox) {
        let ids = self.ids(mb);
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
    pub fn set_cursor(&mut self, mb: &Mailbox, index: usize) {
        self.cursor = clamp_index(index, self.ids(mb).len());
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
    pub fn targets(&self, mb: &Mailbox) -> Vec<MessageId> {
        if !self.selected.is_empty() {
            return self.selected.clone();
        }
        let ids = self.ids(mb);
        ids.get(clamp_index(self.cursor, ids.len()))
            .map_or_else(Vec::new, |id| vec![*id])
    }
    pub fn apply(&mut self, mb: &mut Mailbox, state: TriageState) -> usize {
        let ids = self.targets(mb);
        let changed = mb.set_state(&ids, state);
        self.clear_selection();
        self.clamp(mb);
        changed
    }
    pub fn apply_to_sender(&mut self, mb: &mut Mailbox, state: TriageState) -> usize {
        let Some(sender) = self
            .cursor(mb)
            .and_then(|id| mb.get(id))
            .map(|m| m.from_email.clone())
        else {
            return 0;
        };
        let changed = mb.set_state_for_sender(&sender, true, state);
        self.clamp(mb);
        changed
    }
    pub fn switch_view(&mut self, view: View) {
        self.view = view;
        self.cursor = 0;
        self.clear_selection();
    }
    fn clamp(&mut self, mb: &Mailbox) {
        self.cursor = clamp_index(self.cursor, self.ids(mb).len());
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
