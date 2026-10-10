//! The AI and Senders settings pages, driven through the real settings window.

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::TestAppContext;
use mail_classifier::clock::DAY;
use mail_classifier::judge::{
    Answer, Confidence, Judge, JudgeError, JudgeStatus, Mode, Question, QuestionKey, StubJudge,
};
use serde_json::Value;

#[path = "ui_ext/harness.rs"]
#[allow(dead_code)]
mod harness;
use harness::{Harness, harness_with, harness_with_judge, mailbox, msg};

/// Sidebar page indexes: Accounts, Appearance, Inbox, Reader, AI, Senders.
const AI: usize = 4;
const SENDERS: usize = 5;

fn spam(h: &mut Harness<'_>) -> Mode {
    h.read(|a| a.policy.mode(QuestionKey::Spam))
}

/// Off is the third segment of every row.
fn set_spam_mode(h: &mut Harness<'_>, segment: usize) {
    // Group 2 is "Tagging"; its first row is Spam.
    h.settings_click_in(2, 0, format!("ai-mode-Spam-{segment}"));
}

#[gpui_kit::gpui::test]
fn turning_spam_off_stops_it_producing_suggestions(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox(&[msg(1, 1, "deals@talentloop.com", "Is this a fit?", 1, "Inbox")]));
    h.settings_open_page(AI);
    set_spam_mode(&mut h, 2);
    h.keys("c");
    assert!(
        h.read(|a| a.mailbox.pending(1).is_empty()),
        "Off suggests nothing"
    );

    set_spam_mode(&mut h, 1);
    assert_eq!(spam(&mut h), Mode::Review);
    h.keys("c");
    assert_eq!(
        h.read(|a| a.mailbox.pending(1).iter().map(|s| s.key).collect::<Vec<_>>()),
        vec![QuestionKey::Spam],
        "the same check does suggest when it is not off"
    );
}

#[gpui_kit::gpui::test]
fn the_confidence_row_picks_how_sure_auto_has_to_be(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox(&[msg(1, 1, "alice@example.com", "Question", 1, "Inbox")]));
    h.settings_open_page(AI);
    assert_eq!(spam(&mut h), Mode::Auto(Confidence::High), "the shipped default");

    // Row 1 of "Tagging" is Spam; row 2 is its confidence, shown while Auto.
    h.settings_pick_option(2, 1, 2);
    assert_eq!(spam(&mut h), Mode::Auto(Confidence::Low));

    set_spam_mode(&mut h, 2);
    assert_eq!(spam(&mut h), Mode::Off);

    h.keys("cmd-,");
    h.settings_open_page(AI);
    assert_eq!(spam(&mut h), Mode::Off, "the policy outlives the row that is hidden");
    set_spam_mode(&mut h, 0);
    assert_eq!(spam(&mut h), Mode::Auto(Confidence::Medium), "back to Auto, at the middle preset");
}

#[gpui_kit::gpui::test]
fn the_filter_narrows_the_blocked_senders(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox(&[msg(1, 1, "alice@example.com", "Question", 1, "Inbox")]));
    h.app.update(h.cx, |a, _| {
        a.mailbox.block_sender("spammer@example.com", None, harness::NOON - 4 * DAY);
        a.mailbox.block_sender("boss@example.com", None, harness::NOON - DAY);
    });
    h.settings_open_page(SENDERS);

    h.settings_click("blocked-filter");
    h.settings_type("spammer");
    assert!(h.settings_has("blocked-unblock-spammer@example.com"));
    assert!(!h.settings_has("blocked-unblock-boss@example.com"), "the filter hides the rest");

    h.settings_keys("cmd-a");
    h.settings_type("nobody");
    assert!(
        !h.settings_has("blocked-unblock-spammer@example.com"),
        "a filter nothing matches leaves no rows"
    );
}

#[gpui_kit::gpui::test]
fn unblocking_a_sender_then_undoing_restores_it_with_its_block_time(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox(&[msg(1, 1, "spammer@example.com", "Deal", 1, "Inbox")]));
    let blocked_at = harness::NOON - 3 * DAY;
    h.app.update(h.cx, |a, _| {
        a.mailbox.block_sender("spammer@example.com", None, blocked_at);
    });
    h.settings_open_page(SENDERS);
    assert!(h.settings_has("blocked-unblock-spammer@example.com"));

    h.settings_click("blocked-unblock-spammer@example.com");
    assert!(h.read(|a| a.mailbox.blocked().is_empty()), "Unblock removes the sender");
    assert!(h.toast().contains("undo"), "the toast offers Undo: {}", h.toast());

    h.keys("u");
    assert_eq!(
        h.read(|a| a.mailbox.blocked()),
        vec![("spammer@example.com".to_owned(), blocked_at)],
        "undo restores the block, with the time it was blocked"
    );
}

// ------------------------------------------------------------------ Jev connection

/// A judge that always fails, to exercise the Test connection error path.
struct FailingJudge;

impl Judge for FailingJudge {
    fn judge(
        &self,
        _: &Value,
        _: &[(QuestionKey, Question)],
    ) -> Result<Vec<(QuestionKey, Answer)>, JudgeError> {
        Err(JudgeError::Unavailable("no connection".into()))
    }
}

/// A mailbox whose single message the [`StubJudge`] labels `Possible spam`, so a working
/// classifier visibly classifies it.
fn spam_mailbox() -> mail_classifier::model::Mailbox {
    mailbox(&[msg(1, 1, "deals@talentloop.com", "Is this a fit?", 1, "Inbox")])
}

fn judge_factory(judge: Arc<dyn Judge + Send + Sync>) -> mail_classifier::judge::JudgeFactory {
    Rc::new(move |_, _, _| judge.clone())
}

