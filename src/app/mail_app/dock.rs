//! The three panes as dock panels: the sidebar in the left dock, the list and the reader
//! as two tab groups in the centre.
//!
//! Every panel here is a thin proxy. It holds a [`WeakEntity<MailApp>`] plus the identity
//! it draws — a thread, or none — and calls straight back into the app's own render
//! helpers. The mailbox, the cursor, the reader tabs and every piece of per-thread reader
//! state stay in [`MailApp`], so the dock only decides where those pieces sit and the
//! existing render code survives the move unchanged.
//!
//! The reader group mirrors [`crate::tabs::Tabs`]: one panel per open thread, in tab
//! order, with the active tab displayed. `None` threads means a session owns the reader,
//! which shows it alone and draws no tab bar. Either way a panel standing for "nothing is
//! open" keeps the group alive, so the group never disappears out from under the layout.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::base::ElementExt as _;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::base::ResizeHandleContext;
use gpui_kit::component::dock::{
    AnyDrag, BasePanel, BasePanelView, DockArea, DockAreaRenderer, DockContext, DockEvent, DockLayout, DockPlacement,
    DockSkin, DropIndicator, NodeId, PaneRef, Panel, PanelControl, PanelEvent, PanelId, PanelState, PanelStyle,
    TabGroupContext, TabGroupRenderer, panel_handle,
};

/// A panel that takes a message dropped on its group.
const READER_PANEL: &str = "mail.reader";

/// The dock's look: the kit's skin for everything, minus the split preview.
///
/// Base draws that preview for any drag over any group, and a message drop never splits a
/// pane here — the list and the sidebar do not take mail at all. So the dock draws no split
/// preview anywhere and the reader group carries its own mark instead (see [`MailGroups`]).
struct MailSkin {
    inner: Rc<DockSkin>,
}

impl DockAreaRenderer for MailSkin {
    // Everything but the groups themselves is the kit's chrome unchanged — including the
    // sidebar's resize handle and collapse affordance, which the default hooks do not draw.
    fn frame(&self, window: &mut Window, cx: &mut App) -> Stateful<Div> {
        self.inner.frame(window, cx)
    }

    fn split_frame(&self, node: NodeId, axis: Axis, window: &mut Window, cx: &mut App) -> Stateful<Div> {
        self.inner.split_frame(node, axis, window, cx)
    }

    fn center_frame(&self, window: &mut Window, cx: &mut App) -> Stateful<Div> {
        self.inner.center_frame(window, cx)
    }

    fn render_split_handle(
        &self,
        handle: &ResizeHandleContext,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        self.inner.render_split_handle(handle, window, cx)
    }

    fn render_dock(&self, dock: &DockContext, content: AnyElement, window: &mut Window, cx: &mut App) -> AnyElement {
        self.inner.render_dock(dock, content, window, cx)
    }

    fn build_placeholder(
        &self,
        state: &PanelState,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Arc<dyn BasePanelView>> {
        self.inner.build_placeholder(state, window, cx)
    }

    /// The only hook that is not a plain forward: the kit's skin, with the two group-side
    /// drop hooks adjusted (see [`MailGroups`]).
    fn tab_group_renderer(&self) -> Rc<dyn TabGroupRenderer> {
        Rc::new(MailGroups { inner: self.inner.tab_group_renderer() })
    }
}

/// One group's chrome, delegated to the kit's skin except for the two hooks that decide how
/// a message drag is advertised.
struct MailGroups {
    inner: Rc<dyn TabGroupRenderer>,
}

impl TabGroupRenderer for MailGroups {
    fn frame(&self, group: &TabGroupContext, window: &mut Window, cx: &mut App) -> Stateful<Div> {
        self.inner.frame(group, window, cx)
    }

    /// The reader group lights up for a drag, the way the kit's own drop targets do. Every
    /// other group stays bare, so nothing invites a drop that would be ignored.
    fn content_frame(&self, group: &TabGroupContext, window: &mut Window, cx: &mut App) -> Stateful<Div> {
        let frame = self.inner.content_frame(group, window, cx);
        if !takes_mail(group, cx) {
            return frame;
        }
        frame.drag_over::<AnyDrag>(|this, _, _, cx| this.bg(cx.theme().tokens.drop_target))
    }

