//! Resizable panes: the divider drag, the keyboard sizes, the two orientations and
//! the settings entry that switches between them.

use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, ElementId, Entity, Focusable, Point, TestAppContext,
    WindowBounds, WindowOptions, base::Root, point, px, size,
};
use mail_classifier::app::MailApp;
use mail_classifier::app::actions::bind_keys;
use mail_classifier::app::mail_app::panes::{MIN_LIST_W, MIN_READER_W, Orientation, available_width};
use mail_classifier::model::{Mailbox, MessageId};

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
    h.keys("");
    h
}

impl Harness<'_> {
    fn keys(&mut self, keys: &str) {
        if !keys.is_empty() {
            self.cx.simulate_keystrokes(self.window, keys);
        }
        self.cx.run_until_parked();
    }
    fn read<R>(&mut self, f: impl FnOnce(&MailApp) -> R) -> R {
        self.cx.read_entity(&self.app, |a, _| f(a))
    }
    fn orientation(&mut self) -> Orientation {
        self.read(|a| a.orientation())
    }
    fn list_size(&mut self) -> f32 {
        self.read(|a| a.list_pane_size())
    }
    fn click(&mut self, id: impl Into<ElementId>) {
        let id = id.into();
        self.cx.update_window(self.window, |_, window, cx| window.click(id, cx)).expect("window alive");
        self.cx.run_until_parked();
    }
    fn double_click(&mut self, id: impl Into<ElementId>) {
        let id = id.into();
        self.cx.update_window(self.window, |_, window, cx| window.double_click(id, cx)).expect("window alive");
        self.cx.run_until_parked();
    }
    fn bounds(&mut self, id: impl Into<ElementId>) -> Bounds<gpui_kit::Pixels> {
        let id = id.into();
        let bounds = self.cx.update_window(self.window, |_, window, _| window.find(id).bounds()).expect("window alive");
        self.cx.run_until_parked();
        bounds
    }
    /// The window's viewport in pixels, which is what the panes are clamped against.
    fn viewport(&mut self) -> (f32, f32) {
        let size = self
            .cx
            .update_window(self.window, |_, window, _| window.viewport_size())
            .expect("window alive");
        (f32::from(size.width), f32::from(size.height))
    }
    fn drag_divider(&mut self, dx: f32, dy: f32) {
        let bounds = self.bounds("pane-divider");
        let from = point(
            px(f32::from(bounds.origin.x) + f32::from(bounds.size.width) / 2.),
            px(f32::from(bounds.origin.y) + f32::from(bounds.size.height) / 2.),
        );
        let to = point(px(f32::from(from.x) + dx), px(f32::from(from.y) + dy));
        self.cx.update_window(self.window, |_, window, cx| window.drag(from, to, cx)).expect("window alive");
        self.cx.run_until_parked();
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
    assert!(!h.read(|a| a.pane_dragging()));

    h.drag_divider(-240., 0.);
    let shrunk = h.list_size();
    assert!(shrunk < before, "drag left should narrow the list: {shrunk}");

    // The divider can be dragged, so it is there at all.
    let divider = h.bounds("pane-divider");
    assert!(divider.size.width > px(0.) && divider.size.height > px(0.));
}
#[gpui_kit::gpui::test]
fn drag_is_clamped_to_usable_panes(cx: &mut TestAppContext) {
    let mut h = harness(cx, 1400., 900.);
    h.drag_divider(-2000., 0.);
    assert_eq!(h.list_size(), MIN_LIST_W, "the list pane never collapses");

    let (viewport_w, _) = h.viewport();
    let max = available_width(viewport_w) - MIN_READER_W;
    h.drag_divider(4000., 0.);
    let stretched = h.list_size();
    assert!(stretched > MIN_LIST_W && stretched <= max, "{stretched} must stay under {max}");
    h.drag_divider(4000., 0.);
    assert_eq!(h.list_size(), stretched, "the pointer is already at the end of the window");
    h.drag_divider(-4000., 0.);
    assert_eq!(h.list_size(), MIN_LIST_W, "and back down to the minimum");
}

#[gpui_kit::gpui::test]
fn double_click_on_the_divider_resets_the_size(cx: &mut TestAppContext) {
    let mut h = harness(cx, 1400., 900.);
    let default = h.list_size();
    h.drag_divider(180., 0.);
    assert_ne!(h.list_size(), default);
    h.double_click("pane-divider");
    assert_eq!(h.list_size(), default);
}

#[gpui_kit::gpui::test]
fn keyboard_grows_and_shrinks_the_list_pane(cx: &mut TestAppContext) {
    let mut h = harness(cx, 1400., 900.);
    let start = h.list_size();
    h.keys("alt-right");
    assert_eq!(h.list_size(), start + 40.);
    h.keys("alt-right alt-right");
    assert_eq!(h.list_size(), start + 120.);
    h.keys("alt-left");
    assert_eq!(h.list_size(), start + 80.);
    h.keys("alt-r");
    assert_eq!(h.list_size(), start);
}

#[gpui_kit::gpui::test]
fn keyboard_shrink_stops_at_the_minimum(cx: &mut TestAppContext) {
    let mut h = harness(cx, 1400., 900.);
    h.keys(&"alt-left ".repeat(40));
    assert_eq!(h.list_size(), MIN_LIST_W);
}

#[gpui_kit::gpui::test]
fn a_narrow_window_clamps_the_panes(cx: &mut TestAppContext) {
    let mut h = harness(cx, 900., 700.);
    let (viewport_w, _) = h.viewport();
    let available = available_width(viewport_w);
    let list = h.list_size();
    assert!(list >= MIN_LIST_W, "the list keeps a usable width: {list}");
    assert!(available - list >= MIN_READER_W - 0.5, "the reader keeps a usable width: {}", available - list);

    h.drag_divider(2000., 0.);
    let stretched = h.list_size();
    assert!(stretched <= available - MIN_READER_W, "{stretched} leaves the reader nothing");
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
    let vertical = h.bounds("pane-divider");
    assert!(vertical.size.height > vertical.size.width, "side by side the divider stands up");

    h.keys("alt-l");
    assert_eq!(h.orientation(), Orientation::Stacked);
    let horizontal = h.bounds("pane-divider");
    assert!(horizontal.size.width > horizontal.size.height, "stacked the divider lies down");

    h.keys("alt-l");
    assert_eq!(h.orientation(), Orientation::SideBySide);
    assert_eq!(h.bounds("pane-divider").size.height, vertical.size.height);
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

    h.keys("alt-l");
    assert_eq!(h.list_size(), side_by_side, "going back restores the side-by-side size");
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
        (stacked - available_width(1400.)).abs() < 2.,
        "stacked rows use the full pane width: {stacked}"
    );
    assert!(stacked > side_by_side);
}

#[gpui_kit::gpui::test]
fn settings_pane_layout_entry_switches_the_orientation(cx: &mut TestAppContext) {
    let mut h = harness(cx, 1400., 900.);
    h.keys("cmd-,");
    h.click("settings-search");
    h.cx.update_window(h.window, |_, window, cx| window.input("pane layout", cx)).expect("window alive");
    h.cx.run_until_parked();
    h.click("pane-layout-row");
    assert_eq!(h.orientation(), Orientation::Stacked);
    h.click("pane-layout-row");
    assert_eq!(h.orientation(), Orientation::SideBySide);
}