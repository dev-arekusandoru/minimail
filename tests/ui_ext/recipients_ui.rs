//! The recipient chips: autocomplete, the blurred `+N` line and the reply-all dropped note.

use crate::harness::{Harness, harness_with, mailbox, msg};
use gpui_kit::{AppContext as _, TestAppContext};

/// Put the caret in the open draft's To input.
fn focus_to(h: &mut Harness<'_>) {
    let window = h.window;
    h.cx
        .update_window(window, |_, window, cx| {
            h.app.update(cx, |a, cx| {
                let id = a.tabs.active_draft().or(a.inline_composer).expect("draft");
                a.composers
                    .get(&id)
                    .expect("composer")
                    .update(cx, |c, cx| c.focus_recipients(window, cx));
            });
        })
        .expect("window alive");
    h.cx.run_until_parked();
}

#[gpui_kit::gpui::test]
pub fn typing_a_prefix_suggests_and_enter_commits_a_chip(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox(&[msg(1, 1, "jonas@example.com", "Hi", 1, "Inbox")]));
    h.new_draft();
    focus_to(&mut h);
    h.type_text("jon");
    assert!(h.has(("compose-suggest", 0usize)), "the prefix lists a suggestion");
    h.keys("enter");
    h.settle();
    assert_eq!(h.draft_fields().1, "jonas <jonas@example.com>");
}

#[gpui_kit::gpui::test]
pub fn a_blurred_field_collapses_to_a_plus_n_chip(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox(&[msg(1, 1, "alice@example.com", "Hi", 1, "Inbox")]));
    h.new_draft();
    focus_to(&mut h);
    for i in 0..14 {
        h.type_text(&format!("someone-with-a-long-name-{i}@example.com,"));
    }
    assert!(!h.has(("compose-more", 0usize)), "a focused field shows every chip");
    h.keys("tab");
    h.settle();
    assert!(h.has(("compose-more", 0usize)), "blurred, the overflow shows as +N");
}

#[gpui_kit::gpui::test]
pub fn removing_a_reply_all_recipient_shows_a_note_and_restore_brings_them_back(
    cx: &mut TestAppContext,
) {
    let m = r#"{"id":1,"thread_id":1,"from_name":"alice","from_email":"alice@example.com","to":"raj@example.com, maya@example.com","subject":"Plan","body":"hi","received":"2026-09-01T09:00:00Z","state":"Inbox","snooze":null}"#;
    let mut h = harness_with(cx, mailbox(&[m.to_owned()]));
    h.goto(1);
    h.keys("shift-a");
    h.await_composer("reply all opens inline", |a| a.inline_compose_open());
    h.settle();
    assert!(!h.has("compose-dropped-note"), "nobody dropped yet");
    let before = h.draft_fields().1;
    assert!(before.contains("maya@example.com"), "{before}");

    focus_to(&mut h);
    h.keys("backspace backspace");
    h.settle();
    assert!(!h.draft_fields().1.contains("maya@example.com"));
    assert!(h.has("compose-dropped-note"), "the removal is noted");

    h.click("compose-dropped-restore");
    h.settle();
    assert!(h.draft_fields().1.contains("maya@example.com"), "Restore re-adds them");
    assert!(!h.has("compose-dropped-note"));
}