    fn render_tab_bar(&self, group: &TabGroupContext, window: &mut Window, cx: &mut App) -> AnyElement {
        self.inner.render_tab_bar(group, window, cx)
    }

    fn render_active_panel(
        &self,
        panel: AnyView,
        group: &TabGroupContext,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        self.inner.render_active_panel(panel, group, window, cx)
    }

    /// Never. What base offers here is a split placement, which is a promise this app does
    /// not keep: mail opens a tab, and only on the reader.
    fn render_drop_indicator(&self, _: DropIndicator, _: &mut Window, _: &mut App) -> Option<AnyElement> {
        None
    }

    fn render_empty(&self, group: &TabGroupContext, window: &mut Window, cx: &mut App) -> Option<AnyElement> {
        self.inner.render_empty(group, window, cx)
    }
}

/// Whether this group holds the reader, and so opens what is dropped on it.
fn takes_mail(group: &TabGroupContext, cx: &App) -> bool {
    group.panels().iter().any(|panel| panel.panel_name(cx) == READER_PANEL)
}

use super::panes::{
    MAX_SIDEBAR_W, MIN_LIST_H, MIN_LIST_W, MIN_READER_H, MIN_READER_W, MIN_SIDEBAR_W, Orientation, SIDEBAR_W,
    available_height, available_width,
};
use super::*;

pub(super) struct Dock {
    app: WeakEntity<MailApp>,
    area: Entity<DockArea>,
    sidebar: Entity<SidebarPanel>,
    list: Entity<ListPanel>,
    /// The list slot as last painted. Shared with the list panel, which measures it, so
    /// the app can read the pane's real width without leasing a panel that is mid-render.
    measured: Rc<Cell<(f32, f32)>>,
    /// The reader shown when no thread has a tab — and, during a session, the reader a
    /// session owns. It lives in the reader group at all times so the group is never
    /// empty, and draws only when no thread tab is there to draw instead.
    blank: Entity<ReaderPanel>,
    /// One panel per open thread, in [`crate::tabs::Tabs`] order.
    readers: Vec<Entity<ReaderPanel>>,
    /// Which way the centre splits.
    orientation: Orientation,
    /// Whether the sidebar is on screen. The dock owns the truth; this mirrors it for the
    /// titlebar, which asks without a context.
    sidebar_open: bool,
    /// The centre split's list slot, remembered per orientation so each keeps its own.
    sizes: [f32; 2],
    /// Width the list rows get before the slot has ever been painted.
    list_w_default: Cell<f32>,
    /// What the dock was last built for, so an ordinary frame builds nothing.
    built: Option<Built>,
    /// Which reader panel is displayed, as an index into `readers`.
    active: Option<usize>,
    _subs: Vec<Subscription>,
}

/// The panel set the dock currently holds. A change to any field is the only reason to
/// rebuild the centre; a change of active tab alone is a `select_panel`.
#[derive(PartialEq, Eq, Clone)]
struct Built {
    orientation: Orientation,
    /// Threads in tab order, or `None` while a session owns the reader.
    threads: Option<Vec<u32>>,
}

impl Dock {
    pub(super) fn new(app: &WeakEntity<MailApp>, window: &mut Window, cx: &mut Context<MailApp>) -> Self {
        let app_entity = cx.entity();
        // The kit builds its skin inside the area's constructor, because the skin needs the
        // area's own weak handle. Ours wraps that skin and is installed the same way.
        let mut inner = None;
        let area = cx.new(|cx| {
            let skin = DockSkin::new(cx);
            inner = Some(skin.clone());
            DockArea::new("mail", None, window, cx).with_renderer(Rc::new(MailSkin { inner: skin }))
        });
        let skin = inner.expect("the skin is built inside the area's constructor");
        // `Auto`, not `TabBar`: under `Auto` a group of one panel draws a plain title
        // only when that panel asks for one, so the sidebar and the list — both
        // `title_bar() == false` — stay bare while a reader group of two or more draws
        // the full tab bar.
        skin.set_panel_style(PanelStyle::Auto, cx);
        // Our own close button lives in the reader tab's title, where it already was.
        skin.set_close_button_visible(false, cx);
        skin.set_toggle_button_visible(false, cx);
        let sidebar = cx.new(|cx| SidebarPanel {
            app: app.clone(),
            focus: cx.focus_handle(),
            _app_sub: watch_app(&app_entity, cx),
        });
        let measured = Rc::new(Cell::new((0., 0.)));
        let list = cx.new(|cx| ListPanel {
            app: app.clone(),
            measured: measured.clone(),
            focus: cx.focus_handle(),
            _app_sub: watch_app(&app_entity, cx),
        });
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
            sidebar,
            list,
            measured,
            blank,
            readers: Vec::new(),
            orientation: Orientation::default(),
            sidebar_open: true,
            sizes: default_sizes(window),
            list_w_default: Cell::new(available_width(
                f32::from(window.viewport_size().width),
                SIDEBAR_W,
            )),
            built: None,
            active: None,
            _subs: vec![
                cx.observe(&area, |_, _, cx| cx.notify()),
                cx.subscribe_in(&area, window, |app: &mut MailApp, _, event: &DockEvent, window, cx| {
                    if let DockEvent::DragDrop { item, target } = event {
                        app.drop_on_reader(item, target, window, cx);
                    }
                    cx.notify();
                }),
            ],
        };
        dock.rebuild(Some(&[]), 0, window, cx);
        dock
    }

