//! The centre panes as dock panels: the list and the reader as two tab groups in the
//! centre split.
//!
//! The sidebar is deliberately not one of them. It is a resizable panel of ours beside
//! the dock (see [`super::panes`]), so a tab group can be zoomed over the whole centre
//! — which the dock's own zoom could never do while the sidebar lived in its left dock.
//!
//! Every panel here is a thin proxy. It holds a [`WeakEntity<MailApp>`] plus the identity
//! it draws — a thread, or none — and calls straight back into the app's own render
//! helpers. The mailbox, the cursor, the reader tabs and every piece of per-thread reader
//! state stay in [`MailApp`], so the dock only decides where those pieces sit and the
//! existing render code survives the move unchanged.
//!
//! The list group mirrors [`crate::list_tabs::ListTabs`] the same way: one panel per open
//! view, in tab order, with the active tab displayed. A tab dragged in either strip is a
//! dock edit the model never saw, so [`Dock::sync`] reads the order the tree was left in
//! and hands it back for the model to take.
//!
//! The reader group mirrors [`crate::tabs::Tabs`]: one panel per open thread, in tab
//! order, with the active tab displayed. A panel standing for "nothing is open" keeps the
//! group alive, so the group never disappears out from under the layout.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::base::ElementExt as _;
use gpui_kit::component::dock::{
    BasePanel, DockArea, DockEvent, DockLayout, DockPlacement, DockSkin, NodeId, PaneRef, Panel, PanelControl,
    PanelEvent, PanelId, PanelStyle, panel_handle,
};

use super::panes::{
    MIN_LIST_H, MIN_LIST_W, MIN_READER_H, MIN_READER_W, Orientation, SIDEBAR_W, available_height, available_width,
};
use super::*;

pub(super) struct Dock {
    app: WeakEntity<MailApp>,
    area: Entity<DockArea>,
    /// One panel per open list tab, in [`crate::list_tabs::ListTabs`] order. The strip is
    /// never empty, so there is always at least one.
    lists: Vec<Entity<ListPanel>>,
    /// The list slot as last painted. Shared with the list panel, which measures it, so
    /// the app can read the pane's real width without leasing a panel that is mid-render.
    measured: Rc<Cell<(f32, f32)>>,
    /// The reader shown when no thread has a tab. It lives in the reader group at all
    /// times so the group is never empty, and draws only when no thread tab is there to
    /// draw instead.
    blank: Entity<ReaderPanel>,
    /// One panel per open thread, in [`crate::tabs::Tabs`] order.
    readers: Vec<Entity<ReaderPanel>>,
    /// Which way the centre splits.
    orientation: Orientation,
    /// The centre split's list slot, remembered per orientation so each keeps its own.
    sizes: [f32; 2],
    /// Width the list rows get before the slot has ever been painted.
    list_w_default: Cell<f32>,
    /// What the dock was last built for, so an ordinary frame builds nothing.
    built: Option<Built>,
    /// Which panel each group displays: the reader's as an index into `readers`, the list's
    /// as an index into `lists`.
    active: Option<usize>,
    list_active: usize,
    _subs: Vec<Subscription>,
}

/// The panel set the dock currently holds. A change to any field is the only reason to
/// rebuild the centre; a change of active tab alone is a `select_panel`.
#[derive(PartialEq, Eq, Clone)]
struct Built {
    orientation: Orientation,
    threads: Vec<u32>,
    /// The views the list group is showing, in tab order.
    views: Vec<Query>,
}

