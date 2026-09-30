use std::{cell::RefCell, rc::Rc};

use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, Focusable, Point, TestAppContext, WindowBounds,
    WindowOptions, base::Root, px, size,
};
use gpui_kit::test::TestWindowExt;
use mail_classifier::app::mail_app::panes::Orientation;
use mail_classifier::{app::{actions::bind_keys, settings::{SettingsEvent, SettingsPanel}}, judge::JudgePolicy};

#[path = "ui_ext/harness.rs"]
#[allow(dead_code)]
mod harness;

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
                let panel = cx.new(|cx| SettingsPanel::new(JudgePolicy::default(), false, false, 2, Orientation::SideBySide, window, cx));
                window.focus(&panel.focus_handle(cx), cx);
                panel
            },
        ).expect("open settings window");
        let subscription = cx.subscribe(&panel, move |_, event: &SettingsEvent, _| {
            captured.borrow_mut().push(match event {
                SettingsEvent::Changed(_, summaries) => format!("changed:{summaries}"),
                SettingsEvent::Grouping(on) => format!("group:{on}"),
                SettingsEvent::TabAvatars(on) => format!("tab-avatars:{on}"),
                SettingsEvent::PreviewLines(n) => format!("preview:{n}"),
                SettingsEvent::PaneLayout(o) => format!("layout:{}", o.label()),
                SettingsEvent::FollowUp(timeout) => format!("followup:{timeout}"),
                SettingsEvent::Unblock(email) => format!("unblock:{email}"),
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
    h.click(("settings-section", 0usize));
    h.click("summaries-row");
    h.click(("settings-section", 1usize));
    h.click("theme-row");
    h.click(("settings-section", 2usize));
    h.click("group-row");
    h.click("preview-lines-row");
    assert!(h.events.borrow().contains(&"changed:true".to_owned()));
    assert!(h.events.borrow().contains(&"group:true".to_owned()));
    assert!(h.events.borrow().iter().any(|e| e.starts_with("preview:")));
    assert!(h.events.borrow().iter().filter(|event| *event == "changed:true").count() >= 2);
}

#[gpui_kit::gpui::test]
fn search_filters_across_sections(cx: &mut TestAppContext) {
    let mut h = panel(cx);
    h.click("settings-search");
    h.type_text("theme");
    h.click("theme-row");
    assert!(h.events.borrow().iter().any(|e| e.starts_with("changed:")));
}

#[gpui_kit::gpui::test]
fn lowering_follow_up_after_resurfaces_a_waiting_thread_on_tick(cx: &mut TestAppContext) {
    use mail_classifier::clock::DAY;
    use mail_classifier::model::{Tag, TriageState};
    let mut h = harness::harness_with(
        cx,
        harness::mailbox(&[harness::msg(1, 1, "alice@example.com", "Question", 1, "Inbox")]),
    );
    // Reply and expect an answer: the thread now waits on the default 3-day timeout.
    h.app.update(h.cx, |a, _| {
        a.mailbox.send_reply_at(1, "on it".into(), true, harness::NOON)
    });
    assert_eq!(h.read(|a| a.mailbox.follow_up_timeout()), 3 * DAY);
    h.advance(DAY);
    assert_eq!(h.state_of(1), TriageState::Inbox);
    assert!(!h.has_tag(1, Tag::FollowUp), "three days of patience: one day is not enough");

    // Settings → Inbox & Threads → Follow-up after: 3 days down to 1.
    h.keys("cmd-,");
    h.click(("settings-section", 2usize));
    h.click("follow-up-down");
    h.click("follow-up-down");
    assert_eq!(h.read(|a| a.mailbox.follow_up_timeout()), DAY, "the stepper writes the timeout");
    h.keys("escape");

    h.tick();
    assert_eq!(h.state_of(1), TriageState::Inbox);
    assert!(h.has_tag(1, Tag::FollowUp), "the overdue thread is flagged for follow-up");
    assert!(!h.has_tag(1, Tag::AwaitingReply), "resurfacing clears Awaiting Reply");
}

#[gpui_kit::gpui::test]
fn unblocking_a_sender_from_settings_is_one_undoable_step(cx: &mut TestAppContext) {
    let mut h = harness::harness_with(
        cx,
        harness::mailbox(&[harness::msg(1, 1, "spam@example.com", "Deal", 1, "Inbox")]),
    );
    h.app.update(h.cx, |a, _| {
        a.mailbox.block_sender("spam@example.com", None);
    });
    assert!(h.read(|a| a.mailbox.blocked().contains(&"spam@example.com".to_owned())));

    h.keys("cmd-,");
    h.click(("settings-section", 3usize));
    h.click(("blocked-unblock", 0usize));
    assert!(h.read(|a| a.mailbox.blocked().is_empty()), "Unblock removes the sender");

    h.keys("escape");
    h.keys("u");
    assert!(
        h.read(|a| a.mailbox.blocked().contains(&"spam@example.com".to_owned())),
        "undo restores the block"
    );
}


#[gpui_kit::gpui::test]
fn the_tab_avatar_row_turns_the_setting_off_and_back_on(cx: &mut TestAppContext) {
    let mut h = harness::harness_with(
        cx,
        harness::mailbox(&[harness::msg(1, 1, "alice@example.com", "Question", 1, "Inbox")]),
    );
    assert!(h.read(|a| a.tab_avatars), "on by default");
    h.keys("cmd-,");
    h.click(("settings-section", 1usize));
    h.click("tab-avatars-row");
    assert!(!h.read(|a| a.tab_avatars));
    // The panel reopens showing the current value, so the next click turns it back on.
    h.keys("escape cmd-,");
    h.click(("settings-section", 1usize));
    h.click("tab-avatars-row");
    assert!(h.read(|a| a.tab_avatars));
}
