use std::{cell::RefCell, rc::Rc};

use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, Focusable, Point, TestAppContext, WindowBounds,
    WindowOptions, base::Root, px, size,
};
use gpui_kit::test::TestWindowExt;
use mail_classifier::{app::{actions::bind_keys, settings::{SettingsEvent, SettingsPanel}}, judge::JudgePolicy};

struct PanelHarness<'a> {
    cx: &'a mut TestAppContext,
    window: AnyWindowHandle,
    events: Rc<RefCell<Vec<String>>>,
}
fn panel(cx: &mut TestAppContext) -> PanelHarness<'_> {
    cx.update(|cx| { gpui_kit::init(cx); bind_keys(cx); });
    let events = Rc::new(RefCell::new(Vec::new()));
    let captured = events.clone();
    let window = cx.update(|cx| {
        let (window, panel) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(), size: size(px(1200.), px(800.)),
                })),
                ..Default::default()
            }, cx, |window, cx| {
                let panel = cx.new(|cx| SettingsPanel::new(JudgePolicy::default(), false, false, 2, window, cx));
                window.focus(&panel.focus_handle(cx), cx);
                panel
            },
        ).expect("open settings window");
        let subscription = cx.subscribe(&panel, move |_, _, event: &SettingsEvent, _| {
            captured.borrow_mut().push(match event {
                SettingsEvent::Changed(_, summaries) => format!("changed:{summaries}"),
                SettingsEvent::Grouping(on) => format!("group:{on}"),
                SettingsEvent::PreviewLines(n) => format!("preview:{n}"),
                SettingsEvent::Close => "close".into(),
            });
        });
        subscription.detach();
        window.downcast::<Root>().expect("root").into()
    });
    cx.run_until_parked();
    PanelHarness { cx, window, events }
}

impl PanelHarness<'_> {
    fn click(&mut self, id: impl Into<gpui_kit::ElementId>) {
        let id = id.into();
        self.cx.update_window(self.window, |_, window, cx| window.click(id, cx)).expect("window alive");
        self.cx.run_until_parked();
    }

    fn type_text(&mut self, text: &str) {
        self.cx.update_window(self.window, |_, window, cx| window.input(text, cx)).expect("window alive");
        self.cx.run_until_parked();
    }
}

#[gpui_kit::gpui::test]
fn switching_sections_and_changing_toggle_enum_and_stepper_emit_events(cx: &mut TestAppContext) {
    let mut h = panel(cx);
    h.click(("settings-section", "General"));
    h.click("summaries-row");
    h.click(("settings-section", "Appearance / Theme"));
    h.click("theme-row");
    h.click(("settings-section", "Inbox & Threads"));
    h.click("group-row");
    h.click("preview-lines-row");
    assert!(h.events.borrow().contains(&"changed:true".to_owned()));
    assert!(h.events.borrow().contains(&"group:true".to_owned()));
    assert!(h.events.borrow().iter().any(|e| e.starts_with("preview:")));
    assert!(h.events.borrow().iter().any(|e| e == "changed:false"));
}

#[gpui_kit::gpui::test]
fn search_filters_across_sections(cx: &mut TestAppContext) {
    let mut h = panel(cx);
    h.click("settings-search");
    h.type_text("theme");
    h.click("theme-row");
    assert!(h.events.borrow().iter().any(|e| e.starts_with("changed:")));
}
