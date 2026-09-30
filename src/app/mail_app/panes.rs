//! Pane geometry: how much room the inbox list and the reader share, and the
//! draggable divider between them.
//!
//! Two orientations stack the same two panes along different axes — `SideBySide`
//! puts the list left of the reader, `Stacked` puts it above. Each remembers its
//! own list size, and both clamp it against the room actually available so that
//! neither pane can be dragged out of existence.

use super::*;

use std::sync::Arc;

use crate::theme::Theme;

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
/// Width of the band around the hairline that catches the pointer.
pub const DIVIDER_HIT: f32 = 7.;

/// The fixed view rail, which is never part of the panes.
const SIDEBAR_W: f32 = 148.;
/// The toolbar and the hint bar, which the stacked panes do not get. The toolbar
/// wraps at narrow widths, so this is an allowance: the reader pane keeps its own
/// minimum in the flex layout and takes the hit if the allowance was too small.
const CHROME_H: f32 = 108.;

/// Room the panes share on the width axis.
pub fn available_width(viewport_w: f32) -> f32 {
    (viewport_w - SIDEBAR_W).max(MIN_LIST_W)
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
}

/// How much room the list pane gets, in pixels along the stacking axis.
#[derive(Clone, Copy, Debug, Default)]
pub struct Panes {
    orientation: Orientation,
    /// Remembered list width for `SideBySide`, if the user ever moved the divider.
    width: Option<f32>,
    /// Remembered list height for `Stacked`.
    height: Option<f32>,
    /// Room the panes had when the width was last remembered, so that a resized
    /// window keeps the proportion instead of pinning a pane to its minimum.
    fitted_width: f32,
    /// The same, for the height of the stacked layout.
    fitted_height: f32,
    /// Where the current divider drag started, mapped onto the stacking axis.
    drag: Option<f32>,
}

impl Panes {
    pub fn orientation(&self) -> Orientation {
        self.orientation
    }

    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// Size of the list pane for `available` room along the current axis, clamped
    /// to what both panes can use. Resizes the window by keeping the proportion.
    pub fn list_size(&mut self, available: f32) -> f32 {
        let (min, max) = Self::limits(self.orientation, available);
        let fitted = self.fitted();
        if available > 0. && (fitted - available).abs() > f32::EPSILON {
            // The window changed: scale what was asked for by how much room is
            // left, then let the clamps have the last word.
            if let Some(size) = self.remembered().map(|size| size * available / fitted.max(1.)) {
                self.remember(size);
            }
            self.set_fitted(available);
        }
        self.remembered().unwrap_or_else(|| Self::default(self.orientation, available)).clamp(min, max)
    }

    /// The list size as it was last laid out, without touching the state.
    pub fn current_size(&self) -> f32 {
        let fitted = self.fitted();
        if fitted <= 0. {
            return 0.;
        }
        let (min, max) = Self::limits(self.orientation, fitted);
        self.remembered().unwrap_or_else(|| Self::default(self.orientation, fitted)).clamp(min, max)
    }

    /// Move the divider by `delta` pixels along the stacking axis.
    pub fn nudge(&mut self, available: f32, delta: f32) {
        let size = self.list_size(available) + delta;
        self.remember(size);
    }

    /// Forget both remembered sizes: the next [`Panes::list_size`] is the default.
    pub fn reset(&mut self) {
        self.width = None;
        self.height = None;
    }

    pub fn set_orientation(&mut self, orientation: Orientation) {
        self.orientation = orientation;
    }

    /// Begin a drag with the pointer `along_axis` pixels into the pane region.
    pub fn begin_drag(&mut self, along_axis: f32, available: f32) {
        self.drag = Some(along_axis - self.list_size(available));
    }

    /// Drag to the pointer `along_axis` pixels into the pane region.
    pub fn drag_to(&mut self, along_axis: f32, available: f32) -> Option<f32> {
        let origin = self.drag?;
        let size = along_axis - origin;
        self.remember(size);
        Some(self.list_size(available))
    }

    pub fn end_drag(&mut self) {
        self.drag = None;
    }

    fn remembered(&self) -> Option<f32> {
        match self.orientation {
            Orientation::SideBySide => self.width,
            Orientation::Stacked => self.height,
        }
    }

    fn remember(&mut self, size: f32) {
        match self.orientation {
            Orientation::SideBySide => self.width = Some(size),
            Orientation::Stacked => self.height = Some(size),
        }
    }

    /// Room the current orientation was last fitted to.
    fn fitted(&self) -> f32 {
        match self.orientation {
            Orientation::SideBySide => self.fitted_width,
            Orientation::Stacked => self.fitted_height,
        }
    }

    fn set_fitted(&mut self, available: f32) {
        match self.orientation {
            Orientation::SideBySide => self.fitted_width = available,
            Orientation::Stacked => self.fitted_height = available,
        }
    }

    /// `(min, max)` list size for `available` room. A window too small to hold
    /// both minimums splits what there is and lets the flex layout cope.
    fn limits(orientation: Orientation, available: f32) -> (f32, f32) {
        let (min, reader) = match orientation {
            Orientation::SideBySide => (MIN_LIST_W, MIN_READER_W),
            Orientation::Stacked => (MIN_LIST_H, MIN_READER_H),
        };
        let max = (available - reader).max(min);
        (min.min(available.max(1.)), max.min(available.max(1.)))
    }

    /// The size both orientations start from: the classic 42% list, and 45% of the
    /// height when the panes are stacked.
    fn default(orientation: Orientation, available: f32) -> f32 {
        match orientation {
            Orientation::SideBySide => crate::app::row::list_width(available),
            Orientation::Stacked => available * 0.45,
        }
    }
}

