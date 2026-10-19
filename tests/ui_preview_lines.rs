//! Preview line settings update the measured message-list rows.

use gpui_kit::{AppContext, TestAppContext};
use gpui_kit::test::TestWindowExt;

#[path = "ui_ext/harness.rs"]
#[allow(dead_code)]
mod harness;
use harness::{Harness, harness_with, mailbox, msg};

fn app(cx: &mut TestAppContext) -> Harness<'_> {
    harness_with(cx, mailbox(&[msg(1, 1, "alice@example.com", "Question", 1, "Inbox")]))
}

fn row_height(h: &mut Harness<'_>) -> f32 {
    h.cx.update_window(h.window, |_, window, cx| {
        window.render_frame(cx);
        window.try_find(("row", 1usize)).expect("message row").bounds().size.height.0
    }).unwrap()
}

#[gpui_kit::gpui::test]
fn preview_lines_change_default_message_row_height(cx: &mut TestAppContext) {
    let mut h = app(cx);
    let two_lines = row_height(&mut h);
    h.app.update(h.cx, |app, cx| {
        app.preview_lines = 0;
        cx.notify();
    });
    h.cx.run_until_parked();
    let off = row_height(&mut h);
    assert!(off < two_lines, "Off removes the preview block: {off} >= {two_lines}");

    h.app.update(h.cx, |app, cx| {
        app.preview_lines = 5;
        cx.notify();
    });
    h.cx.run_until_parked();
    let five_lines = row_height(&mut h);
    assert!(five_lines > off, "five preview lines expand the row: {five_lines} <= {off}");
}
