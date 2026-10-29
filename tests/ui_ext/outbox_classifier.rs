use mail_classifier::judge::{Answer, AnswerValue, QuestionKey, Suggestion};
use crate::harness::{Harness, harness, harness_with, mailbox, msg};
use crate::misc::spam_box;
use crate::snooze::snooze_box;
use gpui_kit::TestAppContext;
use mail_classifier::app::composer_state::LOCAL_DEBOUNCE_SECS;
use mail_classifier::model::MessageId;
use mail_classifier::model::TriageState::*;
use mail_classifier::model::{Location, Tag};
use mail_classifier::outgoing::DraftKind;
use std::path::PathBuf;

/// One JSON message in the `work` account: the shared [`msg`] helper writes personal ones,
/// and the shared fixture has no work mail to compose on.
fn work_msg(id: u32, thread: u32, email: &str, subject: &str, day: u32, state: &str) -> String {
    let name = email.split('@').next().unwrap();
    format!(
        r#"{{"id":{id},"thread_id":{thread},"from_name":"{name}","from_email":"{email}","to":"you@example.com","subject":"{subject}","body":"Just checking in about this.","received":"2026-09-{day:02}T09:00:00Z","state":"{state}","snooze":null,"account":"work"}}"#
    )
}

/// Native mouse focus must remain in compose inputs instead of triggering list actions.
#[gpui_kit::gpui::test]
pub fn composer_clicks_keep_typing_and_formatting_in_the_editor(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, snooze_box());
    let original = h.cursor().expect("message");
    h.new_draft();
    h.click("compose-subject");
    h.type_text("Editable subject");
    h.click("compose-body");
    h.type_text("Hello selected text");
    h.keys("cmd-a cmd-b");
    h.settle();
    assert_eq!(h.draft_fields().2, "Editable subject");
    assert_eq!(h.draft_fields().3, "**Hello selected text**");
    h.keys("cmd-z");
    h.settle();
    assert_eq!(h.draft_fields().3, "Hello selected text");
    h.keys("cmd-shift-v");
    h.keys("s");
    assert_eq!(
        h.state_of(original),
        Inbox,
        "preview does not run list shortcuts"
    );
    h.keys("cmd-shift-v");
    assert_eq!(h.draft_fields().3, "Hello selected text");
}

/// Cmd-Enter submits the complete body without replacing a selected span with a newline.
#[gpui_kit::gpui::test]
pub fn selected_body_text_survives_submit_and_recall(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, snooze_box());
    h.compose_reply("before selected after");
    h.keys("cmd-a cmd-enter");
    h.await_holds(1);
    h.keys("u");
    h.settle();
    assert!(!h.hold_pending());
    assert_eq!(h.draft_fields().3, "before selected after");
    assert!(h.sent_bodies().is_empty());
}
// ---------------------------------------------------------------- The send hold

/// Nothing leaves before the hold runs out. The mock account has a real in-memory backend,
/// so this asserts the ordering rather than a refusal: no sent message exists while the
/// countdown is still running, and the original message is untouched throughout.
#[gpui_kit::gpui::test]
pub fn sending_holds_first_and_nothing_leaves_before_the_countdown(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, snooze_box());
    let id = h.cursor().unwrap();
    h.compose_reply("Does that work?");
    h.settle();
    assert_eq!(
        h.open_drafts(),
        1,
        "the reply is a durable draft, not a modal"
    );
    h.keys("cmd-enter");
    h.await_holds(1);
    assert!(h.hold_pending(), "the send waits out its hold");

    // Nothing is in the mailbox yet: a hold is not a delivery.
    assert_eq!(
        h.read(|a| a.mailbox.messages().iter().filter(|m| m.outgoing).count()),
        0
    );
    assert_eq!(h.state_of(id), Inbox, "sending does not move the original");
    h.advance(mail_classifier::outgoing::HOLD_SECONDS - 1);
    h.tick();
    assert!(
        h.read(|a| a.mailbox.messages().iter().filter(|m| m.outgoing).count()) == 0,
        "still inside the hold, still nothing delivered"
    );
    h.assert_invariant("hold");
}