impl Dock {
    pub(super) fn new(
        app: &WeakEntity<MailApp>,
        initial: &Query,
        window: &mut Window,
        cx: &mut Context<MailApp>,
    ) -> Self {
        let app_entity = cx.entity();
        let (area, skin) = DockSkin::dock_area("mail", None, window, cx);
        // A singleton group asks for its bar: `Auto` hides the bar of a panel that declines
        // one (`title_bar`), which is how the reader's stand-in group stays bare with no tabs.
        skin.set_panel_style(PanelStyle::Auto, cx);
        // Our own close button lives in the reader tab's title, where it already was.
        skin.set_close_button_visible(false, cx);
        skin.set_toggle_button_visible(false, cx);
        let measured = Rc::new(Cell::new((0., 0.)));
        let lists = vec![cx.new(|cx| ListPanel {
            app: app.clone(),
            query: initial.clone(),
            measured: measured.clone(),
            focus: cx.focus_handle(),
            _app_sub: watch_app(&app_entity, cx),
        })];
        let blank = cx.new(|cx| ReaderPanel {
            app: app.clone(),
            thread: None,
            standalone: true,
            focus: cx.focus_handle(),
            _app_sub: watch_app(&app_entity, cx),
        });
        let mut dock = Self {
            app: app.clone(),
            area: area.clone(),
            lists,
            measured,
            blank,
            readers: Vec::new(),
            orientation: Orientation::default(),
            sizes: default_sizes(window),
            list_w_default: Cell::new(available_width(
                f32::from(window.viewport_size().width),
                SIDEBAR_W,
            )),
            built: None,
            active: None,
            list_active: 0,
            _subs: vec![
                cx.observe(&area, |_, _, cx| cx.notify()),
                cx.subscribe_in(&area, window, |app: &mut MailApp, _, event: &DockEvent, window, cx| {
                    if let DockEvent::DragDrop { item, target } = event {
                        app.drop_on_reader(item, target, window, cx);
                        app.drop_on_list(item, target, window, cx);
                    }
                    cx.notify();
                }),
            ],
        };
        // The window opens on one pinned view, so the list group starts with that one panel.
        let tabs = TabSets {
            readers: Vec::new(),
            reader: None,
            views: vec![initial.clone()],
            list: 0,
        };
        dock.rebuild_now(&tabs, window, cx);
        dock
    }

    /// The dock's element, in the slot the panes are built into.
    pub(super) fn view(&self) -> AnyElement {
        self.area.clone().into_any_element()
    }

    pub(super) fn orientation(&self) -> Orientation {
        self.orientation
    }

    pub(super) fn set_orientation(&mut self, orientation: Orientation) {
        self.orientation = orientation;
    }


    /// Bring the dock in line with the app. An ordinary frame builds nothing: only a
    /// change of panel set, orientation or activation reaches the area.
    ///
    /// Returns the orders a strip was left in after a tab was dragged in it: the group
    /// reorders without touching the model, so the app is told and puts its own tabs the
    /// same way round.
    pub(super) fn sync(
        &mut self,
        tabs: &TabSets,
        window: &mut Window,
        cx: &mut Context<MailApp>,
    ) -> Dragged {
        let TabSets { readers: threads, reader: active, views, list: list_active } = tabs;
        let mut dragged = Dragged { readers: None, lists: None };
        let wanted = Built {
            orientation: self.orientation,
            threads: threads.clone(),
            views: views.clone(),
        };
        if self.built.as_ref() != Some(&wanted) {
            let ix = active.and_then(|thread| self.reader_ix(thread, cx));
            let list_ix = views.get(*list_active).and_then(|query| self.list_ix(query, cx)).unwrap_or(0);
            self.rebuild(tabs, ix.unwrap_or(0), list_ix, window, cx);
            return dragged;
        }
        dragged.readers = self.dragged_order(threads, cx);
        dragged.lists = self.dragged_views(views, cx);
        if let Some(order) = dragged.readers.as_ref().map(|dragged| dragged.order.clone()) {
            let mut panels: Vec<(usize, Entity<ReaderPanel>)> = std::mem::take(&mut self.readers)
                .into_iter()
                .map(|panel| {
                    let rank = order.iter().position(|thread| Some(*thread) == panel.read(cx).thread);
                    (rank.unwrap_or(usize::MAX), panel)
                })
                .collect();
            panels.sort_by_key(|(rank, _)| *rank);
            self.readers = panels.into_iter().map(|(_, panel)| panel).collect();
            self.built = Some(Built {
                orientation: self.orientation,
                threads: order,
                views: views.clone(),
            });
        }
        if let Some(order) = dragged.lists.as_ref().map(|dragged| dragged.order.clone()) {
            let mut panels: Vec<(usize, Entity<ListPanel>)> = std::mem::take(&mut self.lists)
                .into_iter()
                .map(|panel| {
                    let rank = order.iter().position(|query| *query == panel.read(cx).query);
                    (rank.unwrap_or(usize::MAX), panel)
                })
                .collect();
            panels.sort_by_key(|(rank, _)| *rank);
            self.lists = panels.into_iter().map(|(_, panel)| panel).collect();
            self.built = Some(Built {
                orientation: self.orientation,
                threads: threads.clone(),
                views: order,
            });
        }
        // The active tab may have moved with the panels — or the drag may have opened it — so
        // read its slot again.
        let active = dragged.readers.as_ref().and_then(|dragged| dragged.active).or(*active);
        let ix = active.and_then(|thread| self.reader_ix(thread, cx));
        let list_active = match dragged.lists.as_ref().and_then(|dragged| dragged.active.clone()) {
            Some(query) => self.list_ix(&query, cx).unwrap_or(*list_active),
            None => *list_active,
        };
        // The layout is still the one we installed, so a divider drag since then is
        // ours to remember — that is how each orientation keeps the size it was left at.
        self.remember(cx);
        if ix != self.active {
            self.active = ix;
            if let Some(ix) = ix {
                let id = PanelId::from(self.readers[ix].entity_id());
                self.area.update(cx, |area, cx| area.select_panel(id, window, cx));
            }
        }
        if list_active != self.list_active && list_active < self.lists.len() {
            self.list_active = list_active;
            let id = PanelId::from(self.lists[list_active].entity_id());
            self.area.update(cx, |area, cx| area.select_panel(id, window, cx));
        }
        dragged
    }

