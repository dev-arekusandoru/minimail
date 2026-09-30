//! Pane geometry: the sidebar, the inbox list and the reader, laid out with gpui-kit's
//! resizable panel groups.
//!
//! The sidebar sits in an outer horizontal group; the list and the reader share a nested
//! group whose axis follows the [`Orientation`] — `SideBySide` puts the list left of the
//! reader, `Stacked` puts it above. Every group owns a [`ResizableState`], and each
//! orientation has its own, so each remembers its list size. Minimums are per-panel size
//! ranges; the flex layout copes when a window is too small for all of them.

use super::*;

use gpui_kit::component::resizable::{ResizableState, h_resizable, resizable_panel, v_resizable};

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
/// Width of the double-click target centred on each divider.
pub const DIVIDER_HIT: f32 = 7.;

/// The sidebar's default width.
pub const SIDEBAR_W: f32 = 148.;
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

    /// The size the list starts from, for `available` room along the stacking axis: the
    /// classic 42% list, and 45% of the height when the panes are stacked.
    fn default_list(self, available: f32) -> f32 {
        match self {
            Self::SideBySide => crate::app::row::list_width(available),
            Self::Stacked => available * 0.45,
        }
    }

    /// `(list, reader)` minimums along the stacking axis.
    fn minimums(self) -> (f32, f32) {
        match self {
            Self::SideBySide => (MIN_LIST_W, MIN_READER_W),
            Self::Stacked => (MIN_LIST_H, MIN_READER_H),
        }
    }
}

/// The pane layout: orientation, sidebar visibility and one [`ResizableState`] per group.
/// Hiding the sidebar keeps its state, so the width comes back when it is shown again.
pub struct Panes {
    orientation: Orientation,
    sidebar_visible: bool,
    /// Sidebar | content.
    main: Entity<ResizableState>,
    /// List | reader when side by side.
    side_by_side: Entity<ResizableState>,
    /// List / reader when stacked.
    stacked: Entity<ResizableState>,
}

impl Panes {
    pub fn new(cx: &mut App) -> Self {
        Self {
            orientation: Orientation::default(),
            sidebar_visible: true,
            main: cx.new(|_| ResizableState::default()),
            side_by_side: cx.new(|_| ResizableState::default()),
            stacked: cx.new(|_| ResizableState::default()),
        }
    }

    pub fn orientation(&self) -> Orientation {
        self.orientation
    }

    pub fn sidebar_visible(&self) -> bool {
        self.sidebar_visible
    }

    /// The state of the list / reader group in the current orientation.
    fn list_state(&self) -> &Entity<ResizableState> {
        match self.orientation {
            Orientation::SideBySide => &self.side_by_side,
            Orientation::Stacked => &self.stacked,
        }
    }

    /// Every group state, so the app can repaint when one of them moves.
    pub fn states(&self) -> [&Entity<ResizableState>; 3] {
        [&self.main, &self.side_by_side, &self.stacked]
    }

    /// The list pane as last laid out, along the stacking axis.
    fn measured_list(&self, cx: &App) -> Option<f32> {
        let state = self.list_state().read(cx);
        (state.container_size() > px(0.)).then(|| state.sizes().first().map_or(0., |s| f32::from(*s)))
    }

    /// The sidebar as last laid out (or as it was when hidden).
    fn measured_sidebar(&self, cx: &App) -> f32 {
        let state = self.main.read(cx);
        state.sizes().first().filter(|_| state.container_size() > px(0.)).map_or(SIDEBAR_W, |s| f32::from(*s))
    }
}

impl MailApp {
    /// How the list and the reader are stacked.
    pub fn orientation(&self) -> Orientation {
        self.panes.orientation()
    }

    /// Size of the list pane as last laid out, in pixels along the stacking axis.
    pub fn list_pane_size(&self, cx: &App) -> f32 {
        self.panes.measured_list(cx).unwrap_or(0.)
    }

    /// Whether the sidebar is shown.
    pub fn sidebar_visible(&self) -> bool {
        self.panes.sidebar_visible()
    }

    /// Current sidebar width in pixels; a hidden sidebar keeps the width it will come back with.
    pub fn sidebar_width(&self, cx: &App) -> f32 {
        self.panes.measured_sidebar(cx)
    }

    /// Width the list rows get: the list pane side by side, the whole pane region when stacked.
    pub(super) fn list_width(&self, viewport_w: f32, cx: &App) -> f32 {
        let sidebar = if self.panes.sidebar_visible { self.panes.measured_sidebar(cx) } else { 0. };
        let available = available_width(viewport_w, sidebar);
        match self.panes.orientation {
            Orientation::Stacked => available,
            Orientation::SideBySide => self.panes.measured_list(cx).unwrap_or_else(|| Orientation::SideBySide.default_list(available)),
        }
    }

