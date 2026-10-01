//! Headless mouse tests for selecting and copying text in the reader.

use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, Focusable, Point, TestAppContext, WindowBounds,
    WindowOptions, base::Root, point, px, size,
};
use mail_classifier::app::MailApp;
use mail_classifier::app::actions::bind_keys;
use mail_classifier::model::Mailbox;

const MAILBOX: &str = r#"[{"id":1,"thread_id":1,"from_name":"Ana Lima","from_email":"ana@atrium.studio","to":"you@example.com","subject":"Friday review","body":"Moving the review to ten.","received":"2026-09-30T20:46:00Z","state":"Inbox"},
{"id":2,"thread_id":2,"from_name":"Ben Ito","from_email":"ben@atrium.studio","to":"you@example.com","subject":"Other","body":"Unrelated.","received":"2026-09-29T20:46:00Z","state":"Inbox"}]"#;

fn open(cx: &mut TestAppContext) -> AnyWindowHandle {
    cx.update(|cx| {
        gpui_kit::init(cx);
        bind_keys(cx);
    });
    let window = cx.update(|cx| {
        let (window, _) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(1400.), px(900.)),
                })),
                ..Default::default()
            },
            cx,
            |window, cx| {
                let mailbox = Mailbox::from_json(MAILBOX).expect("valid mailbox");
                let view = cx.new(|cx| MailApp::new(mailbox, window, cx));
                window.focus(&view.focus_handle(cx), cx);
                view
            },
        )
        .expect("open window");
        window.downcast::<Root>().expect("root").into()
    });
    cx.run_until_parked();
    cx.simulate_keystrokes(window, "enter");
    cx.run_until_parked();
    window
}

#[gpui_kit::gpui::test]
fn dragging_across_the_reader_selects_header_and_body_and_cmd_c_copies_it(cx: &mut TestAppContext) {
    let window = open(cx);
    let (head, body) = cx
        .update_window(window, |_, w, cx| {
            w.render_frame(cx);
            (w.find(("reader-msg-head", 1usize)).bounds(), w.find("reader-body").bounds())
        })
        .expect("window alive");
    // From the sender row (the avatar's right edge, on the name) to the body's far corner.
    let from = point(head.origin.x + px(52.), head.origin.y + px(8.));
    let to = point(body.origin.x + body.size.width - px(1.), body.origin.y + body.size.height - px(1.));
    cx.update_window(window, |_, w, cx| w.drag(from, to, cx)).expect("window alive");
    cx.run_until_parked();
    cx.simulate_keystrokes(window, "cmd-c");
    let copied = cx.read_from_clipboard().and_then(|c| c.text()).unwrap_or_default();
    assert!(copied.contains("Ana Lima") && copied.contains("ana@atrium.studio"), "clipboard was {copied:?}");
    assert!(copied.contains("Moving the review to ten."), "clipboard was {copied:?}");
}
