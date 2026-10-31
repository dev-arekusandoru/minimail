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
        let id = a.inline_draft().expect("inline reply");
        a.composer.record(id).expect("record").cc.clone()
    });
    assert!(!cc.trim().is_empty(), "reply-all prefilled a Cc");
    assert!(h.has("compose-cc"), "a prefilled Cc is shown");
    assert!(
        !h.has("compose-show-cc"),
        "there is nothing to reveal for a field already shown"
    );
}

/// Closing keeps the draft (a close never deletes): escape no longer closes, so the
/// palette's Close composer is the keys' way out.
#[gpui_kit::gpui::test]
pub fn closing_flushes_and_keeps_the_draft(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.new_draft();
    h.type_text("keep me");
    h.settle();
    assert!(!h.has("compose-save-close"), "the footer no longer has Save & Close");

    h.run_palette("Close composer");
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
    h.run_palette("Close composer");
    h.settle();
    h.new_draft();
    assert!(h.has("compose-bold"), "a new composer remembers the choice");
    h.keys("cmd-alt-t");
    assert!(!h.has("compose-bold"), "cmd-alt-t hides it again");
}

/// ⌘⇧V shows the preview and the same key brings the editor back.
#[gpui_kit::gpui::test]
pub fn the_preview_key_toggles_both_ways(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.new_draft();
    h.type_text("hello");
    h.keys("cmd-shift-v");
    assert!(h.has("compose-preview"), "cmd-shift-v shows the preview");
    h.keys("cmd-shift-v");
    assert!(!h.has("compose-preview"), "cmd-shift-v again returns to the editor");
    h.type_text(" again");
    h.settle();
    assert_eq!(h.draft_fields().3, "hello again", "and the body takes typing again");
}

/// Escape in a recipient field closes its suggestions and moves typing to the body.
#[gpui_kit::gpui::test]
pub fn escape_in_a_recipient_field_moves_to_the_body(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.new_draft();
    h.click("compose-to");
    h.type_text("someone");
    h.keys("escape");
    assert!(h.read(|a| a.compose_open()), "the composer stays open");
    h.type_text("body text");
    h.settle();
    assert_eq!(h.draft_fields().3, "body text", "typing lands in the body");
}

/// A `/` opens the menu at the start of a word — the body's start or after whitespace —
/// and the text after it filters the rows. Escape or whitespace dismisses it.
#[gpui_kit::gpui::test]
pub fn a_slash_opens_the_menu_at_a_word_start(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.new_draft();
    h.type_text("/");
    assert!(h.has("compose-slash-menu"), "a lone slash opens the menu");
    h.keys("escape");
    assert!(!h.has("compose-slash-menu"), "escape dismisses it");
    assert!(h.read(|a| a.compose_open()), "and does not close the composer");

    h.new_draft();
    h.type_text("hello /bo");
    assert!(h.has("compose-slash-menu"), "a slash after a space opens mid-line");
    h.type_text(" ");
    assert!(!h.has("compose-slash-menu"), "a space ends the query");
}

/// A slash inside a word (`and/or`, a URL) is left as text.
#[gpui_kit::gpui::test]
pub fn a_slash_inside_a_word_does_not_open_the_menu(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.new_draft();
    h.type_text("and/or");
    assert!(!h.has("compose-slash-menu"), "`and/or` is not a slash command");
    h.type_text(" https://example.com");
    assert!(!h.has("compose-slash-menu"), "nor is a URL's slash");
}

/// The query after the `/` filters the bar's commands, and Enter runs the match.
#[gpui_kit::gpui::test]
pub fn a_slash_query_filters_and_enter_applies_it(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.new_draft();
    h.type_text("hello /bo");
    assert!(h.has("compose-slash-menu"), "the menu stays up while the query is typed");
    assert!(h.has(("compose-slash", 0usize)), "Bold ranks first for `bo`");
    assert!(
        !h.has(("compose-slash", 2usize)),
        "only the fuzzy matches are offered"
    );
    assert!(!h.has("compose-slash-later"), "Send later is not a row");

    h.keys("enter");
    assert!(!h.has("compose-slash-menu"), "picking closes the menu");
    let body = h.read(|a| {
        let id = a.tabs.active_draft().expect("a draft tab");
        a.composer.record(id).expect("record").body.source.clone()
    });
    assert_eq!(body, "hello ****", "Bold applies and the `/bo` span is gone");
}