/// Undo during the hold brings the whole draft back, and nothing was ever sent.
#[gpui_kit::gpui::test]
pub fn undo_during_the_hold_restores_the_draft(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, snooze_box());
    h.compose_reply("still here");
    h.settle();
    h.keys("cmd-enter");
    h.await_holds(1);
    assert!(h.hold_pending());
    h.advance(mail_classifier::outgoing::HOLD_SECONDS - 1);
    h.tick();
    assert!(h.hold_pending(), "still inside the hold");
    h.keys("u");
    h.tick();
    assert!(!h.hold_pending(), "undo ends the hold");
    assert_eq!(
        h.open_drafts(),
        1,
        "the draft is still there, still editable"
    );
    assert_eq!(
        h.draft_fields().3,
        "still here",
        "undo restores what was typed, not an empty draft"
    );
    assert_eq!(
        h.read(|a| a.mailbox.messages().iter().filter(|m| m.outgoing).count()),
        0
    );
    h.assert_invariant("undo hold");
}

/// A second Send while the countdown runs does not send: the hold is the point, and only
/// the separate Send now control ends it early.
#[gpui_kit::gpui::test]
pub fn a_second_submit_does_not_bypass_the_hold(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, snooze_box());
    h.compose_reply("once only");
    h.settle();
    h.keys("cmd-enter");
    h.await_holds(1);
    assert!(h.hold_pending());
    h.keys("cmd-enter");
    h.keys("cmd-enter");
    h.tick();
    assert!(
        h.hold_pending(),
        "submitting again did not skip the countdown"
    );
    assert_eq!(
        h.read(|a| a.mailbox.messages().iter().filter(|m| m.outgoing).count()),
        0
    );
    h.keys("u");
    h.tick();
}

/// Edits settle and are written, then more edits settle and are written again, each against
/// the revision the store handed back rather than the one from before. What the provider
/// finally receives is the newest text, which only happens if each round was accepted
/// instead of refused as stale.
#[gpui_kit::gpui::test]
pub fn every_edit_after_the_first_is_written_in_turn(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, snooze_box());
    h.compose_reply("one");
    h.settle();
    h.type_text(" two");
    h.settle();
    assert_eq!(
        h.draft_fields().3,
        "one two",
        "the second save took the second edit"
    );
    h.type_text(" three");
    h.settle();
    assert_eq!(
        h.draft_fields().3,
        "one two three",
        "and the third took the third"
    );

    h.keys("cmd-enter");
    h.await_holds(1);
    h.deliver_held();
    let bodies = h.sent_bodies();
    assert_eq!(bodies.len(), 1, "one delivery, not one per save");
    assert!(
        bodies[0].contains("one two three"),
        "the send carries every edit in order, not the first draft of them: {bodies:?}"
    );
}

/// Send is usually pressed in the same breath as the last keystroke, so the write of that
/// text is still on its way. The countdown waits for it: a hold that began on the older
/// stored copy would deliver a body the user had already finished rewriting.
#[gpui_kit::gpui::test]
pub fn sending_while_the_last_save_is_still_running_sends_the_newest_text(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, snooze_box());
    h.compose_reply("first pass");
    h.settle();
    // Submit in the same breath as the last edit: the save of "second pass" is still out.
    h.type_text(" second pass");
    h.keys("cmd-enter");
    h.settle();
    h.await_holds(1);
    h.deliver_held();
    let bodies = h.sent_bodies();
    assert_eq!(bodies.len(), 1, "the send went out once");
    assert!(
        bodies[0].contains("second pass"),
        "the newest text was delivered, not the one stored before it: {bodies:?}"
    );
}

/// Two messages sent one after the other are two countdowns at once, and each keeps its own
/// Undo: recalling the first hands that draft back whole and leaves the second counting
/// down untouched.
#[gpui_kit::gpui::test]
pub fn two_holds_count_down_at_once_and_are_recalled_one_at_a_time(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, snooze_box());
    h.compose_reply("first message");
    h.settle();
    let first = h.read(|a| a.inline_composer.expect("the inline reply"));
    // Close the first so the list is driving again, then start a second reply beside it.
    h.keys("escape");
    h.tick();
    h.compose_reply("second message");
    h.settle();
    let second = h.read(|a| a.inline_composer.expect("the second inline reply"));
    assert_ne!(first, second, "each reply is its own draft");
    assert_eq!(
        h.read(|a| a.composer.drafts.len()),
        2,
        "both drafts are held open"
    );

    h.show_draft(first);
    h.send_draft();
    h.await_holds(1);
    h.show_draft(second);
    h.send_draft();
    h.await_holds(2);

    // `u` recalls the countdown that started first, and only that one.
    h.keys("u");
    h.await_holds(1);
    assert_eq!(
        h.held_draft(),
        Some(second),
        "the newer countdown is untouched"
    );
    assert_eq!(h.hold_count(), 1, "exactly one countdown was ended");
    h.show_draft(first);
    assert_eq!(
        h.draft_fields().3,
        "first message",
        "the earlier draft came back whole"
    );
    assert_eq!(
        h.held_draft(),
        Some(second),
        "still the second message that is held"
    );
}

