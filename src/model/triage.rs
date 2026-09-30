use super::*;

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

    /// Put the cursor at `index`, clamped to the view.
    pub fn set_cursor(&mut self, mb: &Mailbox, index: usize) {
        self.cursor = clamp_index(index, mb.ids_in(self.view).len());
    }

    /// Replace the selection with `ids` (kept in the given order).
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