    pub(super) fn grow_list_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.nudge_list_pane(STEP, window, cx);
    }

    pub(super) fn shrink_list_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.nudge_list_pane(-STEP, window, cx);
    }

    /// Move the divider along the stacking axis; the panel ranges keep both panes usable.
    pub(super) fn nudge_list_pane(&mut self, delta: f32, window: &mut Window, cx: &mut Context<Self>) {
        let Some(size) = self.panes.measured_list(cx) else { return };
        let state = self.panes.list_state().clone();
        state.update(cx, |state, cx| state.resize_panel(0, px(size + delta), window, cx));
    }

    /// Back to the default sizes for both orientations.
    pub(super) fn reset_panes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for (orientation, state) in
            [(Orientation::SideBySide, &self.panes.side_by_side), (Orientation::Stacked, &self.panes.stacked)]
        {
            let container = f32::from(state.read(cx).container_size());
            if container > 0. {
                let size = orientation.default_list(container);
                state.clone().update(cx, |state, cx| state.resize_panel(0, px(size), window, cx));
            }
        }
        self.show_toast("Pane sizes reset".into(), window, cx);
    }

    /// Back to the default sidebar width.
    pub(super) fn reset_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let main = self.panes.main.clone();
        main.update(cx, |state, cx| state.resize_panel(0, px(SIDEBAR_W), window, cx));
    }

    /// Stack the panes the other way round: list left, or list on top.
    pub(super) fn toggle_pane_layout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let next = self.panes.orientation.toggled();
        self.set_pane_layout(next, window, cx);
    }

    pub(super) fn set_pane_layout(&mut self, orientation: Orientation, window: &mut Window, cx: &mut Context<Self>) {
        self.panes.orientation = orientation;
        self.show_toast(format!("Layout: {}", orientation.label()), window, cx);
    }

    /// Hide or show the sidebar. Its state outlives the panel, so the width is kept.
    pub(super) fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.panes.sidebar_visible = !self.panes.sidebar_visible;
        cx.notify();
    }

    /// The invisible band over a divider that turns a double-click into a reset. The resize
    /// handle underneath keeps the drag, the cursor and the highlight.
    fn divider_target(
        &self,
        id: &'static str,
        cx: &Context<Self>,
        place: impl FnOnce(Div) -> Div,
        reset: fn(&mut Self, &mut Window, &mut Context<Self>),
    ) -> impl IntoElement {
        place(div().absolute()).id(id).test_support().on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
            if ev.click_count() == 2 {
                reset(this, window, cx);
            }
        }))
    }

    /// The three panes: sidebar | list | reader in nested resizable groups, with the
    /// double-click targets over their dividers. `list` is `None` during a triage session,
    /// which leaves the reader alone.
    pub(super) fn render_panes(
        &self,
        list: Option<AnyElement>,
        reader: AnyElement,
        window: &Window,
        cx: &Context<Self>,
    ) -> AnyElement {
        let t = theme::active(cx);
        let orientation = self.panes.orientation;
        let stacked = orientation == Orientation::Stacked;
        let viewport = window.viewport_size();
        let (vw, vh) = (f32::from(viewport.width), f32::from(viewport.height));
        let shown = self.panes.sidebar_visible;
        let sidebar_w = self.panes.measured_sidebar(cx);
        let reader = div().size_full().min_w_0().min_h_0().flex().flex_col().child(reader);

        let content = match list {
            None => reader.into_any_element(),
            Some(list) => {
                let available = if stacked { available_height(vh) } else { available_width(vw, if shown { sidebar_w } else { 0. }) };
                let (min_list, min_reader) = orientation.minimums();
                let group = if stacked { v_resizable("panes-stacked") } else { h_resizable("panes-side-by-side") };
                group
                    .with_state(self.panes.list_state())
                    .child(resizable_panel().size(px(orientation.default_list(available))).size_range(px(min_list)..Pixels::MAX).child(list))
                    .child(resizable_panel().size_range(px(min_reader)..Pixels::MAX).child(reader))
                    .into_any_element()
            }
        };

        let panes = if shown {
            h_resizable("mail-main")
                .with_state(&self.panes.main)
                .child(
                    resizable_panel()
                        .size(px(SIDEBAR_W))
                        .size_range(px(MIN_SIDEBAR_W)..px(MAX_SIDEBAR_W))
                        .flex_none()
                        .bg(t.sidebar)
                        .child(div().size_full().flex().flex_col().child(self.render_sidebar(cx))),
                )
                .child(resizable_panel().size_range(px(MIN_LIST_W + MIN_READER_W)..Pixels::MAX).child(content))
                .into_any_element()
        } else {
            content
        };

        // Double-click targets sit over the dividers, at the boundaries the groups last laid out.
        let body_w = f32::from(self.panes.main.read(cx).container_size());
        let mut targets = Vec::new();
        if body_w > 0. {
            let half = DIVIDER_HIT / 2.;
            if shown {
                let place = |d: Div| d.top_0().bottom_0().left(px(sidebar_w - half)).w(px(DIVIDER_HIT));
                targets.push(self.divider_target("sidebar-divider", cx, place, Self::reset_sidebar).into_any_element());
            }
            if let (Some(list), false) = (self.panes.measured_list(cx), self.in_session() || self.session_end.is_some()) {
                let origin = if shown { sidebar_w } else { 0. };
                let place = move |d: Div| {
                    if stacked {
                        d.left(px(origin)).right_0().top(px(list - half)).h(px(DIVIDER_HIT))
                    } else {
                        d.top_0().bottom_0().left(px(origin + list - half)).w(px(DIVIDER_HIT))
                    }
                };
                targets.push(self.divider_target("pane-divider", cx, place, Self::reset_panes).into_any_element());
            }
        }
        div().relative().flex_1().min_h_0().child(panes).children(targets).into_any_element()
    }
}