    /// Rebuild now, whether or not the panel set changed: the orientation toggle and the
    /// keyboard divider moves all re-lay the centre outright, because the dock has no
    /// other way to move a centre divider.
    pub(super) fn rebuild_now(
        &mut self,
        tabs: &TabSets,
        window: &mut Window,
        cx: &mut Context<MailApp>,
    ) {
        self.built = None;
        let _ = self.sync(tabs, window, cx);
    }

    /// Rebuild the centre around `size` for the list slot. `sidebar` is the room the
    /// sidebar takes off the width axis, which is what bounds the list.
    pub(super) fn place_list(
        &mut self,
        size: f32,
        sidebar: f32,
        tabs: &TabSets,
        window: &mut Window,
        cx: &mut Context<MailApp>,
    ) {
        let orientation = self.orientation;
        let (min, max) = self.limits(sidebar, window);
        self.sizes[orientation.ix()] = size.clamp(min, max);
        self.rebuild_now(tabs, window, cx);
    }

    /// Forget both remembered sizes, so each orientation starts again from its default
    /// share of the room.
    pub(super) fn forget_sizes(
        &mut self,
        sidebar: f32,
        tabs: &TabSets,
        window: &mut Window,
        cx: &mut Context<MailApp>,
    ) {
        self.sizes = self.defaults(sidebar, window);
        self.list_w_default.set(available_width(f32::from(window.viewport_size().width), sidebar));
        self.rebuild_now(tabs, window, cx);
    }

    /// The list slot as the dock last laid it out, along the stacking axis. The dock's
    /// own tree is what says so: it holds the size the divider was dragged to and the
    /// one a keyboard move just installed, both the moment they happen.
    pub(super) fn list_pane_size(&self, cx: &App) -> f32 {
        self.slot_size(cx).unwrap_or_else(|| self.sizes[self.orientation.ix()])
    }

    /// Width the list rows get: the list slot side by side, the whole centre when stacked.
    pub(super) fn list_width(&self) -> f32 {
        let painted = self.measured.get().0;
        if painted > 0. { painted } else { self.list_w_default.get() }
    }

