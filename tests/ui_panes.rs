//! Resizable panes on the dock: the divider drags, the keyboard sizes, the two
//! orientations, the collapsible sidebar and the titlebar and keyboard toggles.

use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, ElementId, Entity, Focusable, Point, TestAppContext, WindowBounds,
    WindowOptions, base::Root, point, px, size,
};
use mail_classifier::app::MailApp;
use mail_classifier::app::actions::bind_keys;
use mail_classifier::app::mail_app::panes::{
    MAX_SIDEBAR_W, MIN_LIST_H, MIN_LIST_W, MIN_SIDEBAR_W, Orientation, SIDEBAR_W, available_width,
};
use mail_classifier::model::{Mailbox, MessageId};

/// The dock draws its own resize handles and registers neither for observation, so the
/// tests drag at the coordinates the layout puts them at. `sidebar-divider` is ours: a
/// transparent band over the sidebar's edge, whose box is the dock region's own.
const SIDEBAR_DIVIDER: &str = "sidebar-divider";

struct Harness<'a> {
    cx: &'a mut TestAppContext,
    window: AnyWindowHandle,
    app: Entity<MailApp>,
}

fn harness(cx: &mut TestAppContext, w: f32, h: f32) -> Harness<'_> {
    cx.update(|cx| {
        gpui_kit::init(cx);
        bind_keys(cx);
    });
    let (window, app) = cx.update(|cx| {
        let (window, content) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(w), px(h)),
                })),
                ..Default::default()
            },
            cx,
            |window, cx| {
                let view = cx.new(|cx| MailApp::new(Mailbox::load_default(), window, cx));
                window.focus(&view.focus_handle(cx), cx);
                view
            },
        )
        .expect("open window");
        (window.downcast::<Root>().expect("root").into(), content)
    });
    let mut h = Harness { cx, window, app };
    h.frame();
    h
}

impl Harness<'_> {
    /// Draw a frame. The dock settles a divider move and the sidebar's width clamp on a
    /// frame, so anything read from the layout is read after one.
    fn frame(&mut self) {
        self.cx.update_window(self.window, |_, window, cx| window.render_frame(cx)).expect("window alive");
        self.cx.run_until_parked();
    }
    fn keys(&mut self, keys: &str) {
        if !keys.is_empty() {
            self.cx.simulate_keystrokes(self.window, keys);
        }
        self.frame();
    }
    fn read<R>(&mut self, f: impl FnOnce(&MailApp) -> R) -> R {
        self.frame();
        self.cx.read_entity(&self.app, |a, _| f(a))
    }
    fn orientation(&mut self) -> Orientation {
        self.read(|a| a.orientation())
    }
    fn list_size(&mut self) -> f32 {
        self.frame();
        self.cx.read_entity(&self.app, |a, cx| a.list_pane_size(cx))
    }
    fn click(&mut self, id: impl Into<ElementId>) {
        let id = id.into();
        self.cx.update_window(self.window, |_, window, cx| window.click(id, cx)).expect("window alive");
        self.frame();
    }
    fn double_click(&mut self, id: impl Into<ElementId>) {
        let id = id.into();
        self.cx.update_window(self.window, |_, window, cx| window.double_click(id, cx)).expect("window alive");
        self.frame();
    }
    fn bounds(&mut self, id: impl Into<ElementId>) -> Bounds<gpui_kit::Pixels> {
        self.frame();
        let id = id.into();
        let bounds = self.cx.update_window(self.window, |_, window, _| window.find(id).bounds()).expect("window alive");
        self.cx.run_until_parked();
        bounds
    }
    fn sidebar_width(&mut self) -> f32 {
        self.frame();
        self.cx.read_entity(&self.app, |a, cx| a.sidebar_width(cx))
    }
    /// A point on the sidebar's resize handle, which hugs the dock's trailing edge.
    fn sidebar_edge(&mut self) -> Point<gpui_kit::Pixels> {
        let region = self.bounds(SIDEBAR_DIVIDER);
        let width = self.sidebar_width();
        point(px(width - 2.5), region.center().y)
    }
    /// The point on the centre split's divider: the list slot's trailing edge, along the
    /// axis the current orientation stacks the panes on.
    fn centre_divider(&mut self) -> Point<gpui_kit::Pixels> {
        let region = self.bounds(SIDEBAR_DIVIDER);
        let top = f32::from(region.origin.y);
        let middle_y = top + f32::from(region.size.height) / 2.;
        let sidebar = if self.read(|a| a.sidebar_visible()) { self.sidebar_width() } else { 0. };
        let size = self.list_size();
        match self.orientation() {
            Orientation::SideBySide => point(px(sidebar + size), px(middle_y)),
            Orientation::Stacked => point(px(sidebar + 300.), px(top + size)),
        }
    }
    fn drag_from(&mut self, from: Point<gpui_kit::Pixels>, dx: f32, dy: f32) {
        let to = point(px(f32::from(from.x) + dx), px(f32::from(from.y) + dy));
        self.cx.update_window(self.window, |_, window, cx| window.drag(from, to, cx)).expect("window alive");
        self.frame();
    }
    fn drag_divider(&mut self, dx: f32, dy: f32) {
        let from = self.centre_divider();
        self.drag_from(from, dx, dy);
    }
    fn drag_sidebar(&mut self, dx: f32, dy: f32) {
        let from = self.sidebar_edge();
        self.drag_from(from, dx, dy);
    }
    fn ids(&mut self) -> Vec<MessageId> {
        self.read(|a| a.visible_ids())
    }
}

