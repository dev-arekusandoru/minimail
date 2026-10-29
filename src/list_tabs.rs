//! Tabs on the message list: which views of the mailbox are open, and which one is showing.
//!
//! The reader's tabs (`crate::tabs`) are keyed by thread; these are keyed by the query a view
//! holds, so "Inbox" and "Inbox + unread + from:alice" are two tabs. Preview and pin work the
//! same way as the reader's: a preview is replaced in place by the next view, and only pinned
//! tabs accumulate. A view that is already open is activated rather than opened twice.
//!
//! The list pane always shows something, so the last tab cannot be closed.

use crate::search::Query;

/// One open list tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListTab {
    /// The view this tab shows — its location and filters, which is what identifies it.
    pub query: Query,
    /// Pinned tabs are kept when a preview is replaced; only pinned tabs accumulate.
    pub pinned: bool,
}

/// The list pane's tabs, in strip order.
#[derive(Default)]
pub struct ListTabs {
    tabs: Vec<ListTab>,
    active: usize,
}

impl ListTabs {
    /// A strip holding `query` as its one, pinned tab. A window opens with one of these, which
    /// is also what keeps the strip from ever being empty.
    pub fn new(query: Query) -> Self {
        Self { tabs: vec![ListTab { query, pinned: true }], active: 0 }
    }

    /// Show `query`, the way clicking a folder does: an already open view is activated, and
    /// otherwise the preview tab is replaced in place, or a new one appended.
    pub fn open(&mut self, query: Query) -> usize {
        if let Some(ix) = self.index_of(&query) {
            self.active = ix;
            return ix;
        }
        if let Some(ix) = self.preview_ix() {
            self.tabs[ix].query = query;
            self.active = ix;
            return ix;
        }
        self.tabs.push(ListTab { query, pinned: false });
        self.active = self.tabs.len() - 1;
        self.active
    }

    /// Show `query` as a pinned tab, the way dropping a folder on the strip does: an already
    /// open view is activated (and promoted), otherwise a pinned tab is appended.
    pub fn open_pinned(&mut self, query: Query) -> usize {
        if let Some(ix) = self.index_of(&query) {
            self.tabs[ix].pinned = true;
            self.active = ix;
            return ix;
        }
        self.tabs.push(ListTab { query, pinned: true });
        self.active = self.tabs.len() - 1;
        self.active
    }

    /// Point tab `ix` at `query` in place. False for an unknown index, and when another tab
    /// already shows `query` (the caller then closes this one: a view is open once).
    pub fn retarget(&mut self, ix: usize, query: Query) -> bool {
        if ix >= self.tabs.len() || self.tabs.iter().enumerate().any(|(i, t)| i != ix && t.query == query) {
            return false;
        }
        self.tabs[ix].query = query;
        true
    }

    /// Pin tab `ix`, so the next preview replaces something else.
    pub fn pin(&mut self, ix: usize) -> bool {
        match self.tabs.get_mut(ix) {
            Some(tab) => {
                tab.pinned = true;
                true
            }
            None => false,
        }
    }

    /// Close tab `ix`. The active tab falls to its right neighbour, else its left one. False
    /// for an unknown index, and for the last tab: the list always shows something.
    pub fn close(&mut self, ix: usize) -> bool {
        if ix >= self.tabs.len() || self.tabs.len() == 1 {
            return false;
        }
        self.tabs.remove(ix);
        if ix < self.active {
            self.active -= 1;
        } else if ix == self.active {
            self.active = self.active.min(self.tabs.len() - 1);
        }
        true
    }

    /// Show tab `ix`. False for an unknown index.
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
        let active = self.active().map(|t| t.query.clone());
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
        if let Some(query) = active
            && let Some(ix) = self.index_of(&query)
        {
            self.active = ix;
        }
        true
    }

    /// Move the tabs into `order` (the views, left to right), keeping the tab that was active
    /// active. False when `order` is not exactly the tabs held, so an order from somewhere else
    /// can never drop or duplicate a tab. This is how a tab dragged in the strip reaches us.
    pub fn reorder_to(&mut self, order: &[Query]) -> bool {
        if order.len() != self.tabs.len() || order.iter().any(|query| self.index_of(query).is_none()) {
            return false;
        }
        let active = self.active().map(|t| t.query.clone());
        let mut changed = false;
        for (ix, query) in order.iter().enumerate() {
            let from = self.index_of(query).expect("checked above");
            if from != ix {
                self.reorder(from, ix);
                changed = true;
            }
        }
        if let Some(query) = active
            && let Some(ix) = self.index_of(&query)
        {
            self.active = ix;
        }
        changed
    }

    /// The tabs, in strip order.
    pub fn tabs(&self) -> &[ListTab] {
        &self.tabs
    }

    /// The showing tab.
    pub fn active(&self) -> Option<&ListTab> {
        self.tabs.get(self.active)
    }

    /// Where the showing tab sits.
    pub fn active_index(&self) -> usize {
        self.active
    }

    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    /// True only for a strip that was never seeded. A `ListTabs` built with [`Self::new`] always
    /// holds the view it was given, and nothing can close the last tab.
    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    /// The tab showing `query`, if one is open.
    pub fn index_of(&self, query: &Query) -> Option<usize> {
        self.tabs.iter().position(|tab| &tab.query == query)
    }

    /// The tab a preview would replace: the first unpinned one.
    fn preview_ix(&self) -> Option<usize> {
        self.tabs.iter().position(|tab| !tab.pinned)
    }
}