    /// `(smallest, largest)` the list pane may take along the stacking axis: the old
    /// per-panel ranges, which the dock's own slots do not carry.
    fn limits(&self, sidebar: f32, window: &Window) -> (f32, f32) {
        let viewport = window.viewport_size();
        let (vw, vh) = (f32::from(viewport.width), f32::from(viewport.height));
        match self.orientation {
            Orientation::SideBySide => {
                (MIN_LIST_W, (available_width(vw, sidebar) - MIN_READER_W).max(MIN_LIST_W))
            }
            Orientation::Stacked => (MIN_LIST_H, (available_height(vh) - MIN_READER_H).max(MIN_LIST_H)),
        }
    }

    /// Take the list slot's current size as this orientation's own, so the orientation
    /// toggle can put it back exactly where the divider was left.
    fn remember(&mut self, cx: &App) {
        if let Some(size) = self.slot_size(cx) {
            self.sizes[self.orientation.ix()] = size;
        }
    }

    /// The centre split's list slot, as the dock's tree holds it.
    fn slot_size(&self, cx: &App) -> Option<f32> {
        let area = self.area.read(cx);
        let split = area.layout(DockPlacement::Center).map(|tree| tree.root().kind());
        let PaneRef::Split { sizes, .. } = split? else { return None };
        sizes.first().copied().flatten().map(f32::from)
    }

    /// The reader panel showing `thread`, if the dock holds one.
    fn reader_ix(&self, thread: u32, cx: &App) -> Option<usize> {
        self.readers.iter().position(|panel| panel.read(cx).thread == Some(thread))
    }

    /// The list panel showing `query`, if the dock holds one.
    fn list_ix(&self, query: &Query, cx: &App) -> Option<usize> {
        self.lists.iter().position(|panel| &panel.read(cx).query == query)
    }

    /// Which pane the window's focus says the user last touched, read from the panels
    /// rather than from any list of interactions: a mouse-down in a group focuses that
    /// group's active panel, so a click anywhere in the reader's content — body, headers,
    /// find bar, background — is caught as surely as one on a tab. `None` when the focus
    /// is neither pane (the app's own handle, a modal's input), leaving the pane as it was.
    pub(super) fn focused_pane(&self, window: &Window, cx: &App) -> Option<Pane> {
        let holds = |panel_focus: &FocusHandle| panel_focus.contains_focused(window, cx);
        if self.lists.iter().any(|panel| holds(&panel.read(cx).focus)) {
            return Some(Pane::List);
        }
        if self.readers.iter().chain(std::iter::once(&self.blank)).any(|panel| holds(&panel.read(cx).focus)) {
            return Some(Pane::Reader);
        }
        None
    }

    /// Whether `node` is the centre group holding the reader tabs — the only drop target that
    /// opens mail. The list is a tab group too, and a drop on it means something else.
    pub(super) fn is_reader_node(&self, node: NodeId, cx: &App) -> bool {
        let readers: Vec<PanelId> = self
            .readers
            .iter()
            .chain(std::iter::once(&self.blank))
            .map(|panel| PanelId::from(panel.entity_id()))
            .collect();
        self.node_holds(node, &readers, cx)
    }

    /// Whether `node` is the centre group holding the list tabs — the drop target that opens a
    /// folder's view as a pinned tab.
    pub(super) fn is_list_node(&self, node: NodeId, cx: &App) -> bool {
        let lists: Vec<PanelId> = self.lists.iter().map(|panel| PanelId::from(panel.entity_id())).collect();
        self.node_holds(node, &lists, cx)
    }

    /// Whether the centre tree's `node` is a tab group holding any of `panels`.
    fn node_holds(&self, node: NodeId, panels: &[PanelId], cx: &App) -> bool {
        self.node_with(panels, cx) == Some(node)
    }

    /// The centre tree's tab group holding any of `panels`.
    fn node_with(&self, panels: &[PanelId], cx: &App) -> Option<NodeId> {
        let area = self.area.read(cx);
        let tree = area.layout(DockPlacement::Center)?;
        let mut found = None;
        tree.root().walk(&mut |pane| {
            if let PaneRef::Tabs { panels: held, .. } = pane.kind()
                && held.iter().any(|id| panels.contains(id))
            {
                found = Some(pane.id());
            }
        });
        found
    }