/// A real file for the picker to hand to the worker. Named after the test that wants it, so
/// two tests never share one.
fn temp_file(name: &str, bytes: &[u8]) -> PathBuf {
    let path = std::env::temp_dir().join(format!("mail-classifier-{}-{name}", std::process::id()));
    std::fs::write(&path, bytes).expect("write the temp file");
    path
}

/// A file is read on the worker while the user keeps typing. The form's attachment list is
/// a snapshot from before that import, so merging the stale list back would quietly drop
/// the file the user just picked: both the file and the newer text have to survive.
#[gpui_kit::gpui::test]
pub fn typing_while_a_file_is_attached_keeps_the_file_and_the_newer_text(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, snooze_box());
    h.compose_reply("see the attached");
    h.settle();
    let path = temp_file("attach-race.txt", b"invoice bytes");
    let name = path
        .file_name()
        .expect("file name")
        .to_string_lossy()
        .into_owned();
    h.attach(vec![path.clone()]);
    h.type_text(" and the notes below");
    h.settle();
    h.tick();
    assert_eq!(
        h.draft_attachments(),
        vec![name],
        "the file the worker imported is still on the draft"
    );
    assert!(
        h.draft_fields().3.contains("and the notes below"),
        "and the text typed while it was importing is still there: {:?}",
        h.draft_fields().3
    );
    let _ = std::fs::remove_file(&path);
}

/// Send is often pressed while the draft is still being worked on. Whatever is still in
/// flight has to land before the countdown starts, so the message that eventually goes out
/// is the one on screen: the newest text and the file that was still being imported.
#[gpui_kit::gpui::test]
pub fn sending_while_a_file_is_still_being_attached_sends_the_newest_draft(
    cx: &mut TestAppContext,
) {
    let mut h = harness_with(cx, snooze_box());
    h.compose_reply("first pass");
    h.settle();
    let path = temp_file("send-race.txt", b"receipt bytes");
    h.attach(vec![path.clone()]);
    h.type_text(" second pass");
    h.keys("cmd-enter");
    h.settle();
    h.await_holds(1);
    h.deliver_held();
    let (bodies, files) = h.read(|a| {
        let sent: Vec<_> = a.mailbox.messages().iter().filter(|m| m.outgoing).collect();
        (
            sent.iter().map(|m| m.body.clone()).collect::<Vec<_>>(),
            sent.iter().map(|m| m.attachments.len()).sum::<usize>(),
        )
    });
    assert_eq!(bodies.len(), 1, "the send went out once");
    assert!(
        bodies[0].contains("second pass"),
        "the newest text was delivered, not the copy stored before it: {bodies:?}"
    );
    assert_eq!(files, 1, "and the file went with it");
    let _ = std::fs::remove_file(&path);
}

/// A reply that is only being held has changed nothing: the original keeps its place and
/// its tags, and recalling the send hands the draft back rather than leaving a half-written
/// reply behind.
#[gpui_kit::gpui::test]
pub fn a_held_reply_changes_nothing_until_a_provider_confirms_it(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, snooze_box());
    let id = h.cursor().unwrap();
    let start = h.counts();
    let total = h.total();
    let tags = h.read(|a| a.mailbox.tags(id).to_vec());

    h.compose_reply("thanks je?");
    h.settle();
    assert_eq!(h.counts(), start, "typing a reply triages nothing");

    h.keys("cmd-enter");
    h.await_holds(1);
    assert!(h.sent_bodies().is_empty(), "a hold is not a delivery");
    assert_eq!(h.state_of(id), Inbox, "the original has not moved");
    assert_eq!(
        h.read(|a| a.mailbox.tags(id).to_vec()),
        tags,
        "a hold is not a send, so the original's tags are untouched"
    );
    assert_eq!(h.counts(), start);
    assert_eq!(h.total(), total);

    h.keys("u");
    h.tick();
    assert!(!h.hold_pending(), "the countdown is over");
    assert_eq!(
        h.read(|a| a.composer.drafts.len()),
        1,
        "the draft is back and editable"
    );
    assert_eq!(h.draft_fields().3, "thanks je?", "with what was typed");
    assert_eq!(h.state_of(id), Inbox);
    h.assert_invariant("held reply");
}