#[gpui_kit::gpui::test]
fn divider_drag_resizes_the_list_pane(cx: &mut TestAppContext) {
    let mut h = harness(cx, 1400., 900.);
    let before = h.list_size();
    h.drag_divider(120., 0.);
    assert!(h.list_size() > before + 60., "drag right should widen the list: {} -> {}", before, h.list_size());

    h.drag_divider(-240., 0.);
    let shrunk = h.list_size();
    assert!(shrunk < before, "drag left should narrow the list: {shrunk}");

    // Side by side the divider is a vertical line, so only a horizontal drag moves it.
    h.drag_divider(0., 0.);
    let width_before = h.list_size();
    h.drag_divider(0., 200.);
    assert!((h.list_size() - width_before).abs() < 1., "a vertical drag must not move the divider");
}

/// LOSS: the dock's centre slots are built without `size_range`, so the divider no longer
/// stops at `MIN_LIST_W` / `MIN_READER_W` — only the dock's own 100px floor and the room
/// left for the reader. The keyboard path still clamps (see
/// `keyboard_shrink_stops_at_the_minimum`), because that one is ours.
#[gpui_kit::gpui::test]
fn the_centre_divider_has_no_pane_minimum(cx: &mut TestAppContext) {
    let mut h = harness(cx, 1400., 900.);
    h.drag_divider(-2000., 0.);
    assert!(
        (0. < h.list_size()) && h.list_size() < MIN_LIST_W,
        "the list can now be dragged below its old minimum: {}",
        h.list_size()
    );

    // Neither pane has a floor but the dock's own 100px, so the reader can be squeezed
    // to it — the old `MIN_READER_W` is gone with the rest of the centre ranges.
    let max = available_width(1400., SIDEBAR_W) - 100.;
    h.drag_divider(4000., 0.);
    let stretched = h.list_size();
    assert!(stretched > MIN_LIST_W && stretched <= max + 1., "{stretched} must stay under {max}");
}

#[gpui_kit::gpui::test]
fn keyboard_grows_and_shrinks_the_list_pane(cx: &mut TestAppContext) {
    let mut h = harness(cx, 1400., 900.);
    let start = h.list_size();
    h.keys("alt-right");
    assert!((h.list_size() - (start + 40.)).abs() < 1., "{}", h.list_size());
    h.keys("alt-right alt-right");
    assert!((h.list_size() - (start + 120.)).abs() < 1.);
    h.keys("alt-left");
    assert!((h.list_size() - (start + 80.)).abs() < 1.);
    h.keys("alt-r");
    assert!((h.list_size() - start).abs() < 1.);
}

#[gpui_kit::gpui::test]
fn keyboard_shrink_stops_at_the_minimum(cx: &mut TestAppContext) {
    let mut h = harness(cx, 1400., 900.);
    h.keys(&"alt-left ".repeat(40));
    assert!((h.list_size() - MIN_LIST_W).abs() < 1., "{}", h.list_size());
}

/// LOSS: the dock draws its own centre divider and offers no double-click on it, so the
/// old double-click-to-reset is gone. `alt-r` is the way back to the default sizes now,
/// and it is what `keyboard_grows_and_shrinks_the_list_pane` covers.
#[gpui_kit::gpui::test]
fn a_narrow_window_keeps_the_list_usable(cx: &mut TestAppContext) {
    let mut h = harness(cx, 900., 700.);
    assert!(h.list_size() >= MIN_LIST_W - 1., "the list keeps a usable width: {}", h.list_size());
    h.drag_divider(2000., 0.);
    assert!(h.list_size() >= MIN_LIST_W - 1.);
}

#[gpui_kit::gpui::test]
fn toolbar_button_toggles_the_pane_layout(cx: &mut TestAppContext) {
    let mut h = harness(cx, 1400., 900.);
    assert_eq!(h.orientation(), Orientation::SideBySide);
    h.click("btn-layout");
    assert_eq!(h.orientation(), Orientation::Stacked);
    h.click("btn-layout");
    assert_eq!(h.orientation(), Orientation::SideBySide);
}

