//! Composer UI behaviour: the Collapse affordance, where collapsing lands in the thread,
//! and the inline Cc/Bcc reveal.

use crate::harness::{harness, harness_with, mailbox, msg};
use gpui_kit::TestAppContext;

/// Expand and Collapse lost their ⋯ menu; the palette is their home now, and both move the
/// same draft between the tab strip and the thread it answers.
#[gpui_kit::gpui::test]
pub fn expand_and_collapse_run_from_the_palette(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.compose_reply("from a thread");
    h.expand_active_draft();
    assert!(h.read(|a| a.compose_in_tab()), "the reply is in a tab");
    h.collapse_active_draft();
    assert!(
        h.read(|a| a.inline_compose_open()),
        "an expanded reply collapses back under its thread"
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

    assert!(h.has("compose-show-cc"), "a ghost `cc` word is offered");
    assert!(h.has("compose-show-bcc"), "a ghost `bcc` word is offered");
    assert!(!h.has("compose-cc"), "an empty Cc row is hidden");
    assert!(!h.has("compose-bcc"), "an empty Bcc row is hidden");

    h.click("compose-show-cc");
    assert!(h.has("compose-cc"), "clicking Cc reveals its row");
    assert!(!h.has("compose-show-cc"), "the ghost word gives way to its clause");
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
    assert!(!h.has("compose-show-bcc"), "the ghost word gives way to its clause");
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

/// Escape closes the composer and keeps the draft (a close never deletes).
#[gpui_kit::gpui::test]
pub fn closing_flushes_and_keeps_the_draft(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.new_draft();
    h.type_text("keep me");
    h.settle();
    assert!(!h.has("compose-save-close"), "the footer no longer has Save & Close");

    h.keys("escape");
    h.settle();
    assert!(!h.read(|a| a.compose_open()), "the composer closed");
    assert_eq!(
        h.read(|a| a.composer.drafts.len()),
        1,
        "the draft was kept, not deleted"
    );
}

/// The formatting bar starts hidden, ⌘⌥T shows it, and the choice outlives the composer.
#[gpui_kit::gpui::test]
pub fn the_formatting_bar_toggles_and_is_remembered(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.new_draft();
    assert!(!h.has("compose-bold"), "the bar starts hidden");
    h.keys("cmd-alt-t");
    assert!(h.has("compose-bold"), "cmd-alt-t shows it");
    h.keys("escape");
    h.settle();
    h.new_draft();
    assert!(h.has("compose-bold"), "a new composer remembers the choice");
    h.keys("cmd-alt-t");
    assert!(!h.has("compose-bold"), "cmd-alt-t hides it again");
}

/// A lone `/` on a line opens the slash menu; more typing or Escape dismisses it.
#[gpui_kit::gpui::test]
pub fn a_slash_opens_the_menu_and_typing_or_escape_dismisses_it(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.new_draft();
    h.type_text("/");
    assert!(h.has("compose-slash-menu"), "a lone slash opens the menu");
    h.type_text("x");
    assert!(!h.has("compose-slash-menu"), "continued typing dismisses it");
    h.keys("backspace backspace");
    h.type_text("/");
    assert!(h.has("compose-slash-menu"));
    h.keys("escape");
    assert!(!h.has("compose-slash-menu"), "escape dismisses it");
    assert!(h.read(|a| a.compose_open()), "and does not close the composer");
}

/// Down and Enter drive the slash menu instead of moving the caret. The menu mirrors the
/// bar (every command plus Send later): Up from the first row wraps to Send later, which
/// opens the intent menu, and picking Bold by click makes the same edit its chord does.
#[gpui_kit::gpui::test]
pub fn the_slash_menu_is_driven_from_the_keyboard(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.new_draft();
    h.type_text("hi\n");
    h.type_text("/");
    // Every bar command, then the menu-only Send later.
    for ix in 0usize..=11 {
        assert!(h.has(("compose-slash", ix)), "slash row {ix} is missing");
    }
    assert!(h.has("compose-slash-later"), "slash row 12 is missing");
    h.keys("up enter");
    assert!(!h.has("compose-slash-menu"), "picking closes the menu");
    assert!(h.has("compose-intent-menu"), "Send later opens the intent menu");
    h.keys("escape");
    h.settle();
    let body = h.read(|a| {
        let id = a.tabs.active_draft().expect("a draft tab");
        a.composer.record(id).expect("record").body.source.clone()
    });
    assert_eq!(body, "hi\n", "the slash was removed and nothing else typed");

    h.type_text("/");
    h.keys("enter");
    assert!(!h.has("compose-slash-menu"), "Enter on Bold closes the menu");
    assert!(!h.has("compose-intent-menu"));
    let body = h.read(|a| {
        let id = a.tabs.active_draft().expect("a draft tab");
        a.composer.record(id).expect("record").body.source.clone()
    });
    assert_eq!(body, "hi\n****", "Bold wraps the empty caret like its chord does");
}

/// Mentioning an attachment with none attached warns in the status line, and Send says so.
#[gpui_kit::gpui::test]
pub fn an_unattached_attachment_mention_warns_and_changes_the_send_label(
    cx: &mut TestAppContext,
) {
    let mut h = harness(cx);
    h.new_draft();
    assert!(h.has("compose-send-label") && !h.has("compose-warning"));
    h.type_text("I've attached the deck.");
    assert!(h.has("compose-warning"), "the warning shows");
    assert!(h.has("compose-send-anyway"), "Send reads Send anyway");
    assert!(!h.has("compose-send-label"));
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