// ------------------------------------------------- Prefilled replies and forwards

/// Reply-all names everyone on the thread but the user, and keeps the original's subject:
/// a prefilled draft the user can edit, not a blank one that lost the conversation.
#[gpui_kit::gpui::test]
pub fn reply_all_prefills_everyone_else_and_the_subject(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let id = h.cursor().unwrap();
    let (from, subject) = h.read(|a| {
        let m = a.mailbox.get(id).unwrap();
        (m.from_email.clone(), m.subject.clone())
    });

    h.keys("shift-a");
    h.settle();
    let (kind, to, subj, _) = h.draft_fields();
    assert_eq!(kind, DraftKind::ReplyAll);
    assert!(to.contains(&from), "the sender is on the list, got {to:?}");
    assert!(
        !to.contains("you@example.com"),
        "the user is not replying to themselves"
    );
    assert!(
        subj.starts_with("Re: ") && subj.ends_with(subject.trim_start_matches("Re: ")),
        "the subject is the original's, got {subj:?}"
    );
}

/// A forward carries the original in its body so there is something to forward: the message
/// arrives as text the user can trim, not as a reference they have to find again.
#[gpui_kit::gpui::test]
pub fn forward_prefills_the_original_and_leaves_the_recipient_blank(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let id = h.cursor().unwrap();
    let from = h.read(|a| a.mailbox.get(id).unwrap().from_email.clone());

    h.keys("w");
    h.settle();
    let (kind, to, subj, body) = h.draft_fields();
    assert_eq!(kind, DraftKind::Forward);
    assert!(
        to.is_empty(),
        "a forward is addressed by the user, got {to:?}"
    );
    assert!(
        subj.starts_with("Fwd: ") || subj.to_lowercase().starts_with("fw"),
        "the subject says what this is, got {subj:?}"
    );
    assert!(
        body.contains("Forwarded message") && body.contains(&from),
        "the original is in the body: {body:?}"
    );
}

/// Closing a composer keeps the draft and only an explicit discard removes it: closing an
/// untouched draft is still a close, never a delete.
#[gpui_kit::gpui::test]
pub fn closing_keeps_the_draft_and_discard_removes_it(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, snooze_box());
    h.compose_reply("keep me");
    h.settle();
    let (_, _, subject, body) = h.draft_fields();
    assert!(!body.is_empty());
    h.keys("escape");
    h.tick();
    assert!(!h.read(|a| a.inline_compose_open()), "the surface closed");
    assert_eq!(
        h.read(|a| a.composer.drafts.len()),
        1,
        "the draft is retained after closing"
    );

    // Reopen it from the same durable record, with what was typed.
    let id = h.read(|a| a.composer.drafts[0].record.id);
    h.show_draft(id);
    assert!(
        h.read(|a| a.compose_in_tab()),
        "the retained draft reopens in a tab"
    );
    assert_eq!(
        h.draft_fields().3,
        body,
        "the reopened draft is the one that was written"
    );
    assert_eq!(h.draft_fields().2, subject);

    // Discard asks first, and only the confirmation deletes.
    h.keys("cmd-n");
    h.settle();
    assert_eq!(
        h.read(|a| a.composer.drafts.len()),
        2,
        "the earlier draft is still held"
    );
    h.discard_active_draft();
    assert_eq!(
        h.read(|a| a.composer.drafts.len()),
        1,
        "the discard removed exactly the confirmed draft"
    );
    h.assert_invariant("close");
}

