//! Pane layout: the sidebar, the inbox list and the reader.
//!
//! The three panes are dock panels now — the sidebar in the left dock, the list and the
//! reader as two tab groups in the centre (see [`super::dock`]). What stays here is what
//! the dock cannot hold for us: which way the centre splits, whether the sidebar is on
//! screen, and the size each orientation remembers.
use super::dock::Dock;
use super::*;

/// Smallest list pane that still shows a sender, a subject and an icon cluster.
pub const MIN_LIST_W: f32 = 320.;
/// Smallest reader pane: a message header plus a readable measure of body.
pub const MIN_READER_W: f32 = 380.;
/// Smallest list pane in `Stacked` (height): a header and a few rows.
pub const MIN_LIST_H: f32 = 180.;
/// Smallest reader pane in `Stacked` (height).
pub const MIN_READER_H: f32 = 220.;
/// How much a grow / shrink action moves the divider.
pub const STEP: f32 = 40.;
/// Width of the double-click target centred on the sidebar's edge.
pub const DIVIDER_HIT: f32 = 7.;

/// The sidebar's default width.
pub const SIDEBAR_W: f32 = 190.;
/// Sidebar drag limits.
pub const MIN_SIDEBAR_W: f32 = 120.;
pub const MAX_SIDEBAR_W: f32 = 320.;
/// The titlebar and the hint bar, which the stacked panes do not get. Only used to size the
/// stacked list before the first layout has measured the real container.
const CHROME_H: f32 = 78.;

/// Room the panes share on the width axis.
pub fn available_width(viewport_w: f32, sidebar_w: f32) -> f32 {
    (viewport_w - sidebar_w).max(MIN_LIST_W)
}

/// Room the panes share on the height axis.
pub fn available_height(viewport_h: f32) -> f32 {
    (viewport_h - CHROME_H).max(MIN_LIST_H)
}

/// How the list and the reader are stacked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Orientation {
    /// List on the left, reader on the right (the default).
    #[default]
    SideBySide,
    /// List on top, reader below.
    Stacked,
}

impl Orientation {
    pub fn toggled(self) -> Self {
        match self {
            Self::SideBySide => Self::Stacked,
            Self::Stacked => Self::SideBySide,
        }
    }

    /// Label of the *other* orientation, i.e. what the toggle switches to.
    pub fn next_label(self) -> &'static str {
        self.toggled().label()
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::SideBySide => "Side by side",
            Self::Stacked => "Stacked",
        }
    }

    /// Index of this orientation in the dock's remembered-size pair.
    pub(crate) fn ix(self) -> usize {
        match self {
            Self::SideBySide => 0,
            Self::Stacked => 1,
        }
    }
}

/// The pane layout: the dock that draws the three panes, the way its centre splits and
/// whether the sidebar is on screen.
pub struct Panes {
    dock: Dock,
}

impl Panes {
    pub(super) fn new(app: &WeakEntity<MailApp>, window: &mut Window, cx: &mut Context<MailApp>) -> Self {
        Self { dock: Dock::new(app, window, cx) }
    }

    pub fn orientation(&self) -> Orientation {
        self.dock.orientation()
    }

    /// Which way the centre splits from now on; the next sync re-lays it.
    pub(super) fn set_orientation(&mut self, orientation: Orientation) {
        self.dock.set_orientation(orientation);
    }

    /// Which threads the reader group should be showing: `None` while a session owns the
    /// reader, which shows it alone with no tabs.
    fn reader_threads(&self, app: &MailApp) -> Option<Vec<u32>> {
        if app.in_session() || app.session_end.is_some() {
            return None;
        }
        Some(app.tabs.tabs().iter().map(|tab| tab.thread).collect())
    }

    pub fn sidebar_visible(&self) -> bool {
        self.dock.sidebar_open()
    }

    pub(super) fn dock_mut(&mut self) -> &mut Dock {
        &mut self.dock
    }

    pub(super) fn dock_ref(&self) -> &Dock {
        &self.dock
    }
}

/// The thread of the active tab, which is the one reader panel to display.
fn active_thread(app: &MailApp) -> Option<u32> {
    app.tabs.active().map(|tab| tab.thread)
}

impl MailApp {
    /// How the list and the reader are stacked.
    pub fn orientation(&self) -> Orientation {
        self.panes.orientation()
    }

    /// Size of the list pane as last laid out, in pixels along the stacking axis.
    pub fn list_pane_size(&self, cx: &App) -> f32 {
        self.panes.dock_ref().list_pane_size(cx)
    }