    /// The panels of the group `zoom` names, for finding that group in the tree.
    fn zoom_panels(&self, zoom: Zoom, cx: &App) -> Vec<PanelId> {
        let _ = cx;
        match zoom {
            Zoom::List => self.lists.iter().map(|panel| PanelId::from(panel.entity_id())).collect(),
            Zoom::Reader => self
                .readers
                .iter()
                .chain(std::iter::once(&self.blank))
                .map(|panel| PanelId::from(panel.entity_id()))
                .collect(),
        }
    }

    /// How many tab groups the centre holds: the list and the reader.
    pub(super) fn group_count(&self, cx: &App) -> usize {
        let area = self.area.read(cx);
        let mut groups = 0;
        if let Some(tree) = area.layout(DockPlacement::Center) {
            tree.root().walk(&mut |pane| {
                if matches!(pane.kind(), PaneRef::Tabs { .. }) {
                    groups += 1;
                }
            });
        }
        groups
    }

    /// The group that fills the dock now, if one does.
    pub(super) fn zoomed(&self, cx: &App) -> Option<Zoom> {
        let node = self.area.read(cx).zoomed_group()?;
        [Zoom::List, Zoom::Reader]
            .into_iter()
            .find(|zoom| self.node_holds(node, &self.zoom_panels(*zoom, cx), cx))
    }

    /// `alt-z`: give the dock back if a group fills it, else zoom `zoom`'s group in. A group
    /// whose displayed panel is not zoomable refuses, which is the kit's call.
    pub(super) fn toggle_zoom(&mut self, zoom: Zoom, window: &mut Window, cx: &mut Context<MailApp>) {
        if self.area.read(cx).is_zoomed() {
            self.area.update(cx, |area, cx| area.set_zoomed_out(window, cx));
        } else {
            self.zoom_in(zoom, window, cx);
        }
    }

    fn zoom_in(&mut self, zoom: Zoom, window: &mut Window, cx: &mut Context<MailApp>) {
        let panels = self.zoom_panels(zoom, cx);
        if let Some(node) = self.node_with(&panels, cx) {
            self.area.update(cx, |area, cx| area.set_zoomed_in(node, window, cx));
        }
    }

    /// The reader group's tabs as the dock's own tree now holds them, when that order differs
    /// from `threads` — a tab dragged in the strip. The model follows it, so the strip and
    /// `Tabs` can never disagree about which tab is where, or which one is open.
    fn dragged_order(&self, threads: &[u32], cx: &App) -> Option<DraggedTabs<u32>> {
        let id_of = |panel: &Entity<ReaderPanel>| PanelId::from(panel.entity_id());
        let blank = id_of(&self.blank);
        let readers: Vec<PanelId> = self.readers.iter().map(id_of).collect();
        let area = self.area.read(cx);
        let tree = area.layout(DockPlacement::Center)?;
        let thread_of = |id: &PanelId| {
            let ix = readers.iter().position(|reader| reader == id)?;
            self.readers.get(ix).and_then(|panel| panel.read(cx).thread)
        };
        let mut held = None;
        tree.root().walk(&mut |pane| {
            if let PaneRef::Tabs { panels, active_ix } = pane.kind()
                && panels.contains(&blank)
            {
                held = Some((
                    panels.iter().filter_map(&thread_of).collect::<Vec<u32>>(),
                    panels.get(active_ix).and_then(thread_of),
                ));
            }
        });
        let (order, active) = held?;
        (order != threads).then_some(DraggedTabs { order, active })
    }