    /// The dock's element, in the slot the three panes used to be built into.
    pub(super) fn view(&self) -> AnyElement {
        self.area.clone().into_any_element()
    }

    pub(super) fn orientation(&self) -> Orientation {
        self.orientation
    }

    pub(super) fn set_orientation(&mut self, orientation: Orientation) {
        self.orientation = orientation;
    }

    pub(super) fn sidebar_open(&self) -> bool {
        self.sidebar_open
    }

    pub(super) fn sidebar_width(&self, cx: &App) -> f32 {
        f32::from(self.area.read(cx).dock_size(DockPlacement::Left).unwrap_or(px(SIDEBAR_W)))
    }

    /// Sidebar on or off. The dock keeps the size either way, so it comes back as it was.
    pub(super) fn toggle_sidebar(&mut self, window: &mut Window, cx: &mut App) {
        let open = Cell::new(self.sidebar_open);
        self.area.update(cx, |area, cx| {
            area.toggle_dock(DockPlacement::Left, window, cx);
            open.set(area.is_dock_open(DockPlacement::Left));
        });
        self.sidebar_open = open.get();
    }

    /// The sidebar's own limits, which the dock has no size range for.
    pub(super) fn clamp_sidebar(&mut self, window: &mut Window, cx: &mut App) {
        let Some(size) = self.area.read(cx).dock_size(DockPlacement::Left) else { return };
        let size = f32::from(size);
        let clamped = size.clamp(MIN_SIDEBAR_W, MAX_SIDEBAR_W);
        if clamped == size {
            return;
        }
        self.area.update(cx, |area, cx| area.set_dock_size(DockPlacement::Left, px(clamped), window, cx));
    }

    pub(super) fn reset_sidebar(&mut self, window: &mut Window, cx: &mut App) {
        self.area.update(cx, |area, cx| area.set_dock_size(DockPlacement::Left, px(SIDEBAR_W), window, cx));
    }

    /// Bring the dock in line with the app. An ordinary frame builds nothing: only a
    /// change of panel set, orientation or activation reaches the area.
    ///
    /// Returns the reader order the dock was left in when a tab was dragged in the strip: the
    /// group reorders without touching the model, so the app is told and puts `Tabs` the same
    /// way round.
    pub(super) fn sync(
        &mut self,
        threads: Option<&[u32]>,
        active: Option<u32>,
        window: &mut Window,
        cx: &mut Context<MailApp>,
    ) -> Option<DraggedTabs> {
        self.clamp_sidebar(window, cx);
        let wanted = Built { orientation: self.orientation, threads: threads.map(<[u32]>::to_vec) };
        if self.built.as_ref() != Some(&wanted) {
            let ix = active.and_then(|thread| self.reader_ix(thread, cx));
            self.rebuild(threads, ix.unwrap_or(0), window, cx);
            return None;
        }
        let dragged = threads.and_then(|threads| self.dragged_order(threads, cx));
        if let Some(dragged) = &dragged {
            let order = &dragged.order;
            let mut panels: Vec<(usize, Entity<ReaderPanel>)> = std::mem::take(&mut self.readers)
                .into_iter()
                .map(|panel| {
                    let rank = order.iter().position(|thread| Some(*thread) == panel.read(cx).thread);
                    (rank.unwrap_or(usize::MAX), panel)
                })
                .collect();
            panels.sort_by_key(|(rank, _)| *rank);
            self.readers = panels.into_iter().map(|(_, panel)| panel).collect();
            self.built = Some(Built { orientation: self.orientation, threads: Some(order.clone()) });
        }
        // The active tab may have moved with the panels — or the drag may have opened it — so
        // read its slot again.
        let active = dragged.as_ref().and_then(|dragged| dragged.active).or(active);
        let ix = active.and_then(|thread| self.reader_ix(thread, cx));
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
        dragged
    }

