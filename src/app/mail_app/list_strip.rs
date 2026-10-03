//! The list strip, wired: opening a view from the sidebar or from a key, switching, closing,
//! and the state each tab keeps while it is open.
//!
//! [`crate::list_tabs`] owns the strip's order, its preview tab and which tab is showing;
//! `list_views` holds one [`ListView`] per tab, in the same order. Every operation here that
//! can change one changes the other in the same breath, so a tab can never be left holding
//! another view's cursor, and a drag in the strip puts both back the same way round.

use super::*;

/// Which pane the tab keys act on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Pane {
    /// The list strip.
    #[default]
    List,
    /// The reader strip.
    Reader,
}

impl MailApp {
    // ------------------------------------------------------------------ the active view

    /// The view the active list tab shows. The strip is never empty — the last tab cannot be
    /// closed — so there is always one.
    pub(super) fn view(&self) -> &ListView {
        &self.list_views[self.list_tabs.active_index()]
    }

    pub(super) fn view_mut(&mut self) -> &mut ListView {
        let ix = self.list_tabs.active_index();
        debug_assert!(ix < self.list_views.len(), "a view for every tab");
        &mut self.list_views[ix]
    }

    /// The state the list tab in slot `ix` keeps: where its cursor is, what it has selected
    /// and the row its scroll sits at.
    pub fn tab_state(&self, ix: usize) -> (usize, Vec<MessageId>, usize) {
        match self.list_views.get(ix) {
            Some(view) => (view.cursor_index(), view.selected(), view.scroll_row()),
            None => (0, Vec::new(), 0),
        }
    }

    /// The active tab's `Triage`: its query, its cursor and its selection.
    pub fn triage(&self) -> &Triage {
        &self.view().triage
    }

    pub fn triage_mut(&mut self) -> &mut Triage {
        &mut self.view_mut().triage
    }

    /// The folder the active view was opened at, whose sidebar row it highlights.
    pub fn folder(&self) -> &Location {
        &self.view().folder
    }

    /// The tab a preview would replace: the first unpinned one.
    fn preview_ix(&self) -> Option<usize> {
        self.list_tabs.tabs().iter().position(|tab| !tab.pinned)
    }

    // ------------------------------------------------------------------ opening

    /// Show `query` in the strip: a view that is already open is activated rather than opened
    /// twice, otherwise the preview tab takes it over in place, or a new tab is appended.
    /// `pin` makes the tab permanent, as a cmd-click or a double-click does.
    ///
    /// This is opening a view, not switching tabs, so the reader keeps what it shows.
    pub(super) fn open_view(&mut self, query: Query, folder: Location, pin: bool, cx: &mut Context<Self>) {
        match self.list_tabs.index_of(&query) {
            Some(ix) => {
                if pin {
                    self.list_tabs.pin(ix);
                }
                self.list_tabs.activate(ix);
                self.list_views[ix].folder = folder;
            }
            None if !pin && self.preview_ix().is_some() => {
                let ix = self.preview_ix().expect("just checked");
                self.list_tabs.open(query.clone());
                self.list_views[ix] = ListView::new(query, folder);
            }
            None => {
                if pin {
                    self.list_tabs.open_pinned(query.clone());
                } else {
                    self.list_tabs.open(query.clone());
                }
                self.list_views.push(ListView::new(query, folder));
            }
        }
        self.focus_pane(Pane::List);
        cx.notify();
    }

    /// Show `location` the way the sidebar does: a fresh `in:` and no other filters.
    pub(super) fn show_location(&mut self, location: Location, pin: bool, cx: &mut Context<Self>) {
        let query = self.mailbox.location_query(&location);
        self.end_session();
        self.open_view(query, location, pin, cx);
    }

    // ------------------------------------------------------------------ switching

    /// Click on the list tab at `ix`: show it, and let the reader's preview follow its
    /// cursor. A double-click or a cmd-click pins it.
    pub(super) fn click_list_tab(
        &mut self,
        ix: usize,
        clicks: usize,
        pin: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.list_tabs.activate(ix) {
            if clicks >= 2 || pin {
                self.list_tabs.pin(ix);
            }
            self.follow_active_view();
        }
        self.focus_pane(Pane::List);
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    /// Close the list tab at `ix`. The last tab cannot be closed: the list shows something.
    pub(super) fn close_list_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.list_tabs.close(ix) {
            self.list_views.remove(ix);
            self.follow_active_view();
        }
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    /// `ctrl-tab` / `ctrl-shift-tab` on the list strip: step to the next or previous tab.
    pub(super) fn cycle_list_tabs(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs_locked() {
            return;
        }
        if self.list_tabs.cycle(delta) {
            self.follow_active_view();
            window.focus(&self.focus_handle, cx);
            cx.notify();
        }
    }

    /// A tab dragged in the strip reached the dock before it reached us: put the views the
    /// same way round as the tabs, so a tab keeps the state it was left with.
    pub(super) fn adopt_dragged_order(&mut self, order: &[Query]) {
        // Where each view sits now, which is the only reading that survives the reorder.
        let Some(places) = order.iter().map(|query| self.list_tabs.index_of(query)).collect::<Option<Vec<_>>>()
        else {
            return;
        };
        if !self.list_tabs.reorder_to(order) {
            return;
        }
        let mut pool: Vec<Option<ListView>> = std::mem::take(&mut self.list_views).into_iter().map(Some).collect();
        self.list_views = places
            .into_iter()
            .filter_map(|ix| pool.get_mut(ix).and_then(|view| view.take()))
            .collect();
    }

    /// The reader's preview follows the active list tab: the message under that tab's cursor
    /// is the one the reader shows. A thread that already has a reader tab keeps it, pinned
    /// ones included — following never closes a tab or rewrites a pinned one.
    pub(super) fn follow_active_view(&mut self) {
        if self.tabs_locked() {
            return;
        }
        if let Some(id) = self.cursor_id() {
            let pane = self.pane_focus;
            self.open_message(id, false);
            // Following the cursor is not the user turning to the reader: the list stays the
            // focused pane, so the tab keys keep cycling this strip.
            self.focus_pane(pane);
        }
    }

    // ------------------------------------------------------------------ which pane

    /// The pane the tab keys act on. Touching a pane is what focuses it: the strip a tab was
    /// clicked in, the list a row was clicked in, the reader a message was opened in.
    pub(super) fn focus_pane(&mut self, pane: Pane) {
        self.pane_focus = pane;
    }

    pub(super) fn focused_pane(&self) -> Pane {
        self.pane_focus
    }

    /// `cmd-w`: close the active tab of the focused pane.
    pub(super) fn close_focused_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs_locked() {
            return;
        }
        match self.focused_pane() {
            Pane::List => self.close_active_list_tab(window, cx),
            Pane::Reader => self.close_active_reader_tab(window, cx),
        }
    }

    fn close_active_list_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_list_tab(self.list_tabs.active_index(), window, cx);
    }

    /// `ctrl-tab` / `ctrl-shift-tab`: cycle the focused pane's strip.
    pub(super) fn cycle_focused_tab(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        match self.focused_pane() {
            Pane::List => self.cycle_list_tabs(delta, window, cx),
            Pane::Reader => self.cycle_reader_tabs(delta, window, cx),
        }
    }
}