type DragHandler = Box<dyn Fn(&MouseDownEvent, &mut Window, &mut App) + 'static>;
type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// The hairline between the panes: a pointer band that highlights on hover, drags
/// on press and resets on double-click.
#[derive(IntoElement)]
pub struct Divider {
    orientation: Orientation,
    theme: Arc<Theme>,
    on_drag: DragHandler,
    on_reset: ClickHandler,
}

impl Divider {
    pub fn new(
        orientation: Orientation,
        theme: Arc<Theme>,
        on_drag: DragHandler,
        on_reset: ClickHandler,
    ) -> Self {
        Self { orientation, theme, on_drag, on_reset }
    }
}

impl RenderOnce for Divider {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let side_by_side = self.orientation == Orientation::SideBySide;
        let (line, highlight) = (self.theme.border, self.theme.accent.opacity(0.28));
        div()
            .id("pane-divider")
            .test_support()
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .when(side_by_side, |d| d.w(px(DIVIDER_HIT)).h_full().cursor_col_resize())
            .when(!side_by_side, |d| d.h(px(DIVIDER_HIT)).w_full().cursor_row_resize())
            .hover(move |d| d.bg(highlight))
            .on_mouse_down(MouseButton::Left, self.on_drag)
            .on_click(self.on_reset)
            .child(
                div()
                    .when(side_by_side, |d| d.w(px(1.)).h_full())
                    .when(!side_by_side, |d| d.h(px(1.)).w_full())
                    .bg(line),
            )
    }
}

/// Room the two panes share in a `window`, as `(width, height)`.
fn available(window: &Window) -> (f32, f32) {
    let size = window.viewport_size();
    (available_width(f32::from(size.width)), available_height(f32::from(size.height)))
}

impl MailApp {
    /// How the list and the reader are stacked.
    pub fn orientation(&self) -> Orientation {
        self.panes.orientation()
    }

    /// Size of the list pane as last laid out, in pixels along the stacking axis.
    pub fn list_pane_size(&self) -> f32 {
        self.panes.current_size()
    }

    /// Whether the divider is being dragged right now.
    pub fn pane_dragging(&self) -> bool {
        self.panes.dragging()
    }

    pub(super) fn grow_list_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.nudge_list_pane(STEP, window, cx);
    }

    pub(super) fn shrink_list_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.nudge_list_pane(-STEP, window, cx);
    }

    /// Move the divider along the stacking axis, keeping both panes usable.
    pub(super) fn nudge_list_pane(&mut self, delta: f32, window: &mut Window, cx: &mut Context<Self>) {
        let (width, height) = available(window);
        self.panes.nudge(self.axis_available(width, height), delta);
        cx.notify();
    }

    /// Back to the default sizes for both orientations.
    pub(super) fn reset_panes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.panes.reset();
        let (width, height) = available(window);
        self.panes.list_size(self.axis_available(width, height));
        self.show_toast("Pane sizes reset".into(), window, cx);
    }

    /// Stack the panes the other way round: list left, or list on top.
    pub(super) fn toggle_pane_layout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let next = self.panes.orientation().toggled();
        self.set_pane_layout(next, window, cx);
    }

    pub(super) fn set_pane_layout(&mut self, orientation: Orientation, window: &mut Window, cx: &mut Context<Self>) {
        self.panes.set_orientation(orientation);
        let (width, height) = available(window);
        self.panes.list_size(self.axis_available(width, height));
        self.show_toast(format!("Layout: {}", orientation.label()), window, cx);
    }

    /// Press on the divider: remember where in the pane the pointer grabbed it.
    pub(super) fn begin_divider_drag(&mut self, ev: &MouseDownEvent, window: &Window, cx: &mut Context<Self>) {
        let (width, height) = available(window);
        self.panes.begin_drag(self.pointer_along(ev.position), self.axis_available(width, height));
        cx.notify();
    }

    pub(super) fn drag_divider(&mut self, ev: &MouseMoveEvent, window: &Window, cx: &mut Context<Self>) {
        let (width, height) = available(window);
        self.panes.drag_to(self.pointer_along(ev.position), self.axis_available(width, height));
        cx.notify();
    }

    pub(super) fn end_divider_drag(&mut self, cx: &mut Context<Self>) {
        self.panes.end_drag();
        cx.notify();
    }

    /// Double-clicking the divider is the shortcut back to the default size.
    pub(super) fn reset_from_divider(&mut self, ev: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        if ev.click_count() == 2 {
            self.reset_panes(window, cx);
        }
    }

    /// The hairline between the list and the reader.
    pub(super) fn render_divider(&self, cx: &Context<Self>) -> impl IntoElement {
        let orientation = self.panes.orientation();
        Divider::new(
            orientation,
            crate::theme::active(cx),
            Box::new(cx.listener(|this, ev, window, cx| this.begin_divider_drag(ev, window, cx))),
            Box::new(cx.listener(|this, ev, window, cx| this.reset_from_divider(ev, window, cx))),
        )
    }

    /// Room the divider moves in, along the axis the panes are stacked on.
    fn axis_available(&self, width: f32, height: f32) -> f32 {
        match self.panes.orientation() {
            Orientation::SideBySide => width,
            Orientation::Stacked => height,
        }
    }

    /// Pointer position along the stacking axis. Window coordinates are fine:
    /// the drag origin absorbs where the pane region starts.
    fn pointer_along(&self, position: Point<Pixels>) -> f32 {
        match self.panes.orientation() {
            Orientation::SideBySide => f32::from(position.x),
            Orientation::Stacked => f32::from(position.y),
        }
    }
}