    /// Rebuild now, whether or not the panel set changed: the orientation toggle and the
    /// keyboard divider moves all re-lay the centre outright, because the dock has no
    /// other way to move a centre divider.
    pub(super) fn rebuild_now(
        &mut self,
        threads: Option<&[u32]>,
        active: Option<u32>,
        window: &mut Window,
        cx: &mut Context<MailApp>,
    ) {
        self.built = None;
        self.sync(threads, active, window, cx);
    }

    /// Rebuild the centre around `size` for the list slot.
    pub(super) fn place_list(
        &mut self,
        size: f32,
        threads: Option<&[u32]>,
        active: Option<u32>,
        window: &mut Window,
        cx: &mut Context<MailApp>,
    ) {
        let orientation = self.orientation;
        let (min, max) = self.limits(window, cx);
        self.sizes[orientation.ix()] = size.clamp(min, max);
        self.rebuild_now(threads, active, window, cx);
    }

    /// Forget both remembered sizes, so each orientation starts again from its default
    /// share of the room.
    pub(super) fn forget_sizes(
        &mut self,
        threads: Option<&[u32]>,
        active: Option<u32>,
        window: &mut Window,
        cx: &mut Context<MailApp>,
    ) {
        self.sizes = self.defaults(window, cx);
        self.list_w_default.set(available_width(f32::from(window.viewport_size().width), self.sidebar_width(cx)));
        self.rebuild_now(threads, active, window, cx);
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
    fn limits(&self, window: &Window, cx: &App) -> (f32, f32) {
        let viewport = window.viewport_size();
        let (vw, vh) = (f32::from(viewport.width), f32::from(viewport.height));
        match self.orientation {
            Orientation::SideBySide => {
                let sidebar = if self.sidebar_open { self.sidebar_width(cx) } else { 0. };
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

    /// Whether `node` is the centre group holding the reader tabs — the only drop target that
    /// opens mail. The list is a tab group too, and a drop on it means nothing.
    pub(super) fn is_reader_node(&self, node: NodeId, cx: &App) -> bool {
        let area = self.area.read(cx);
        let Some(tree) = area.layout(DockPlacement::Center) else { return false };
        let readers: Vec<PanelId> = self
            .readers
            .iter()
            .chain(std::iter::once(&self.blank))
            .map(|panel| PanelId::from(panel.entity_id()))
            .collect();
        let mut hit = false;
        tree.root().walk(&mut |pane| {
            if pane.id() == node
                && let PaneRef::Tabs { panels, .. } = pane.kind()
            {
                hit = panels.iter().any(|id| readers.contains(id));
            }
        });
        hit
    }

    /// The reader group's tabs as the dock's own tree now holds them, when that order differs
    /// from `threads` — a tab dragged in the strip. The model follows it, so the strip and
    /// `Tabs` can never disagree about which tab is where, or which one is open.
    fn dragged_order(&self, threads: &[u32], cx: &App) -> Option<DraggedTabs> {
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

    /// Install the whole layout: the sidebar in the left dock, and the centre's two tab
    /// groups with the reader's carrying the tabs and whichever of them is active.
    fn rebuild(&mut self, threads: Option<&[u32]>, active: usize, window: &mut Window, cx: &mut Context<MailApp>) {
        let session = threads.is_none();
        let threads = threads.unwrap_or(&[]);
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
        // The stand-in shows only when there is no tab to show it instead.
        self.blank.update(cx, |panel, _| panel.standalone = self.readers.is_empty());

        let sidebar = DockLayout::tabs().panel_view(panel_handle(self.sidebar.clone()), cx);
        let list = DockLayout::tabs().panel_view(panel_handle(self.list.clone()), cx);
        let reader = self.reader_layout(active, cx);
        let orientation = self.orientation;
        let size = Some(px(self.sizes[orientation.ix()]));

        self.area.update(cx, |area, cx| {
            if !area.has_dock(DockPlacement::Left) {
                area.set_dock(DockPlacement::Left, sidebar, window, cx);
                area.set_dock_collapsible(DockPlacement::Left, true, window, cx);
                area.set_dock_size(DockPlacement::Left, px(SIDEBAR_W), window, cx);
            }
            let split = match orientation {
                Orientation::SideBySide => DockLayout::h_split(),
                Orientation::Stacked => DockLayout::v_split(),
            };
            area.set_center(split.child(list, size).child(reader, None), window, cx);
        });
        self.active = (active < self.readers.len()).then_some(active);
        self.built = Some(Built { orientation, threads: (!session).then(|| threads.to_vec()) });
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
    fn defaults(&self, window: &Window, cx: &App) -> [f32; 2] {
        let viewport = window.viewport_size();
        let sidebar = if self.sidebar_open { self.sidebar_width(cx) } else { 0. };
        let width = available_width(f32::from(viewport.width), sidebar);
        [crate::app::row::list_width(width), available_height(f32::from(viewport.height)) * 0.45]
    }
}


/// A tab dragged in the strip: the group's tabs in their new order, and the one it left open.
pub(super) struct DraggedTabs {
    /// Threads, left to right.
    pub order: Vec<u32>,
    /// The thread the group now shows, when the drag moved the open tab or opened a new one.
    pub active: Option<u32>,
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

/// The sidebar: today's `render_sidebar`, in the dock's left region.
pub(super) struct SidebarPanel {
    app: WeakEntity<MailApp>,
    focus: FocusHandle,
    /// Repaints this panel whenever the app's state changes, which is what the panes got for
    /// free while they were built inside `MailApp`'s own render.
    _app_sub: Subscription,
}

impl BasePanel for SidebarPanel {
    fn panel_name(&self) -> &'static str {
        "mail.sidebar"
    }
    fn closable(&self, _: &App) -> bool {
        false
    }
    fn zoomable(&self, _: &App) -> bool {
        false
    }
}

impl Panel for SidebarPanel {
    fn title_bar(&self, _: &App) -> bool {
        false
    }
    fn inner_padding(&self, _: &App) -> bool {
        false
    }
    fn zoom_control(&self, _: &App) -> Option<PanelControl> {
        None
    }
}

impl EventEmitter<PanelEvent> for SidebarPanel {}

impl Focusable for SidebarPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for SidebarPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sidebar_bg = cx.theme().sidebar;
        let Some(app) = self.app.upgrade() else { return div() };
        let menu = app.update(cx, |app, cx| app.render_sidebar(cx));
        div().size_full().min_w_0().min_h_0().flex().flex_col().bg(sidebar_bg).child(menu)
    }
}

/// The message list: today's `render_list`, filling the dock's list slot.
pub(super) struct ListPanel {
    app: WeakEntity<MailApp>,
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
        false
    }
}

impl Panel for ListPanel {
    fn title_bar(&self, _: &App) -> bool {
        false
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
            app.list_w = app.list_width();
            app.render_list(cx)
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
    /// has a tab — and, during a session, for the reader a session owns.
    thread: Option<u32>,
    /// Set on the stand-in: it draws only while it is the reader group's one visible
    /// panel, which is what keeps a session and an empty inbox both free of a tab bar.
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
        false
    }
}

impl Panel for ReaderPanel {
    /// The tab itself: monogram, italic-while-preview title and our own close button.
    fn title(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (Some(thread), Some(app)) = (self.thread, self.app.upgrade()) else {
            return div().into_any_element();
        };
        app.update(cx, |app, cx| app.reader_tab(thread, cx))
    }

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
        let content = app.update(cx, |app, cx| {
            if let Some(compose) = &app.compose {
                return div().flex_1().min_w_0().min_h_0().child(compose.clone()).into_any_element();
            }
            app.render_reader(cx)
        });
        div().size_full().min_w_0().min_h_0().flex().flex_col().child(content)
    }
}