//! Viewport-constrained modal helpers shared by every overlay.

use gpui_kit::*;

/// Fraction of the viewport a modal may occupy.
const FRACTION: f32 = 0.9;
/// Preferred gap above top-anchored modals (palette, snooze, settings, rules).
const TOP: f32 = 80.;

fn viewport(window: &Window) -> (f32, f32) {
    let size = window.viewport_size();
    (f32::from(size.width), f32::from(size.height))
}

/// Top gap that shrinks with the window so a modal never starts below the fold.
pub fn top_offset(window: &Window) -> f32 {
    TOP.min(viewport(window).1 * 0.1)
}

/// Width for a modal that prefers `want` px but stays within the viewport.
pub fn fit_width(window: &Window, want: f32) -> f32 {
    want.min(viewport(window).0 * FRACTION)
}

/// Maximum height for a modal placed `top` px from the window's top edge.
pub fn fit_height(window: &Window, top: f32) -> f32 {
    let vh = viewport(window).1;
    (vh - top - vh * (1. - FRACTION) / 2.).max(0.)
}

/// Sizes a modal to its preferred width, clamped to the viewport, with height
/// capped so it fits below the top-anchored overlay gap.
pub trait FitViewport: Styled + Sized {
    fn fit_viewport(self, window: &Window, width: f32) -> Self {
        let top = top_offset(window);
        self.w(px(fit_width(window, width))).max_h(px(fit_height(window, top)))
    }
}

impl<T: Styled + Sized> FitViewport for T {}

/// Top-anchored full-window backdrop for a panel entity.
pub fn overlay(window: &Window, view: impl IntoElement) -> Div {
    div()
        .absolute()
        .inset_0()
        .occlude()
        .flex()
        .items_start()
        .justify_center()
        .pt(px(top_offset(window)))
        .child(view)
}