    /// The list group's tabs as the dock's own tree now holds them, when that order differs
    /// from `views` — a tab dragged in the strip. The model follows it, so the strip and
    /// `ListTabs` can never disagree about which tab is where, or which one is open.
    fn dragged_views(&self, views: &[Query], cx: &App) -> Option<DraggedTabs<Query>> {
        let id_of = |panel: &Entity<ListPanel>| PanelId::from(panel.entity_id());
        let lists: Vec<PanelId> = self.lists.iter().map(id_of).collect();
        let area = self.area.read(cx);
        let tree = area.layout(DockPlacement::Center)?;
        let query_of = |id: &PanelId| {
            let ix = lists.iter().position(|list| list == id)?;
            self.lists.get(ix).map(|panel| panel.read(cx).query.clone())
        };
        let mut held = None;
        tree.root().walk(&mut |pane| {
            if let PaneRef::Tabs { panels, active_ix } = pane.kind()
                && panels.iter().any(|id| lists.contains(id))
            {
                held = Some((
                    panels.iter().filter_map(&query_of).collect::<Vec<Query>>(),
                    panels.get(active_ix).and_then(query_of),
                ));
            }
        });
        let (order, active) = held?;
        (order != views).then_some(DraggedTabs { order, active })
    }

    /// Install the whole layout: the centre's two tab groups, the list holding one panel
    /// per open view and the reader one per open thread, each with the tab it is showing
    /// selected.
    fn rebuild(
        &mut self,
        tabs: &TabSets,
        active: usize,
        list_active: usize,
        window: &mut Window,
        cx: &mut Context<MailApp>,
    ) {
        let TabSets { readers: threads, views, .. } = tabs;
        // A rebuild replaces the groups, so a zoom has to be put back on the new one.
        let zoomed = self.zoomed(cx);
        if zoomed.is_some() {
            self.area.update(cx, |area, cx| area.set_zoomed_out(window, cx));
        }
        // One panel per open thread, reusing the entity that already draws one so a tab
        // keeps its identity across a rebuild that only changed its neighbours.
        let mut pool = std::mem::take(&mut self.readers);
        let mut readers = Vec::with_capacity(threads.len());
        for thread in threads {
            let reusing = pool.iter().position(|panel| panel.read(cx).thread == Some(*thread));
            readers.push(match reusing {
                Some(ix) => pool.remove(ix),
                None => {
                    let app = self.app.clone();
                    let app_entity = cx.entity();
                    cx.new(|cx| ReaderPanel {
                        app,
                        thread: Some(*thread),
                        standalone: false,
                        focus: cx.focus_handle(),
                        _app_sub: watch_app(&app_entity, cx),
                    })
                }
            });
        }
        self.readers = readers;
        // One panel per open view, the same way: a tab keeps its identity across a rebuild
        // that only changed its neighbours, and a preview that took over another view's slot
        // gets a panel of its own.
        let mut pool = std::mem::take(&mut self.lists);
        let mut lists = Vec::with_capacity(views.len());
        for query in views {
            let reusing = pool.iter().position(|panel| &panel.read(cx).query == query);
            lists.push(match reusing {
                Some(ix) => pool.remove(ix),
                None => {
                    let app = self.app.clone();
                    let measured = self.measured.clone();
                    let app_entity = cx.entity();
                    cx.new(|cx| ListPanel {
                        app,
                        query: query.clone(),
                        measured,
                        focus: cx.focus_handle(),
                        _app_sub: watch_app(&app_entity, cx),
                    })
                }
            });
        }
        self.lists = lists;
        self.blank.update(cx, |panel, _| panel.standalone = self.readers.is_empty());

        let list = list_layout(self, list_active, cx);
        let reader = self.reader_layout(active, cx);
        let orientation = self.orientation;
        let size = Some(px(self.sizes[orientation.ix()]));

        self.area.update(cx, |area, cx| {
            let split = match orientation {
                Orientation::SideBySide => DockLayout::h_split(),
                Orientation::Stacked => DockLayout::v_split(),
            };
            let split = split.child(list, size).child(reader, None);
            area.set_center(split, window, cx);
        });
        self.active = (active < self.readers.len()).then_some(active);
        self.list_active = list_active.min(self.lists.len().saturating_sub(1));
        self.built = Some(Built {
            orientation,
            threads: threads.to_vec(),
            views: views.clone(),
        });
        if let Some(zoom) = zoomed {
            self.zoom_in(zoom, window, cx);
        }
    }

