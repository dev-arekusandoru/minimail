//! What one list tab holds: the view it shows, and the list state that goes with it.
//!
//! A list tab is a whole view — its location and its filters — so it is [`crate::list_tabs`]
//! that says which views are open, and this what each one remembers while it is open: where
//! the cursor is, what is selected, how far the list is scrolled. Bundling the two means a
//! tab and its state cannot drift apart when the strip is reordered, a tab is activated, or
//! the dock mirrors a drag back into the model.

use std::cell::Cell;
use super::*;

pub struct ListView {
    /// The view: its location is the `in:` value, everything else a pill.
    pub triage: Triage,
    /// The folder the sidebar row for this view is. Its `in:` (and `account:`) values are
    /// the query's implicit part: hidden as pills until something else is applied.
    pub(super) folder: Location,
    /// Cursor over grouped rows.
    pub(super) row_cursor: usize,
    /// Shift-selection anchor over grouped rows.
    pub(super) row_anchor: Option<usize>,
    /// Variable-height list state, so every tab keeps the scroll position it was left at.
    pub(super) list_state: ListState,
    /// What `list_state` was last measured against, so a stale row height is re-measured.
    pub(super) list_shape: Option<list::ListShape>,
    /// End of the list's visible range, from its scroll handler.
    pub(super) list_visible_end: Cell<usize>,
}

impl ListView {
    /// A fresh view of `query` over `folder`: the top of the list, nothing selected, unscrolled.
    pub(super) fn new(query: Query, folder: Location) -> Self {
        Self {
            triage: Triage::new(query),
            folder,
            row_cursor: 0,
            row_anchor: None,
            list_state: ListState::new(0, ListAlignment::Top, px(200.)),
            list_shape: None,
            list_visible_end: Cell::new(0),
        }
    }

    /// The query this tab shows, which is what identifies it in the strip.
    pub fn query(&self) -> &Query {
        &self.triage.query
    }

    /// The folder whose sidebar row this view came from.
    pub fn location(&self) -> &Location {
        &self.folder
    }

    /// Where the cursor sits in the model's own numbering.
    pub fn cursor_index(&self) -> usize {
        self.triage.cursor_index()
    }

    /// The messages selected in this tab, which no other tab's selection touches.
    pub fn selected(&self) -> Vec<MessageId> {
        self.triage.selected()
    }

    /// The row this tab's scroll sits at.
    pub fn scroll_row(&self) -> usize {
        self.list_state.logical_scroll_top().item_ix
    }
}