/// Typing does not write on every keystroke: the edit is queued, and one write follows the
/// pause. A burst is still a single pending write, and only when typing stops does it reach
/// the store.
#[gpui_kit::gpui::test]
pub fn typing_debounces_the_local_save_until_the_pause(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, snooze_box());
    h.new_draft();
    h.type_text("one");
    assert_eq!(
        h.pending_saves(),
        1,
        "the keystroke queued a write, not a database transaction"
    );
    h.type_text(" two");
    assert_eq!(h.pending_saves(), 1, "a burst is still one pending write");

    // Nothing has been written yet: the pause has not passed.
    h.advance(LOCAL_DEBOUNCE_SECS - 1);
    assert_eq!(h.pending_saves(), 1, "still within the pause");

    h.settle();
    assert_eq!(h.pending_saves(), 0, "the write followed the pause");
    assert_eq!(h.draft_fields().3, "one two", "and it carried every edit");
}

/// Closing is an explicit save: the pending write goes out at once rather than waiting out
/// the debounce, so the user never loses the seconds since their last keystroke.
#[gpui_kit::gpui::test]
pub fn closing_flushes_the_pending_write_immediately(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, snooze_box());
    h.compose_reply("typed then closed");
    assert_eq!(h.pending_saves(), 1, "the edit is queued, not yet written");

    h.keys("escape");
    h.tick();
    assert!(!h.read(|a| a.inline_compose_open()), "the surface closed");
    h.await_composer("the close wrote the draft", |a| a.composer_idle());
    assert_eq!(h.pending_saves(), 0, "close did not wait out the debounce");

    // Reopen: the text survived without an idle pause.
    let id = h.read(|a| a.composer.drafts[0].record.id);
    h.show_draft(id);
    assert_eq!(h.draft_fields().3, "typed then closed");
}

/// The per-account Drafts row (directly above Sent) opens the Drafts pane in the main area:
/// the account's stored drafts are listed, and Enter reopens the cursor one in the composer.
#[gpui_kit::gpui::test]
pub fn the_sidebar_drafts_row_opens_the_pane_and_reopens_a_draft(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, snooze_box());
    h.compose_reply("from the sidebar");
    h.settle();
    let id = h.read(|a| a.composer.drafts[0].record.id);
    // Close it so the draft is only reachable through the list.
    h.keys("escape");
    h.tick();
    assert!(!h.read(|a| a.inline_compose_open()));

    // The account's Drafts row shows the Drafts pane. The list starts on All, whose panel has
    // no per-account tiles, so pick the account on the rail first.
    h.click("sidebar-rail-personal");
    h.tick();
    assert!(h.has("sidebar-drafts-personal"), "the account has a Drafts row");
    h.click("sidebar-drafts-personal");
    h.tick();
    assert_eq!(
        h.location(),
        Some(Location::Drafts("personal".into())),
        "the Drafts row shows the Drafts pane"
    );
    assert!(h.drafts_pane_open());
    let rows = h.draft_rows();
    assert_eq!(rows.len(), 1, "the account's stored draft is listed");
    assert_eq!(rows[0].0, "personal", "and it is this account's");
    assert!(
        rows[0].3.contains("from the sidebar"),
        "the snippet shows what was written: {rows:?}"
    );

    // A click opens the row, the same as Enter.
    h.click(("draft-row", 0usize));
    h.tick();
    assert_eq!(
        h.read(|a| a.tabs.active_draft()),
        Some(id),
        "the clicked draft opened"
    );
    assert_eq!(h.draft_fields().3, "from the sidebar");
    assert_eq!(
        h.read(|a| a.composer.drafts.len()),
        1,
        "one record, not a copy"
    );
}

