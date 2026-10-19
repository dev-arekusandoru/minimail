//! Preview line settings update the measured message-list rows.

use gpui_kit::{AppContext, TestAppContext};
use gpui_kit::test::TestWindowExt;

#[path = "ui_ext/harness.rs"]
#[allow(dead_code)]
mod harness;
use harness::{Harness, harness_with, mailbox, msg};

fn app(cx: &mut TestAppContext) -> Harness<'_> {
    let long = "MMMMMMMM ".repeat(53);
    let message = msg(1, 1, "alice@example.com", "Question", 1, "Inbox")
        .replace("Just checking in about this.", &long);
    harness_with(cx, mailbox(&[message]))
}

fn row_height(h: &mut Harness<'_>) -> f32 {
    h.cx.update_window(h.window, |_, window, cx| {
        window.render_frame(cx);
        f32::from(window.try_find(("row", 1usize)).expect("message row").bounds().size.height)
    }).unwrap()
}

#[gpui_kit::gpui::test]
fn preview_lines_control_changes_default_message_row_height(cx: &mut TestAppContext) {
    let mut h = app(cx);
    assert!(h.read(|app| mail_classifier::preview::snippet(
        &mail_classifier::reading::reader_text(app.mailbox.get(1).unwrap())
    ).len()) > 400);
    let two_lines = row_height(&mut h);
    h.settings_open_page(2);
    h.settings_click_in(0, 1, "decrement");
    h.settings_click_in(0, 1, "decrement");
    assert_eq!(h.read(|a| a.preview_lines), 0);
    let off = row_height(&mut h);
    assert!(off < two_lines, "Off removes the preview block: {off} >= {two_lines}");

    for _ in 0..5 {
        h.settings_click_in(0, 1, "increment");
    }
    assert_eq!(h.read(|a| a.preview_lines), 5);
    let five_lines = row_height(&mut h);
    assert!(five_lines > off, "five preview lines expand the row: {five_lines} <= {off}");
    h.settings_keys("escape");
    h.keys("cmd-k");
    h.type_text("cycle preview lines");
    h.keys("enter");
    assert_eq!(h.read(|a| a.preview_lines), 0);
    let cycled_off = row_height(&mut h);
    assert!(cycled_off < five_lines, "palette cycle hides previews: {cycled_off} >= {five_lines}");
}

