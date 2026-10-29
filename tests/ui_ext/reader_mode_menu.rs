//! The message menu's Reader mode row: the reader footer's toggle moved into the menu the
//! reader's `⋯` and the row's right-click share, so it must switch the message from either.

use gpui_kit::TestAppContext;

use crate::harness::{harness_with, html_msg, mailbox};

/// The Reader mode row switches the message from either menu, whichever one opened it.
#[gpui_kit::gpui::test]
fn the_reader_mode_row_switches_from_both_menus(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox(&[html_msg()]));
    h.keys("enter");
    let id = h.read(|a| a.opened()).expect("a message is open");
    assert!(!h.read(|a| a.reader_plain(id)), "HTML shows as designed until asked");

    h.click(("btn-message-more", id as usize));
    assert!(h.has_row("Reader mode"), "the HTML message offers the switch");
    h.click_row("Reader mode");
    assert!(h.read(|a| a.reader_plain(id)), "the reader menu switched to Reader mode");

    h.click(("btn-message-more", id as usize));
    assert!(h.has_row("Show original"), "the label follows the mode");
    h.click_row("Show original");
    assert!(!h.read(|a| a.reader_plain(id)), "the reader menu switched back");
}

/// The row's right-click carries the same row, and switching from it targets the message too.
#[gpui_kit::gpui::test]
fn the_row_right_click_carries_the_reader_mode_row(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox(&[html_msg()]));
    h.keys("enter");
    let id = h.read(|a| a.opened()).expect("a message is open");

    h.right_click(("row", id as usize));
    assert!(h.has_row("Reader mode"), "the row menu offers the switch");
    h.click_row("Reader mode");
    assert!(h.read(|a| a.reader_plain(id)), "the row menu switched the right-clicked message");
}