/// The Drafts pane is per account: each account's row lists only its own drafts, and the
/// count the sidebar badge uses is that same set.
#[gpui_kit::gpui::test]
pub fn the_drafts_pane_lists_only_that_accounts_drafts(cx: &mut TestAppContext) {
    // The shared fixture is all-personal, so this mailbox gives each account one message to
    // compose on.
    let mb = mailbox(&[
        msg(1, 101, "dana@whitfield.dev", "Personal note", 14, "Inbox"),
        work_msg(2, 202, "ravi@acme.co", "Work note", 14, "Inbox"),
    ]);
    let mut h = harness_with(cx, mb);
    // The list starts on All; an account's tiles appear once its rail tile is picked.
    h.click("sidebar-rail-personal");
    h.click("sidebar-inbox-personal");
    h.new_draft();
    h.type_text("personal draft");
    h.settle();
    let personal = h.read(|a| a.tabs.active_draft().expect("personal draft tab"));
    h.close_draft_tab(personal);
    h.click("sidebar-rail-work");
    h.click("sidebar-inbox-work");
    h.new_draft();
    h.type_text("work draft");
    h.settle();
    let work = h.read(|a| a.tabs.active_draft().expect("work draft tab"));
    h.close_draft_tab(work);

    h.click("sidebar-rail-personal");
    h.click("sidebar-drafts-personal");
    h.tick();
    assert_eq!(h.location(), Some(Location::Drafts("personal".into())));
    let rows = h.draft_rows();
    assert_eq!(rows.len(), 1, "personal's own draft only");
    assert_eq!(rows[0].0, "personal");
    assert!(rows[0].3.contains("personal draft"), "{rows:?}");
    assert_eq!(
        h.read(|a| a.account_draft_count("personal")),
        1,
        "the badge counts the same set"
    );

    h.click("sidebar-rail-work");
    h.click("sidebar-drafts-work");
    h.tick();
    assert_eq!(h.location(), Some(Location::Drafts("work".into())));
    let rows = h.draft_rows();
    assert_eq!(rows.len(), 1, "work's own draft only");
    assert_eq!(rows[0].0, "work");
    assert!(rows[0].3.contains("work draft"), "{rows:?}");
    assert_eq!(h.read(|a| a.account_draft_count("work")), 1);
    assert_eq!(
        h.read(|a| a.account_draft_count("personal")),
        1,
        "the other account's draft is untouched"
    );
}

/// A search narrows the Drafts pane to the drafts it matches (subject/recipients/body) while
/// staying in the folder; clearing the filter restores every draft.
#[gpui_kit::gpui::test]
pub fn a_search_narrows_the_drafts_pane_and_clearing_restores_it(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.click("sidebar-rail-personal");
    h.click("sidebar-inbox-personal");
    h.new_draft();
    h.type_text("budget notes");
    h.settle();
    let first = h.read(|a| a.tabs.active_draft().expect("first draft tab"));
    h.close_draft_tab(first);
    h.advance(60);
    h.click("sidebar-inbox-personal");
    h.new_draft();
    h.type_text("lunch plans");
    h.settle();
    let second = h.read(|a| a.tabs.active_draft().expect("second draft tab"));
    h.close_draft_tab(second);

    h.click("sidebar-drafts-personal");
    h.tick();
    assert_eq!(h.draft_rows().len(), 2, "both drafts are listed");

    // Searching from the pane keeps the folder and adds the term as a filter.
    h.keys("/");
    h.type_text("budget");
    h.keys("enter");
    h.tick();
    assert_eq!(
        h.location(),
        Some(Location::Drafts("personal".into())),
        "the search stays in the Drafts folder"
    );
    let rows = h.draft_rows();
    assert_eq!(rows.len(), 1, "the query narrows the list");
    assert!(rows[0].3.contains("budget notes"), "{rows:?}");

    // Clearing the filter (quick-filter Clear) brings every draft back.
    h.keys("1");
    h.tick();
    assert_eq!(h.draft_rows().len(), 2, "clearing restores every draft");
    assert_eq!(h.location(), Some(Location::Drafts("personal".into())));
}

/// The Drafts pane shows the account's drafts newest edit first, and deleting the cursor one
/// through the same confirmation the composer uses removes it from the list.
#[gpui_kit::gpui::test]
pub fn drafts_are_newest_first_and_discard_removes_them(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.click("sidebar-rail-personal");
    h.click("sidebar-inbox-personal");
    h.new_draft();
    h.type_text("older draft");
    h.settle();
    let first = h.read(|a| a.tabs.active_draft().expect("first draft tab"));
    h.close_draft_tab(first);
    // A later edit makes the second draft the newer one.
    h.advance(60);
    h.click("sidebar-inbox-personal");
    h.new_draft();
    h.type_text("newer draft");
    h.settle();
    let second = h.read(|a| a.tabs.active_draft().expect("second draft tab"));
    h.close_draft_tab(second);

    h.click("sidebar-drafts-personal");
    h.tick();
    let rows = h.draft_rows();
    assert_eq!(rows.len(), 2, "both drafts are listed");
    assert!(
        rows[0].3.contains("newer draft") && rows[1].3.contains("older draft"),
        "newest edit first: {rows:?}"
    );

    // `d` deletes the cursor draft after the same confirmation the composer asks for.
    h.keys("d");
    h.tick();
    assert!(h.read(|a| a.modal_open()), "the discard is confirmed first");
    h.keys("1");
    h.await_composer("the draft is gone", |a| a.draft_rows().len() == 1);
    let rows = h.draft_rows();
    assert_eq!(rows.len(), 1, "the discarded draft left the list");
    assert!(rows[0].3.contains("older draft"), "{rows:?}");
    assert_eq!(h.read(|a| a.account_draft_count("personal")), 1);
}