    /// The reader group: the open tabs in order, then the panel that stands in when there
    /// is no tab to show.
    fn reader_layout(&self, active: usize, cx: &App) -> DockLayout {
        let mut group = DockLayout::tabs();
        for panel in &self.readers {
            group = group.panel_view(panel_handle(panel.clone()), cx);
        }
        let active = active.min(self.readers.len().saturating_sub(1));
        group.panel_view(panel_handle(self.blank.clone()), cx).active_index(active)
    }

    /// The share each orientation starts its list at, before anything has measured it:
    /// the classic 42% side by side, 45% of the height when stacked.
    fn defaults(&self, sidebar: f32, window: &Window) -> [f32; 2] {
        let viewport = window.viewport_size();
        let width = available_width(f32::from(viewport.width), sidebar);
        [crate::app::row::list_width(width), available_height(f32::from(viewport.height)) * 0.45]
    }
}


/// A tab dragged in a strip: the group's tabs in their new order, and the one it left open.
pub(super) struct DraggedTabs<K> {
    /// The tabs, left to right: threads in the reader strip, views in the list one.
    pub order: Vec<K>,
    /// The tab the group now shows, when the drag moved the open tab or opened a new one.
    pub active: Option<K>,
}

/// What the two strips were left in after a tab was dragged in one of them.
pub(super) struct Dragged {
    /// The reader group, when its order moved.
    pub readers: Option<DraggedTabs<u32>>,
    /// The list group, when its order moved.
    pub lists: Option<DraggedTabs<Query>>,
}

/// The tab sets the dock should be holding: what each group's tabs are and which one it is
/// showing. The two panes are two strips, so each carries its own.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct TabSets {
    /// Threads in tab order.
    pub readers: Vec<u32>,
    /// The reader tab to display.
    pub reader: Option<u32>,
    /// The views in tab order.
    pub views: Vec<Query>,
    /// The list tab to display, as an index into `views`.
    pub list: usize,
}

/// A tab group that can fill the dock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Zoom {
    List,
    Reader,
}

impl Zoom {
    pub(super) fn name(self) -> &'static str {
        match self {
            Zoom::List => "list",
            Zoom::Reader => "reader",
        }
    }
}

/// The list group: one panel per open view, in tab order, with the active tab displayed.
fn list_layout(dock: &Dock, active: usize, cx: &App) -> DockLayout {
    let mut group = DockLayout::tabs();
    for panel in &dock.lists {
        group = group.panel_view(panel_handle(panel.clone()), cx);
    }
    group.active_index(active.min(dock.lists.len().saturating_sub(1)))
}

/// Repaint `cx`'s entity whenever the app changes. A panel is its own entity, so nothing
/// re-renders it when `MailApp` is notified: the pre-dock layout got that for free by building
/// every pane inside `MailApp`'s own render.
fn watch_app<T: 'static>(app: &Entity<MailApp>, cx: &mut Context<T>) -> Subscription {
    cx.observe(app, |_, _, cx| cx.notify())
}

/// Each orientation's default list size, for a dock that has a window but no context yet.
fn default_sizes(window: &Window) -> [f32; 2] {
    let viewport = window.viewport_size();
    [
        crate::app::row::list_width(available_width(f32::from(viewport.width), SIDEBAR_W)),
        available_height(f32::from(viewport.height)) * 0.45,
    ]
}

/// One list tab: today's `render_list` for the view that tab shows, filling the dock's
/// list slot, with the tab's own name drawn in the strip above it.
pub(super) struct ListPanel {
    app: WeakEntity<MailApp>,
    /// The view this tab shows, which is what identifies it in the strip.
    query: Query,
    /// The list slot as last painted, so rows size themselves from the real pane. Shared
    /// with the dock, which answers the app's questions about the pane's size.
    measured: Rc<Cell<(f32, f32)>>,
    focus: FocusHandle,
    _app_sub: Subscription,
}

impl BasePanel for ListPanel {
    fn panel_name(&self) -> &'static str {
        "mail.list"
    }
    fn closable(&self, _: &App) -> bool {
        false
    }
    fn zoomable(&self, _: &App) -> bool {
        true
    }
}