/// Down and Enter drive the slash menu instead of moving the caret. The menu is exactly
/// the bar's commands, so Down from Bold lands on Italic.
#[gpui_kit::gpui::test]
pub fn the_slash_menu_is_driven_from_the_keyboard(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.new_draft();
    h.type_text("hi\n/");
    for ix in 0usize..=11 {
        assert!(h.has(("compose-slash", ix)), "slash row {ix} is missing");
    }
    h.keys("down enter");
    assert!(!h.has("compose-slash-menu"), "picking closes the menu");
    let body = h.read(|a| {
        let id = a.tabs.active_draft().expect("a draft tab");
        a.composer.record(id).expect("record").body.source.clone()
    });
    assert_eq!(body, "hi\n**", "Italic wraps the empty caret like its chord does");
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

/// An inline reply belongs to its own thread: opening another conversation leaves it there,
/// and coming back shows the same draft again. Nothing is parked in a draft tab.
#[gpui_kit::gpui::test]
pub fn an_inline_reply_stays_attached_to_its_thread(cx: &mut TestAppContext) {
    let msgs = vec![
        msg(1, 1, "alice@example.com", "Thread A", 10, "Inbox"),
        msg(2, 2, "bob@example.com", "Thread B", 11, "Inbox"),
    ];
    let mut h = harness_with(cx, mailbox(&msgs));
    h.goto(1);
    h.keys("enter");
    h.compose_reply("answer in A");
    h.settle();
    let id = h.read(|a| a.inline_draft()).expect("the reply is inline in A");
    assert!(
        h.has("reader-reply-card"),
        "the reply renders as a card at the bottom of the thread"
    );
    assert!(
        h.read(|a| a.tabs.draft_ids().is_empty()),
        "an inline reply has no tab"
    );

    h.click(("row", 2usize));
    h.tick();
    assert_eq!(h.read(|a| a.inline_draft()), None, "B has no reply of its own");
    assert!(
        h.read(|a| a.tabs.draft_ids().is_empty()),
        "leaving A did not move its draft into a tab"
    );

    h.click(("row", 1usize));
    h.tick();
    assert_eq!(
        h.read(|a| a.inline_draft()),
        Some(id),
        "the same draft is back under A"
    );
    assert!(
        h.has("reader-reply-card"),
        "and it renders as a card again"
    );
    assert!(
        h.read(|a| a.tabs.draft_ids().is_empty()),
        "still no draft tab"
    );
}

/// Reopening a closed reply brings back the conversation it answers, with the reply inline
/// under it — even when that thread's tab was closed too.
#[gpui_kit::gpui::test]
pub fn reopening_a_reply_opens_its_original_thread_inline(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.compose_reply("kept for later");
    h.settle();
    let id = h.read(|a| a.composer.drafts[0].record.id);
    let thread = h
        .read(|a| a.tabs.active().and_then(|tab| tab.thread_id()))
        .expect("the reply's thread is open");

    h.run_palette("Close composer");
    h.tick();
    // Close the original's tab as well, so reopening has to bring it back.
    h.click(("reader-tab-close", thread as usize));
    h.tick();
    assert!(
        !h.read(|a| a.compose_in_tab()) && !h.read(|a| a.inline_compose_open()),
        "closing the thread also closed its reply surface"
    );
    assert!(
        h.read(|a| a.tabs.tabs().is_empty()),
        "the original's tab is gone"
    );

    h.keys("cmd-shift-d");
    h.tick();
    h.keys("enter");
    h.tick();
    assert_eq!(
        h.read(|a| a.inline_draft()),
        Some(id),
        "the reply reopened inline under its original"
    );
    assert_eq!(
        h.read(|a| a.tabs.active_draft()),
        None,
        "not in a draft tab"
    );
    assert!(
        !h.read(|a| a.tabs.tabs().is_empty()),
        "its original's thread opened"
    );
    assert!(h.has("reader-reply-card"), "the reply card is rendered");
    assert_eq!(
        h.draft_fields().3,
        "kept for later",
        "with what was written"
    );
}

/// A forward is not a reply: reopening it keeps its own tab, as a new message does.
#[gpui_kit::gpui::test]
pub fn a_forward_reopens_in_a_tab(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.keys("w");
    h.await_composer("the forward opens", |a| a.compose_open());
    h.type_text("forwarded body");
    h.settle();
    let id = h.read(|a| a.inline_draft()).expect("the forward is inline");
    h.run_palette("Close composer");
    h.tick();

    h.keys("cmd-shift-d");
    h.tick();
    h.keys("enter");
    h.tick();
    assert_eq!(
        h.read(|a| a.tabs.active_draft()),
        Some(id),
        "the forward reopened in a tab"
    );
    assert_eq!(
        h.read(|a| a.inline_draft()),
        None,
        "a forward has no thread to sit under"
    );
}

/// A lone message has no rail, so its reply card sits below it with a gap.
#[gpui_kit::gpui::test]
pub fn the_reply_card_renders_below_a_lone_message(cx: &mut TestAppContext) {
    let msgs = vec![msg(1, 1, "alice@example.com", "Alone", 10, "Inbox")];
    let mut h = harness_with(cx, mailbox(&msgs));
    h.goto(1);
    h.keys("enter");
    h.compose_reply("reply to the lone message");
    h.settle();
    assert!(
        h.has("reader-reply-card"),
        "the reply card renders below a lone message"
    );
}