/// Expanding moves the *same* draft into the shared tab strip: one record, one entity, so
/// nothing typed can be lost by moving it and collapsing it back.
#[gpui_kit::gpui::test]
pub fn expanding_moves_the_same_draft_between_inline_and_a_tab(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, snooze_box());
    h.compose_reply("moving draft");
    h.settle();
    let id = h.read(|a| a.inline_composer.expect("inline"));
    assert_eq!(
        h.read(|a| a.tabs.draft_ids().len()),
        0,
        "an inline reply has no tab yet"
    );

    h.expand_active_draft();
    assert!(
        h.read(|a| a.compose_in_tab()),
        "the draft is in the tab strip"
    );
    assert_eq!(h.read(|a| a.inline_composer), None, "and no longer inline");
    assert_eq!(
        h.read(|a| a.composer.drafts.len()),
        1,
        "still exactly one draft"
    );
    assert_eq!(h.draft_fields().3, "moving draft");

    h.collapse_active_draft();
    assert_eq!(
        h.read(|a| a.inline_composer),
        Some(id),
        "the same draft came back under its thread"
    );
    assert_eq!(h.read(|a| a.composer.drafts.len()), 1);
    assert_eq!(h.draft_fields().3, "moving draft");
}

/// Several drafts stay open at once, each with its own content, and closing one does not
/// touch the others.
#[gpui_kit::gpui::test]
pub fn several_drafts_stay_open_independently(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, snooze_box());
    h.new_draft();
    h.type_text("first draft");
    h.settle();
    let first = h.read(|a| a.tabs.active_draft().expect("draft tab"));
    assert_eq!(h.draft_fields().3, "first draft");

    h.new_draft();
    h.type_text("second draft");
    h.settle();
    let second = h.read(|a| a.tabs.active_draft().expect("draft tab"));
    assert_ne!(first, second, "a new message is its own draft");
    assert_eq!(h.draft_fields().3, "second draft");
    assert_eq!(
        h.read(|a| a.composer.drafts.len()),
        2,
        "both drafts are retained"
    );

    h.show_draft(first);
    assert_eq!(
        h.draft_fields().3,
        "first draft",
        "the earlier draft kept its own body"
    );
    h.show_draft(second);
    assert_eq!(h.draft_fields().3, "second draft");

    // Closing one surface leaves the other draft's content exactly as it was.
    h.close_draft_tab(second);
    assert_eq!(
        h.read(|a| a.composer.drafts.len()),
        2,
        "a closed draft is retained, not dropped"
    );
    h.show_draft(first);
    assert_eq!(
        h.draft_fields().3,
        "first draft",
        "the other draft kept its own body"
    );
}

/// A closed draft is only useful if there is a way back to it: `cmd-shift-d` opens the
/// Drafts pane, and Enter reopens that same record rather than a fresh blank one.
#[gpui_kit::gpui::test]
pub fn a_closed_draft_reopens_from_the_drafts_pane(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, snooze_box());
    h.compose_reply("kept for later");
    h.settle();
    let id = h.read(|a| a.composer.drafts[0].record.id);
    h.keys("escape");
    h.tick();
    assert!(
        !h.read(|a| a.compose_in_tab()) && !h.read(|a| a.inline_compose_open()),
        "escape closes the surface; the draft is kept, not shown"
    );

    h.keys("cmd-shift-d");
    h.tick();
    assert!(h.drafts_pane_open(), "cmd-shift-d opens the Drafts pane");
    assert_eq!(
        h.read(|a| a.composer.drafts.len()),
        1,
        "the pane lists the stored draft"
    );
    h.keys("enter");
    h.tick();
    assert_eq!(
        h.read(|a| a.tabs.active_draft()),
        Some(id),
        "the same draft reopened"
    );
    assert_eq!(
        h.draft_fields().3,
        "kept for later",
        "with what was written, not a blank draft"
    );
    assert_eq!(
        h.read(|a| a.composer.drafts.len()),
        1,
        "one record, not a copy"
    );
}

