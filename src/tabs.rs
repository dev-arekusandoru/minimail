//! Reader tabs: one tab per thread, Zed-style preview and pinned tabs.
//!
//! A tab shows one thread and remembers which of its messages is the opened one. Opening a
//! message from the list goes to the *preview* tab, which the next open replaces; pinning makes
//! a tab permanent. There is at most one preview tab. Pure view state: nothing here is an undo
//! step, and nothing persists.
//!
//! Operations that remove a tab return the thread it showed, so the caller can drop that
//! thread's per-tab state.

use crate::model::MessageId;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tab {
    pub thread: u32,
    /// The opened message of the thread.
    pub msg: MessageId,
    /// Pinned tabs stay until closed; the unpinned one is the preview.
    pub pinned: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Tabs {
    tabs: Vec<Tab>,
    /// Index of the active tab; meaningful only while `tabs` is not empty.
    active: usize,
}

impl Tabs {
    pub fn tabs(&self) -> &[Tab] {
        &self.tabs
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    pub fn active_index(&self) -> Option<usize> {
        (!self.tabs.is_empty()).then_some(self.active)
    }

    pub fn active(&self) -> Option<&Tab> {
        self.tabs.get(self.active)
    }

    /// The opened message of the active tab.
    pub fn opened(&self) -> Option<MessageId> {
        self.active().map(|t| t.msg)
    }

    pub fn index_of(&self, thread: u32) -> Option<usize> {
        self.tabs.iter().position(|t| t.thread == thread)
    }

    pub fn tab_for(&self, thread: u32) -> Option<&Tab> {
        self.index_of(thread).map(|ix| &self.tabs[ix])
    }

    /// Open `msg` (of `thread`) as a preview and activate it. A thread that already has a tab
    /// keeps it (and its pin state) and just shows `msg`; otherwise the preview tab is replaced
    /// in place, or a new preview tab is appended. Returns the thread of a replaced preview.
    pub fn open(&mut self, thread: u32, msg: MessageId) -> Option<u32> {
        if let Some(ix) = self.index_of(thread) {
            self.tabs[ix].msg = msg;
            self.active = ix;
            return None;
        }
        let fresh = Tab { thread, msg, pinned: false };
        if let Some(ix) = self.tabs.iter().position(|t| !t.pinned) {
            let replaced = std::mem::replace(&mut self.tabs[ix], fresh);
            self.active = ix;
            return Some(replaced.thread);
        }
        self.tabs.push(fresh);
        self.active = self.tabs.len() - 1;
        None
    }

    /// Open `msg` and pin its tab.
    pub fn open_pinned(&mut self, thread: u32, msg: MessageId) -> Option<u32> {
        let replaced = self.open(thread, msg);
        self.pin(thread);
        replaced
    }

    /// Pin the tab of `thread`, if there is one.
    pub fn pin(&mut self, thread: u32) {
        if let Some(ix) = self.index_of(thread) {
            self.tabs[ix].pinned = true;
        }
    }

    pub fn pin_active(&mut self) {
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.pinned = true;
        }
    }

    /// Change which message the tab of `thread` has opened, without activating it.
    pub fn set_msg(&mut self, thread: u32, msg: MessageId) {
        if let Some(ix) = self.index_of(thread) {
            self.tabs[ix].msg = msg;
        }
    }

    /// Make tab `ix` active; false when there is no such tab.
    pub fn activate(&mut self, ix: usize) -> bool {
        if ix >= self.tabs.len() {
            return false;
        }
        self.active = ix;
        true
    }

    /// Step the active tab by `delta`, wrapping at both ends. False with fewer than two tabs.
    pub fn cycle(&mut self, delta: isize) -> bool {
        let n = self.tabs.len() as isize;
        if n < 2 {
            return false;
        }
        self.active = (self.active as isize + delta).rem_euclid(n) as usize;
        true
    }

    /// Move the tab at `from` to index `to` (where it ends up), keeping the tab that was active
    /// active. False when either index is out of range. Pin state travels with the tab.
    pub fn reorder(&mut self, from: usize, to: usize) -> bool {
        if from >= self.tabs.len() || to >= self.tabs.len() {
            return false;
        }
        let active = self.active().map(|t| t.thread);
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
        if let Some(thread) = active
            && let Some(ix) = self.index_of(thread)
        {
            self.active = ix;
        }
        true
    }

    /// Close tab `ix`. The active tab falls to its right neighbour, else its left one.
    pub fn close(&mut self, ix: usize) -> Option<u32> {
        if ix >= self.tabs.len() {
            return None;
        }
        let closed = self.tabs.remove(ix);
        if ix < self.active {
            self.active -= 1;
        }
        self.active = self.active.min(self.tabs.len().saturating_sub(1));
        Some(closed.thread)
    }

    /// Drop every preview tab whose thread `keep` rejects (pinned tabs always stay). Returns
    /// the dropped threads.
    pub fn retain_previews(&mut self, keep: impl Fn(u32) -> bool) -> Vec<u32> {
        let mut dropped = Vec::new();
        let mut active = self.active;
        let mut ix = 0;
        while ix < self.tabs.len() {
            if self.tabs[ix].pinned || keep(self.tabs[ix].thread) {
                ix += 1;
                continue;
            }
            dropped.push(self.tabs.remove(ix).thread);
            if ix < active {
                active -= 1;
            }
        }
        self.active = active.min(self.tabs.len().saturating_sub(1));
        dropped
    }

    /// Whether some tab is an unpinned preview.
    pub fn has_preview(&self) -> bool {
        self.tabs.iter().any(|t| !t.pinned)
    }
}

/// Tab label for a thread subject: `(no subject)` when blank, cut to `max` characters with `…`.
pub fn title(subject: &str, max: usize) -> String {
    let subject = subject.trim();
    if subject.is_empty() {
        return "(no subject)".to_owned();
    }
    if subject.chars().count() <= max {
        return subject.to_owned();
    }
    let cut: String = subject.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}