impl Panel for ListPanel {
    /// The tab itself: the location's name, italic while it is the preview, and our own
    /// close button.
    fn title(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (Some(query), Some(app)) = (Some(&self.query), self.app.upgrade()) else {
            return div().into_any_element();
        };
        app.update(cx, |app, cx| app.list_tab(query.clone(), cx))
    }

    /// The strip only when there is more than one view open: a group of one panel would
    /// otherwise draw a title bar repeating the list's own header.
    fn title_bar(&self, cx: &App) -> bool {
        self.app.read_with(cx, |app, _| app.list_tabs.len() > 1).unwrap_or(false)
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }
    fn zoom_control(&self, _: &App) -> Option<PanelControl> {
        None
    }
}

impl EventEmitter<PanelEvent> for ListPanel {}

impl Focusable for ListPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ListPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let measured = self.measured.clone();
        let Some(app) = self.app.upgrade() else { return div() };
        let list = app.update(cx, |app, cx| {
            let before = app.enter_slot(Slot::Tab);
            app.list_w = app.list_width();
            let list = app.render_list(cx);
            app.leave_slot(before);
            list
        });
        div().size_full().min_w_0().min_h_0().on_prepaint(move |bounds, _, _| {
            measured.set((f32::from(bounds.size.width), f32::from(bounds.size.height)))
        }).child(list)
    }
}

/// One reader tab: the reader as it stands for a thread, with the tab's own title.
pub(super) struct ReaderPanel {
    app: WeakEntity<MailApp>,
    /// The thread this tab shows, or `None` for the panel that stands in when no thread
    /// has a tab.
    thread: Option<u32>,
    /// Set on the stand-in: it draws only while it is the reader group's one visible
    /// panel, which is what keeps an empty inbox free of a tab bar.
    standalone: bool,
    focus: FocusHandle,
    _app_sub: Subscription,
}

impl BasePanel for ReaderPanel {
    fn panel_name(&self) -> &'static str {
        "mail.reader"
    }
    fn visible(&self, _: &App) -> bool {
        self.thread.is_some() || self.standalone
    }
    fn closable(&self, _: &App) -> bool {
        false
    }
    fn zoomable(&self, _: &App) -> bool {
        true
    }
}

impl Panel for ReaderPanel {
    /// The tab itself: monogram, italic-while-preview title and our own close button.
    fn title(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (Some(thread), Some(app)) = (self.thread, self.app.upgrade()) else {
            // Only reachable if a style draws a tab for the stand-in panel: its title is
            // what an empty tab strip would show. Observed so a test can prove it is gone.
            return div().id("reader-tab-strip").test_support().into_any_element();
        };
        app.update(cx, |app, cx| app.reader_tab(thread, cx))
    }

    /// The stand-in draws no title bar when the reader has no tabs.
    fn title_bar(&self, _: &App) -> bool {
        self.thread.is_some()
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }

    fn zoom_control(&self, _: &App) -> Option<PanelControl> {
        None
    }
}

impl EventEmitter<PanelEvent> for ReaderPanel {}

impl Focusable for ReaderPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ReaderPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(app) = self.app.upgrade() else { return div() };
        let content = app.update(cx, |app, cx| reader_content(app, Slot::Tab, cx));
        div().size_full().min_w_0().min_h_0().flex().flex_col().child(content)
    }
}

/// The reader `slot` draws: the composer when it is open and this is the reader it replies
/// from — the Triage takeover's while Triage mode is open — otherwise the opened thread.
pub(super) fn reader_content(app: &mut MailApp, slot: Slot, cx: &mut Context<MailApp>) -> AnyElement {
    let before = app.enter_slot(slot);
    let triage = slot == Slot::Triage;
    // The email views this pass draws stay; the others (another tab's, or all when the composer
    // replaces the reader) are hidden.
    let webviews = app.webviews.clone();
    webviews.begin_pass(triage);
    let content = match &app.compose {
        Some(compose) if triage == app.triage_open() => {
            div().flex_1().min_w_0().min_h_0().child(compose.clone()).into_any_element()
        }
        _ => app.render_reader(cx),
    };
    webviews.end_pass(triage);
    app.leave_slot(before);
    content
}