#[gpui_kit::gpui::test]
fn key_toggles_the_orientation_and_the_divider_follows(cx: &mut TestAppContext) {
    let mut h = harness(cx, 1400., 900.);
    // Side by side the divider stands up: only a horizontal drag moves it.
    let size = h.list_size();
    h.drag_divider(0., 200.);
    assert!((h.list_size() - size).abs() < 1.);
    h.drag_divider(100., 0.);
    assert!(h.list_size() > size + 60.);

    h.keys("alt-l");
    assert_eq!(h.orientation(), Orientation::Stacked);
    // Stacked it lies down: only a vertical drag moves it, and the size is a height.
    let stacked = h.list_size();
    h.drag_divider(100., 0.);
    assert!((h.list_size() - stacked).abs() < 1.);
    h.drag_divider(0., -100.);
    assert!(h.list_size() < stacked - 50.);

    h.keys("alt-l");
    assert_eq!(h.orientation(), Orientation::SideBySide);
}

#[gpui_kit::gpui::test]
fn each_orientation_remembers_its_own_size(cx: &mut TestAppContext) {
    let mut h = harness(cx, 1400., 900.);
    let wide = h.list_size();
    h.drag_divider(100., 0.);
    let side_by_side = h.list_size();
    assert!(side_by_side > wide);

    h.keys("alt-l");
    let stacked = h.list_size();
    h.drag_divider(0., -140.);
    let stacked_dragged = h.list_size();
    assert!(stacked_dragged < stacked, "stacked the divider moves the height");
    assert!(stacked_dragged >= MIN_LIST_H - 1.);

    h.keys("alt-l");
    assert!((h.list_size() - side_by_side).abs() < 1., "going back restores the side-by-side size");
}

#[gpui_kit::gpui::test]
fn rows_span_the_pane_in_both_orientations(cx: &mut TestAppContext) {
    let mut h = harness(cx, 1400., 900.);
    let id = h.ids()[0];
    let side_by_side = f32::from(h.bounds(("row", id as usize)).size.width);
    let list = h.list_size();
    assert!((side_by_side - list).abs() < 2., "row fills the list pane: {side_by_side} vs {list}");

    h.keys("alt-l");
    let stacked = f32::from(h.bounds(("row", id as usize)).size.width);
    assert!(
        (stacked - available_width(1400., SIDEBAR_W)).abs() < 2.,
        "stacked rows use the full pane width: {stacked}"
    );
    assert!(stacked > side_by_side);
}

#[gpui_kit::gpui::test]
fn sidebar_drag_resizes_within_limits(cx: &mut TestAppContext) {
    let mut h = harness(cx, 1400., 900.);
    assert!((h.sidebar_width() - SIDEBAR_W).abs() < 1.);

    h.drag_sidebar(60., 0.);
    assert!((h.sidebar_width() - (SIDEBAR_W + 60.)).abs() < 8., "drag right widens: {}", h.sidebar_width());
    h.drag_sidebar(2000., 0.);
    assert!((h.sidebar_width() - MAX_SIDEBAR_W).abs() < 1.);
    h.drag_sidebar(-2000., 0.);
    assert!((h.sidebar_width() - MIN_SIDEBAR_W).abs() < 1.);
}

#[gpui_kit::gpui::test]
fn double_click_on_the_sidebar_edge_resets_its_width(cx: &mut TestAppContext) {
    let mut h = harness(cx, 1400., 900.);
    h.drag_sidebar(100., 0.);
    assert!(h.sidebar_width() > SIDEBAR_W + 50.);
    h.double_click("sidebar-divider");
    assert!((h.sidebar_width() - SIDEBAR_W).abs() < 1., "{}", h.sidebar_width());
}

#[gpui_kit::gpui::test]
fn cmd_b_collapses_and_restores_the_sidebar_at_its_width(cx: &mut TestAppContext) {
    let mut h = harness(cx, 1400., 900.);
    h.drag_sidebar(60., 0.);
    let width = h.sidebar_width();
    let list = h.list_size();
    let row = h.ids()[0];
    let before = h.bounds(("row", row as usize)).origin.x;

    h.keys("cmd-b");
    assert!(!h.read(|a| a.sidebar_visible()));
    let hidden = h.bounds(("row", row as usize)).origin.x;
    assert!(hidden < before, "the list moves left into the freed room: {hidden} vs {before}");

    h.keys("cmd-b");
    assert!(h.read(|a| a.sidebar_visible()));
    assert!((h.sidebar_width() - width).abs() < 1., "width kept: {} vs {width}", h.sidebar_width());
    assert!((h.list_size() - list).abs() < 2., "list kept: {} vs {list}", h.list_size());
}

#[gpui_kit::gpui::test]
fn titlebar_button_toggles_the_sidebar(cx: &mut TestAppContext) {
    let mut h = harness(cx, 1400., 900.);
    h.click("btn-sidebar");
    assert!(!h.read(|a| a.sidebar_visible()));
    h.click("btn-sidebar");
    assert!(h.read(|a| a.sidebar_visible()));
}