    /// Whether the sidebar is shown.
    pub fn sidebar_visible(&self) -> bool {
        self.panes.sidebar_visible()
    }

    /// Current sidebar width in pixels; a hidden sidebar keeps the width it will come back with.
    pub fn sidebar_width(&self, cx: &App) -> f32 {
        self.panes.dock_ref().sidebar_width(cx)
    }

    /// Width the list rows get: the list pane side by side, the whole pane region when stacked.
    pub(super) fn list_width(&self) -> f32 {
        self.panes.dock_ref().list_width()
    }

    pub(super) fn grow_list_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.nudge_list_pane(STEP, window, cx);
    }

    pub(super) fn shrink_list_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.nudge_list_pane(-STEP, window, cx);
    }

    /// Move the divider along the stacking axis. The dock has no centre divider of its own
    /// to move, so this re-lays the centre around the new size; the range that keeps both
    /// panes usable is applied here, since the dock's slots carry no minimum.
    pub(super) fn nudge_list_pane(&mut self, delta: f32, window: &mut Window, cx: &mut Context<Self>) {
        let size = self.list_pane_size(cx);
        if size <= 0. {
            return;
        }
        let threads = self.panes.reader_threads(self);
        let active = active_thread(self);
        self.panes.dock_mut().place_list(size + delta, threads.as_deref(), active, window, cx);
    }

    /// Back to the default sizes for both orientations.
    pub(super) fn reset_panes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let threads = self.panes.reader_threads(self);
        let active = active_thread(self);
        self.panes.dock_mut().forget_sizes(threads.as_deref(), active, window, cx);
        self.show_toast("Pane sizes reset".into(), window, cx);
    }

    /// Back to the default sidebar width.
    pub(super) fn reset_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.panes.dock_mut().reset_sidebar(window, cx);
    }

    /// Stack the panes the other way round: list left, or list on top.
    pub(super) fn toggle_pane_layout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let next = self.panes.orientation().toggled();
        self.set_pane_layout(next, window, cx);
    }

    pub(super) fn set_pane_layout(&mut self, orientation: Orientation, window: &mut Window, cx: &mut Context<Self>) {
        if orientation == self.panes.orientation() {
            return;
        }
        self.panes.dock_mut().set_orientation(orientation);
        let threads = self.panes.reader_threads(self);
        let active = active_thread(self);
        self.panes.dock_mut().rebuild_now(threads.as_deref(), active, window, cx);
        self.show_toast(format!("Layout: {}", orientation.label()), window, cx);
    }

    /// Hide or show the sidebar. The dock keeps its size either way, so the width comes
    /// back with it.
    pub(super) fn toggle_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.panes.dock_mut().toggle_sidebar(window, cx);
        cx.notify();
    }

    /// Keep the dock's panel set in step with the app's: the reader group's tabs, the
    /// active one, and the orientation. A tab dragged in the strip is a dock edit the model
    /// never saw, so the order it was left in goes back into `Tabs`.
    pub(super) fn sync_dock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let threads = self.panes.reader_threads(self);
        let active = active_thread(self);
        let dragged = self.panes.dock_mut().sync(threads.as_deref(), active, window, cx);
        if let Some(dragged) = dragged {
            self.tabs.reorder_to(&dragged.order);
            if let Some(ix) = dragged.active.and_then(|thread| self.tabs.index_of(thread)) {
                self.tabs.activate(ix);
            }
        }
    }

    /// The band over the sidebar's edge that turns a double-click back to its default
    /// width. The dock's own resize handle underneath keeps the drag, the cursor and the
    /// highlight.
    fn sidebar_divider_target(&self, sidebar_w: f32, cx: &Context<Self>) -> impl IntoElement {
        let half = DIVIDER_HIT / 2.;
        div()
            .absolute()
            .id("sidebar-divider")
            .test_support()
            .top_0()
            .bottom_0()
            .left(px(sidebar_w - half))
            .w(px(DIVIDER_HIT))
            .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                if ev.click_count() == 2 {
                    this.reset_sidebar(window, cx);
                }
            }))
    }

    /// The dock, in the slot the panes used to be laid out by hand: under the titlebar and
    /// above the hint bar.
    pub(super) fn render_panes(&self, cx: &Context<Self>) -> AnyElement {
        let sidebar_w = self.panes.dock_ref().sidebar_width(cx);
        div()
            .relative()
            .flex_1()
            .min_h_0()
            .child(self.panes.dock_ref().view())
            .child(self.sidebar_divider_target(sidebar_w, cx))
            .into_any_element()
    }
}