/// An empty Drafts pane says so rather than showing nothing at all.
#[gpui_kit::gpui::test]
pub fn the_drafts_pane_says_when_there_is_nothing_to_reopen(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, snooze_box());
    h.keys("cmd-shift-d");
    h.tick();
    assert!(
        h.drafts_pane_open(),
        "the keyboard still reaches the Drafts pane"
    );
    assert!(
        h.draft_rows().is_empty(),
        "an empty account has no drafts to list"
    );
    assert!(
        h.header().contains("Drafts"),
        "the header names the pane: {}",
        h.header()
    );
}

// ---------------------------------------------------------------- Classifier

pub fn tag_total(h: &mut Harness<'_>) -> usize {
    h.read(|a| {
        a.mailbox
            .messages()
            .iter()
            .map(|m| a.mailbox.tags(m.id).len())
            .sum()
    })
}

pub fn pending_total(h: &mut Harness<'_>) -> usize {
    h.read(|a| {
        a.mailbox
            .messages()
            .iter()
            .map(|m| a.mailbox.pending(m.id).len())
            .sum()
    })
}

#[gpui_kit::gpui::test]
pub fn classifier_runs_on_startup(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    assert!(
        tag_total(&mut h) + pending_total(&mut h) > 0,
        "startup classification produces badges"
    );
    h.assert_invariant("classify");
}

/// A message with a folder suggestion waiting, seeded by hand: the offline stub never files
/// mail, so it is the only way a suggestion pends.
fn seed_folder_suggestion(h: &mut Harness<'_>) -> MessageId {
    let id = h.read(|a| a.visible_ids()[0]);
    h.app.update(h.cx, |a, _| {
        let options = a.mailbox.folder_options("personal");
        a.mailbox.add_suggestions(vec![Suggestion {
            message: id,
            key: QuestionKey::Folder,
            answer: Answer {
                probabilities: vec![1.0; options.len() + 1],
                value: AnswerValue::Choice(0),
                confidence: 0.9,
            },
        }]);
    });
    id
}

#[gpui_kit::gpui::test]
pub fn accept_applies_and_reject_discards_pending_suggestions(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let id = seed_folder_suggestion(&mut h);
    let before_state = h.state_of(id);
    let tags_before = h.read(|a| a.mailbox.tags(id).len());
    assert!(!h.read(|a| a.mailbox.pending(id).is_empty()));

    h.keys("y");
    assert!(h.read(|a| a.mailbox.pending(id).is_empty()), "accept clears the pending badges");
    assert_ne!(h.state_of(id), before_state, "accepting the folder suggestion files it");
    h.assert_invariant("accept");

    // One undo step brings the pending badge back.
    h.keys("u");
    assert!(!h.read(|a| a.mailbox.pending(id).is_empty()));
    assert_eq!(h.state_of(id), before_state);

    // Reject discards without applying.
    h.keys("n");
    assert!(h.read(|a| a.mailbox.pending(id).is_empty()));
    assert_eq!(h.state_of(id), before_state);
    assert_eq!(h.read(|a| a.mailbox.tags(id).len()), tags_before);
}

#[gpui_kit::gpui::test]
pub fn background_labels_are_not_undoable(cx: &mut TestAppContext) {
    // spam_box's message 1 is auto-applied `PossibleSpam` during startup classification.
    let mut h = harness_with(cx, spam_box());
    assert!(h.has_tag(1, Tag::PossibleSpam), "startup auto-applies a label");

    // The user's own action: archive the cursor message.
    let id = h.cursor().unwrap();
    h.keys("e");
    assert_eq!(h.state_of(id), Archived, "the user archived the message");

    // `u` undoes the archive; the background label was never on the undo stack.
    h.keys("u");
    assert_eq!(h.state_of(id), Inbox, "u undoes the user's action");
    assert!(h.has_tag(1, Tag::PossibleSpam), "u does not undo the background label");
    h.assert_invariant("undo background label");
}
