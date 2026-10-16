//! Reader tabs: one tab per thread *or* per draft, Zed-style preview and pinned tabs.
//!
//! The strip is shared by the message viewer and the composer: a thread tab shows one
//! thread and remembers which of its messages is open, a draft tab shows one durable
//! draft. Opening a message from the list goes to the *preview* tab, which the next open
//! replaces; pinning makes a tab permanent. Preview replacement is per kind, so opening a
//! draft never throws away the thread being read and opening a message never throws away a
//! half-written draft. There is at most one preview tab of each kind.
//!
//! Pure view state: nothing here is an undo step, and nothing persists. A closed tab is
//! *not* a deleted draft — the durable store owns that — so operations that remove a tab
//! return what it was showing, and the caller decides what to forget.

use crate::model::MessageId;
use crate::outgoing::DraftId;

/// What a tab is showing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TabKind {
    /// A thread of the message viewer, with the message that is open in it.
    Thread { thread: u32, msg: MessageId },
    /// A durable draft in the composer.
    Draft(DraftId),
}

impl TabKind {
    /// The preview slot this kind competes for, so one kind never replaces another.
    fn preview_slot(&self) -> Option<&'static str> {
        match self {
            Self::Thread { .. } => Some("thread"),
            Self::Draft(_) => Some("draft"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tab {
    pub kind: TabKind,
    /// Pinned tabs stay until closed; the unpinned one of each kind is the preview.
    pub pinned: bool,
}

impl Tab {
    pub fn thread(thread: u32, msg: MessageId, pinned: bool) -> Self {
        Self {
            kind: TabKind::Thread { thread, msg },
            pinned,
        }
    }

    pub fn draft(draft: DraftId, pinned: bool) -> Self {
        Self {
            kind: TabKind::Draft(draft),
            pinned,
        }
    }

    /// The thread of a reader tab.
    pub fn thread_id(&self) -> Option<u32> {
        match self.kind {
            TabKind::Thread { thread, .. } => Some(thread),
            TabKind::Draft(_) => None,
        }
    }

    /// The opened message of a reader tab.
    pub fn msg(&self) -> Option<MessageId> {
        match self.kind {
            TabKind::Thread { msg, .. } => Some(msg),
            TabKind::Draft(_) => None,
        }
    }

    /// The draft of a composer tab.
    pub fn draft_id(&self) -> Option<DraftId> {
        match self.kind {
            TabKind::Draft(d) => Some(d),
            TabKind::Thread { .. } => None,
        }
    }

    pub fn is_draft(&self) -> bool {
        matches!(self.kind, TabKind::Draft(_))
    }

    /// The subject a reader tab is named after, for the tab bar.
    pub fn thread_of(&self) -> Option<u32> {
        self.thread_id()
    }
}

#[derive(Clone, Debug, Default)]
pub struct Tabs {
    tabs: Vec<Tab>,
    /// Index of the active tab; meaningful only while `tabs` is not empty.
    active: usize,
}

/// A removed tab: what it was showing, so the caller can drop that per-tab state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Closed {
    Thread(u32),
    Draft(DraftId),
}

impl Closed {
    pub fn thread(&self) -> Option<u32> {
        match self {
            Self::Thread(t) => Some(*t),
            Self::Draft(_) => None,
        }
    }
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

    /// The opened message of the active tab, when it is a reader tab.
    pub fn opened(&self) -> Option<MessageId> {
        self.active().and_then(Tab::msg)
    }

    /// The draft of the active tab, when it is a composer tab.
    pub fn active_draft(&self) -> Option<DraftId> {
        self.active().and_then(Tab::draft_id)
    }

    pub fn index_of(&self, thread: u32) -> Option<usize> {
        self.tabs.iter().position(|t| t.thread_id() == Some(thread))
    }

    pub fn tab_for(&self, thread: u32) -> Option<&Tab> {
        self.index_of(thread).map(|ix| &self.tabs[ix])
    }

    pub fn index_of_draft(&self, draft: DraftId) -> Option<usize> {
        self.tabs.iter().position(|t| t.draft_id() == Some(draft))
    }

    /// The tab showing exactly `kind`, for a drag order that carries reader tabs of either
    /// kind.
    pub fn index_of_kind(&self, kind: &TabKind) -> Option<usize> {
        self.tabs.iter().position(|t| &t.kind == kind)
    }

    pub fn tab_for_draft(&self, draft: DraftId) -> Option<&Tab> {
        self.index_of_draft(draft).map(|ix| &self.tabs[ix])
    }

    /// Whether some tab shows `draft`.
    pub fn has_draft(&self, draft: DraftId) -> bool {
        self.index_of_draft(draft).is_some()
    }

    /// Every draft that currently has a tab, oldest first.
    pub fn draft_ids(&self) -> Vec<DraftId> {
        self.tabs.iter().filter_map(Tab::draft_id).collect()
    }

    /// Open `msg` (of `thread`) as a preview and activate it. A thread that already has a tab
    /// keeps it (and its pin state) and just shows `msg`; otherwise the preview tab is replaced
    /// in place, or a new preview tab is appended. Returns the thread of a replaced preview.
    pub fn open(&mut self, thread: u32, msg: MessageId) -> Option<u32> {
        if let Some(ix) = self.index_of(thread) {
            self.tabs[ix].kind = TabKind::Thread { thread, msg };
            self.active = ix;
            return None;
        }
        let fresh = Tab::thread(thread, msg, false);
        if let Some(ix) = self.preview_slot(&TabKind::Thread { thread, msg }) {
            let replaced = std::mem::replace(&mut self.tabs[ix], fresh);
            self.active = ix;
            return replaced.thread_id();
        }
        self.push(fresh);
        None
    }

    /// Open `msg` and pin its tab.
    pub fn open_pinned(&mut self, thread: u32, msg: MessageId) -> Option<u32> {
        let replaced = self.open(thread, msg);
        self.pin(thread);
        replaced
    }

    /// Show `draft` in the shared strip and activate it, as the draft preview. A draft that
    /// already has a tab keeps it; otherwise the unpinned *draft* tab is replaced. Returns the
    /// draft of a replaced preview tab, whose retained draft is untouched by this.
    pub fn open_draft(&mut self, draft: DraftId) -> Option<DraftId> {
        if let Some(ix) = self.index_of_draft(draft) {
            self.active = ix;
            return None;
        }
        let fresh = Tab::draft(draft, false);
        if let Some(ix) = self.preview_slot(&TabKind::Draft(draft)) {
            let replaced = std::mem::replace(&mut self.tabs[ix], fresh);
            self.active = ix;
            return replaced.draft_id();
        }
        self.push(fresh);
        None
    }

    /// Show `draft` in its own permanent tab: a draft the user is holding on to.
    pub fn open_draft_pinned(&mut self, draft: DraftId) -> Option<DraftId> {
        let replaced = self.open_draft(draft);
        if let Some(ix) = self.index_of_draft(draft) {
            self.tabs[ix].pinned = true;
        }
        replaced
    }

    /// The first unpinned tab of `kind`'s kind, which is the slot a new preview replaces.
    fn preview_slot(&self, kind: &TabKind) -> Option<usize> {
        let want = kind.preview_slot();
        self.tabs
            .iter()
            .position(|t| !t.pinned && t.kind.preview_slot() == want)
    }

    fn push(&mut self, tab: Tab) {
        self.tabs.push(tab);
        self.active = self.tabs.len() - 1;
    }

    /// Pin the tab of `thread`, if there is one.
    pub fn pin(&mut self, thread: u32) {
        if let Some(ix) = self.index_of(thread) {
            self.tabs[ix].pinned = true;
        }
    }

    /// Keep the draft's tab permanent.
    pub fn pin_draft(&mut self, draft: DraftId) {
        if let Some(ix) = self.index_of_draft(draft) {
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
            self.tabs[ix].kind = TabKind::Thread { thread, msg };
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
        let active = self.active().map(|t| t.kind.clone());
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
        if let Some(kind) = active
            && let Some(ix) = self.index_of_kind(&kind)
        {
            self.active = ix;
        }
        true
    }

    /// Move the tabs into `order` (reader tabs, left to right), keeping the tab that was
    /// active active and every pin with its tab. False when `order` is not exactly the tabs it
    /// holds, so an order from somewhere else can never drop or duplicate a tab.
    pub fn reorder_to(&mut self, order: &[TabKind]) -> bool {
        if order.len() != self.tabs.len() || order.iter().any(|kind| self.index_of_kind(kind).is_none()) {
            return false;
        }
        let active = self.active().map(|t| t.kind.clone());
        let mut changed = false;
        for (ix, kind) in order.iter().enumerate() {
            let from = self.index_of_kind(kind).expect("checked above");
            if from != ix {
                self.reorder(from, ix);
                changed = true;
            }
        }
        // `reorder` keeps the active tab active; keep the field in step with its tab.
        if let Some(kind) = active
            && let Some(ix) = self.index_of_kind(&kind)
        {
            self.active = ix;
        }
        changed
    }

    /// Close tab `ix`. The active tab falls to its right neighbour, else its left one.
    /// Closing a draft tab only stops showing it; the draft itself is retained.
    pub fn close(&mut self, ix: usize) -> Option<Closed> {
        if ix >= self.tabs.len() {
            return None;
        }
        let closed = self.tabs.remove(ix);
        if ix < self.active {
            self.active -= 1;
        }
        self.active = self.active.min(self.tabs.len().saturating_sub(1));
        Some(match closed.kind {
            TabKind::Thread { thread, .. } => Closed::Thread(thread),
            TabKind::Draft(draft) => Closed::Draft(draft),
        })
    }

    /// Drop every preview thread tab whose thread `keep` rejects (pinned tabs and draft tabs
    /// always stay). Returns the dropped threads.
    pub fn retain_previews(&mut self, keep: impl Fn(u32) -> bool) -> Vec<u32> {
        let mut dropped = Vec::new();
        let mut active = self.active;
        let mut ix = 0;
        while ix < self.tabs.len() {
            if self.tabs[ix].is_draft()
                || self.tabs[ix].pinned
                || keep(self.tabs[ix].thread_id().unwrap_or_default())
            {
                ix += 1;
                continue;
            }
            let tab = self.tabs.remove(ix);
            if let Some(thread) = tab.thread_id() {
                dropped.push(thread);
            }
            if ix < active {
                active -= 1;
            }
        }
        self.active = active.min(self.tabs.len().saturating_sub(1));
        dropped
    }

    /// Whether some thread tab is an unpinned preview.
    pub fn has_preview(&self) -> bool {
        self.tabs.iter().any(|t| !t.pinned && !t.is_draft())
    }

    /// Whether any draft has a tab, pinned or not.
    pub fn has_drafts(&self) -> bool {
        self.tabs.iter().any(Tab::is_draft)
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