/// Whether message 1 carries a classifier label (a tag or a pending suggestion).
fn classified(h: &mut Harness<'_>) -> bool {
    h.read(|a| !a.mailbox.tags(1).is_empty() || !a.mailbox.pending(1).is_empty())
}

/// Install `factory` and start from no judge, as a fresh app with no key is.
fn unclassified(cx: &mut TestAppContext, factory: mail_classifier::judge::JudgeFactory) -> Harness<'_> {
    let h = harness_with_judge(cx, spam_mailbox(), None);
    h.app.update(h.cx, |a, _| a.set_judge_factory(factory));
    h
}

/// Type `text` into the field `id`, replacing what it holds.
fn fill(h: &mut Harness<'_>, id: &'static str, text: &str) {
    h.settings_click(id);
    h.settings_keys("cmd-a");
    h.settings_type(text);
}

/// The inline error and test result the open settings panel is showing.
fn panel_jev(h: &mut Harness<'_>) -> (Option<String>, Option<mail_classifier::app::settings::JevTest>) {
    let panel = h.read(|a| a.settings_panel()).expect("settings open");
    panel.read_with(h.cx, |panel, _| {
        (panel.jev_error_text().map(str::to_owned), panel.jev_test_result().cloned())
    })
}

#[gpui_kit::gpui::test]
fn saving_a_key_installs_a_judge_and_classifies_the_inbox(cx: &mut TestAppContext) {
    let mut h = unclassified(cx, judge_factory(Arc::new(StubJudge)));
    assert_eq!(h.read(|a| a.judge_status().clone()), JudgeStatus::NoKey);

    h.settings_open_page(AI);
    fill(&mut h, "jev-key", "sk-test");
    h.settings_click("jev-save");

    assert!(
        matches!(h.read(|a| a.judge_status().clone()), JudgeStatus::Idle | JudgeStatus::Working(_)),
        "Save installs the judge the factory builds"
    );
    assert!(classified(&mut h), "and the Inbox backfill classifies");
    assert!(h.settings_has("jev-remove-key"), "the panel now says a key is saved");
}

#[gpui_kit::gpui::test]
fn removing_the_key_turns_classification_off(cx: &mut TestAppContext) {
    let mut h = unclassified(cx, judge_factory(Arc::new(StubJudge)));
    h.settings_open_page(AI);
    fill(&mut h, "jev-key", "sk-test");
    h.settings_click("jev-save");
    assert!(classified(&mut h));

    h.settings_click("jev-remove-key");
    assert_eq!(
        h.read(|a| a.judge_status().clone()),
        JudgeStatus::NoKey,
        "Remove key takes the classifier away"
    );
    assert!(
        !h.settings_has("jev-remove-key"),
        "and the row no longer offers to remove a key there is not"
    );
}

#[gpui_kit::gpui::test]
fn a_blank_key_field_leaves_the_saved_key_alone(cx: &mut TestAppContext) {
    let mut h = unclassified(cx, judge_factory(Arc::new(StubJudge)));
    h.settings_open_page(AI);
    fill(&mut h, "jev-key", "sk-test");
    h.settings_click("jev-save");
    assert!(h.settings_has("jev-remove-key"));

    // Save again with the (cleared) key field blank: the endpoint is applied, the key stays.
    h.settings_click("jev-save");
    assert!(
        matches!(h.read(|a| a.judge_status().clone()), JudgeStatus::Idle | JudgeStatus::Working(_)),
        "a blank field does not remove the key"
    );
    assert!(h.settings_has("jev-remove-key"));
}

#[gpui_kit::gpui::test]
fn a_bad_base_url_is_rejected_and_saves_nothing(cx: &mut TestAppContext) {
    let mut h = unclassified(cx, judge_factory(Arc::new(StubJudge)));
    h.settings_open_page(AI);
    fill(&mut h, "jev-key", "sk-test");
    fill(&mut h, "jev-base", "ftp://not-a-url");
    h.settings_click("jev-save");

    let (error, _) = panel_jev(&mut h);
    assert!(
        error.as_deref().is_some_and(|e| e.contains("Base URL")),
        "the bad URL is reported inline: {error:?}"
    );
    assert_eq!(
        h.read(|a| a.judge_status().clone()),
        JudgeStatus::NoKey,
        "an invalid endpoint saves nothing"
    );
}

#[gpui_kit::gpui::test]
fn a_test_connection_reports_the_model_without_saving(cx: &mut TestAppContext) {
    let mut h = unclassified(cx, judge_factory(Arc::new(StubJudge)));
    h.settings_open_page(AI);
    fill(&mut h, "jev-key", "sk-test");
    h.settings_click("jev-test");

    let (_, result) = panel_jev(&mut h);
    assert_eq!(
        result,
        Some(mail_classifier::app::settings::JevTest::Connected("stub".into())),
        "it names the model the test connected with"
    );
    assert_eq!(
        h.read(|a| a.judge_status().clone()),
        JudgeStatus::NoKey,
        "a test connection saves nothing"
    );
}

#[gpui_kit::gpui::test]
fn a_failing_test_connection_shows_the_error_and_saves_nothing(cx: &mut TestAppContext) {
    let mut h = unclassified(cx, judge_factory(Arc::new(FailingJudge)));
    h.settings_open_page(AI);
    fill(&mut h, "jev-key", "sk-test");
    h.settings_click("jev-test");

    let (_, result) = panel_jev(&mut h);
    assert!(
        matches!(&result, Some(mail_classifier::app::settings::JevTest::Failed(e)) if e.contains("no connection")),
        "the failure is shown inline: {result:?}"
    );
    assert_eq!(
        h.read(|a| a.judge_status().clone()),
        JudgeStatus::NoKey,
        "a failing test saves nothing"
    );
}
