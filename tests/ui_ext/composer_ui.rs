//! Composer UI behaviour: the Collapse affordance, where collapsing lands in the thread,
//! and the inline Cc/Bcc reveal.

use crate::harness::{harness, harness_with, mailbox, msg};
use gpui_kit::TestAppContext;

/// Collapse is only offered for a draft that came out of a thread. A fresh message has no
/// thread to go back to, so it never shows the control; an expanded reply does.
#[gpui_kit::gpui::test]
pub fn collapse_is_offered_only_for_a_draft_that_came_from_a_thread(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.compose_reply("from a thread");
    h.expand_active_draft();
    assert!(h.read(|a| a.compose_in_tab()), "the reply is in a tab");
    assert!(
        h.has("compose-collapse"),
        "an expanded reply can collapse back to its thread"
    );

    h.new_draft();
    assert!(
        !h.has("compose-collapse"),
        "a fresh message has no thread, so it offers no Collapse"
    );
}

/// Collapsing puts the reply back under its thread and brings the reader down to it, so the
/// inline composer — the last child of the thread — is on screen rather than below the fold.
#[gpui_kit::gpui::test]
pub fn collapsing_scrolls_the_thread_to_the_inline_composer(cx: &mut TestAppContext) {
    // One long conversation, so the thread is taller than the reader viewport.
    let msgs: Vec<String> = (0..12)
        .map(|i| msg(200 + i, 7, "alice@example.com", "Long conversation", 10, "Inbox"))
        .collect();
    let mut h = harness_with(cx, mailbox(&msgs));

    h.compose_reply("at the bottom");
    h.expand_active_draft();
    h.collapse_active_draft();

    assert!(
        h.read(|a| a.inline_compose_open()),
        "the reply came back inline under its thread"
    );
    let (offset, max) = h.read(|a| a.reader_scroll(7)).expect("thread 7 is open");
    assert!(max > 0.0, "the thread is longer than the viewport");
    assert!(
        (offset + max).abs() < 1.0,
        "the thread is scrolled to its bottom (composer visible): offset {offset}, max {max}"
    );
}

/// Cc and Bcc are hidden while empty. Each is revealed by its own inline link on the To
/// row, which puts the caret in the field; a field that already carries recipients shows
/// itself without a click.
#[gpui_kit::gpui::test]
pub fn cc_and_bcc_reveal_inline_and_only_when_asked(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.new_draft();

    assert!(h.has("compose-show-cc"), "the Cc link is on the To row");
    assert!(h.has("compose-show-bcc"), "the Bcc link is on the To row");
    assert!(!h.has("compose-cc"), "an empty Cc row is hidden");
    assert!(!h.has("compose-bcc"), "an empty Bcc row is hidden");

    h.click("compose-show-cc");
    assert!(h.has("compose-cc"), "clicking Cc reveals its row");
    assert!(!h.has("compose-show-cc"), "the Cc link hides once shown");
    assert!(!h.has("compose-bcc"), "revealing Cc leaves Bcc alone");

    // The reveal focused the field: typing lands in Cc.
    h.type_text("newcc@example.com");
    h.keys("enter");
    h.settle();
    let cc = h.read(|a| {
        let id = a.tabs.active_draft().expect("draft tab");
        a.composer.record(id).expect("record").cc.clone()
    });
    assert_eq!(cc, "newcc@example.com", "the caret was in Cc");

    h.click("compose-show-bcc");
    assert!(h.has("compose-bcc"), "clicking Bcc reveals its row");
    assert!(!h.has("compose-show-bcc"), "the Bcc link hides once shown");
}

/// A reply-all that carries a Cc opens with the Cc row already shown.
#[gpui_kit::gpui::test]
pub fn a_reply_with_cc_shows_the_field_automatically(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    // The first fixture message has a Cc, so a reply-all prefills it.
    h.goto(1);
    h.keys("shift-a");
    h.await_composer("reply all opens inline", |a| a.inline_compose_open());
    h.settle();
    let cc = h.read(|a| {
        let id = a.inline_composer.expect("inline reply");
        a.composer.record(id).expect("record").cc.clone()
    });
    assert!(!cc.trim().is_empty(), "reply-all prefilled a Cc");
    assert!(h.has("compose-cc"), "a prefilled Cc is shown");
    assert!(
        !h.has("compose-show-cc"),
        "there is nothing to reveal for a field already shown"
    );
}

/// The footer offers Save & Close instead of Close; it flushes the draft and closes, keeping
/// the draft (a close never deletes).
#[gpui_kit::gpui::test]
pub fn save_and_close_flushes_and_keeps_the_draft(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.new_draft();
    h.type_text("keep me");
    h.settle();
    assert!(h.has("compose-save-close"), "the footer offers Save & Close");
    assert!(!h.has("compose-close"), "the old Close button is gone");

    h.click("compose-save-close");
    h.settle();
    assert!(!h.read(|a| a.compose_open()), "the composer closed");
    assert_eq!(
        h.read(|a| a.composer.drafts.len()),
        1,
        "the draft was kept, not deleted"
    );
}

/// Closing a draft tab with its x asks Send / Save & Close / Discard, and the choice acts.
#[gpui_kit::gpui::test]
pub fn closing_a_draft_tab_asks_send_save_or_discard(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.new_draft();
    h.type_text("draft body");
    h.settle();
    let close = ("reader-tab-close", u32::MAX as usize);
    assert!(h.has(close), "the draft tab has a close control");

    h.click(close);
    assert!(h.read(|a| a.dialog_open()), "closing a draft tab asks first");
    let options = h.app.read_with(h.cx, |a, cx| a.dialog_options(cx));
    assert_eq!(
        options,
        vec![
            "Send".to_owned(),
            "Save & Close".to_owned(),
            "Discard".to_owned()
        ],
        "exactly the three choices"
    );

    // Save & Close is the default option, so `enter` takes it (digit keys skip the default).
    h.keys("enter");
    h.settle();
    assert!(!h.read(|a| a.compose_open()), "the choice closed the tab");
    assert_eq!(
        h.read(|a| a.composer.drafts.len()),
        1,
        "Save & Close kept the draft"
    );
}

/// `cmd-shift-backspace` discards: it asks first, then deletes and closes.
#[gpui_kit::gpui::test]
pub fn the_discard_shortcut_asks_and_deletes(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.new_draft();
    h.type_text("throwaway");
    h.settle();

    h.keys("cmd-shift-backspace");
    h.tick();
    assert!(
        h.read(|a| a.dialog_open()),
        "the shortcut asks before deleting"
    );
    h.keys("1");
    h.await_composer("the draft is deleted", |a| a.composer.drafts.is_empty());
    assert!(!h.read(|a| a.compose_open()), "the composer closed");
}
