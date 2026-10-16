//! The AI and Senders settings pages, driven through the real settings window.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::TestAppContext;
use mail_classifier::clock::DAY;
use mail_classifier::contacts::ContactStore;
use mail_classifier::judge::{
    Answer, Confidence, Judge, JudgeError, JudgeStatus, Mode, Question, QuestionKey, StubJudge,
};
use mail_classifier::model::Tag;
use serde_json::Value;

#[path = "ui_ext/harness.rs"]
#[allow(dead_code)]
mod harness;
use harness::{Harness, harness_with, harness_with_judge, harness_with_judge_prefs, harness_with_prefs, mailbox, msg};

/// Sidebar page indexes: Accounts, Appearance, Inbox, Reader, AI, Senders.
const AI: usize = 4;
const SENDERS: usize = 5;

fn spam(h: &mut Harness<'_>) -> Mode {
    h.read(|a| a.policy.mode(QuestionKey::Spam))
}

/// The Off segment of a non-folder question's row.
fn set_spam_mode(h: &mut Harness<'_>, segment: usize) {
    // Group 2 is "Tagging"; its first row is Spam.
    h.settings_click_in(2, 0, format!("ai-mode-Spam-{segment}"));
}

#[gpui_kit::gpui::test]
fn toggling_auto_classify_reaches_the_app_and_is_stored(cx: &mut TestAppContext) {
    let store = Rc::new(ContactStore::open_in_memory().expect("in-memory address book"));
    let scope = mail_classifier::prefs::Scope::Global;
    let mut h = harness_with_prefs(
        cx,
        mailbox(&[msg(1, 1, "alice@example.com", "Question", 1, "Inbox")]),
        store.clone(),
    );
    h.settings_open_page(AI);
    let before = mail_classifier::app_settings::AUTO_CLASSIFY.get(store.as_ref(), &scope);

    // Tagging's switches follow the question rows: item 5 is "Classify new mail automatically".
    h.settings_scroll(2, 0, "ai-mode-Spam-0", -600.);
    h.settings_click_in(2, 5, "check");
    assert_eq!(
        mail_classifier::app_settings::AUTO_CLASSIFY.get(store.as_ref(), &scope),
        !before,
        "the switch reaches the app and is stored without a restart"
    );
}

#[gpui_kit::gpui::test]
fn the_folder_row_offers_only_off_and_review(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox(&[msg(1, 1, "alice@example.com", "Question", 1, "Inbox")]));
    h.settings_open_page(AI);
    assert!(h.settings_has("ai-mode-Folder-off"), "the Folder row offers Off");
    assert!(h.settings_has("ai-mode-Folder-review"), "and Review");
    assert!(!h.settings_has("ai-mode-Folder-auto"), "never Auto");

    // The Folder row is item 4 of Tagging; clicking Off turns the question off.
    h.settings_scroll(2, 0, "ai-mode-Spam-0", -600.);
    h.settings_click_in(2, 4, "ai-mode-Folder-off");
    assert_eq!(h.read(|a| a.policy.mode(QuestionKey::Folder)), Mode::Off);
}

#[gpui_kit::gpui::test]
fn turning_spam_off_stops_it_labelling(cx: &mut TestAppContext) {
    // Start with no judge, so nothing is labelled before the Spam row is turned off.
    let mut h = harness_with_judge(
        cx,
        mailbox(&[msg(1, 1, "deals@talentloop.com", "Is this a fit?", 1, "Inbox")]),
        None,
    );
    h.settings_open_page(AI);
    set_spam_mode(&mut h, 1);
    assert_eq!(spam(&mut h), Mode::Off);

    h.set_judge(Some(harness::stub_judge()));
    assert!(!h.has_tag(1, Tag::PossibleSpam), "Off labels nothing");

    set_spam_mode(&mut h, 0);
    assert_eq!(spam(&mut h), Mode::Auto(Confidence::High), "back to the shipped preset");
    h.keys("c");
    assert!(
        h.has_tag(1, Tag::PossibleSpam),
        "the same check labels the message when the question is not off"
    );
}

/// A label question is applied as soon as it arrives, so its row offers only Auto and Off.
#[gpui_kit::gpui::test]
fn a_non_folder_row_offers_only_auto_and_off(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox(&[msg(1, 1, "alice@example.com", "Question", 1, "Inbox")]));
    h.settings_open_page(AI);
    assert_eq!(spam(&mut h), Mode::Auto(Confidence::High), "the shipped default");
    assert!(h.settings_has("ai-mode-Spam-0"), "the Auto segment");
    assert!(h.settings_has("ai-mode-Spam-1"), "and Off");
    assert!(!h.settings_has("ai-mode-Spam-2"), "there is no third segment");

    set_spam_mode(&mut h, 1);
    assert_eq!(spam(&mut h), Mode::Off);
    h.keys("cmd-,");
    h.settings_open_page(AI);
    assert_eq!(spam(&mut h), Mode::Off, "the policy outlives the panel");
    set_spam_mode(&mut h, 0);
    assert_eq!(spam(&mut h), Mode::Auto(Confidence::High), "back to Auto, at the shipped preset");
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
        matches!(h.read(|a| a.judge_status().clone()), JudgeStatus::Idle | JudgeStatus::Working { .. }),
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
        matches!(h.read(|a| a.judge_status().clone()), JudgeStatus::Idle | JudgeStatus::Working { .. }),
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

#[test]
fn settings_events_never_debug_the_jev_key() {
    use mail_classifier::app::settings::{JevKey, SettingsEvent};
    let events = [
        SettingsEvent::SaveJev {
            key: JevKey::new("sk-live-secret".into()),
            base: "https://api.typesafe.ai/v1".into(),
            model: "jev-latest".into(),
        },
        SettingsEvent::TestJev {
            key: JevKey::new("sk-live-secret".into()),
            base: "https://api.typesafe.ai/v1".into(),
            model: "jev-latest".into(),
        },
    ];
    for event in events {
        let debug = format!("{event:?}");
        assert!(!debug.contains("sk-live-secret"), "the key leaked: {debug}");
        assert!(debug.contains("<redacted>"), "the key is not marked: {debug}");
    }
}

#[gpui_kit::gpui::test]
fn a_plain_http_remote_base_url_is_rejected(cx: &mut TestAppContext) {
    let mut h = unclassified(cx, judge_factory(Arc::new(StubJudge)));
    h.settings_open_page(AI);
    fill(&mut h, "jev-key", "sk-test");
    fill(&mut h, "jev-base", "http://api.typesafe.ai/v1");
    h.settings_click("jev-save");

    let (error, _) = panel_jev(&mut h);
    assert!(
        error.as_deref().is_some_and(|e| e.contains("Base URL")),
        "plain HTTP to a remote host is rejected: {error:?}"
    );
    assert_eq!(
        h.read(|a| a.judge_status().clone()),
        JudgeStatus::NoKey,
        "an insecure endpoint saves nothing"
    );
}

/// A key store whose deletion always fails, to exercise the Remove key failure path.
struct NoDeleteSecrets;
impl mail_classifier::provider::secrets::SecretStore for NoDeleteSecrets {
    fn get(&self, _: &str) -> Option<String> {
        Some("sk-test".into())
    }
    fn set(&self, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn delete(&self, _: &str) -> Result<(), String> {
        Err("keychain busy".into())
    }
}

#[gpui_kit::gpui::test]
fn a_failed_key_removal_stops_the_judge_but_keeps_the_key(cx: &mut TestAppContext) {
    let mut h = unclassified(cx, judge_factory(Arc::new(StubJudge)));
    h.app.update(h.cx, |a, _| {
        a.set_secrets(Rc::new(NoDeleteSecrets));
    });
    h.settings_open_page(AI);
    fill(&mut h, "jev-key", "sk-test");
    h.settings_click("jev-save");
    assert!(
        matches!(h.read(|a| a.judge_status().clone()), JudgeStatus::Idle | JudgeStatus::Working { .. }),
        "Save installs the judge"
    );

    h.settings_click("jev-remove-key");
    assert_eq!(
        h.read(|a| a.judge_status().clone()),
        JudgeStatus::NoKey,
        "removal stops classification even if the keychain refuses"
    );
    let (error, _) = panel_jev(&mut h);
    assert!(
        error.as_deref().is_some_and(|e| e.contains("Could not remove")),
        "the failed deletion is reported: {error:?}"
    );
    assert!(
        h.settings_has("jev-remove-key"),
        "the panel still shows the key as saved after a failed deletion"
    );
}

/// A factory that records the endpoint and model it was built with.
fn recording_factory(seen: Rc<RefCell<Vec<(String, String)>>>) -> mail_classifier::judge::JudgeFactory {
    Rc::new(move |base, model, _key| {
        seen.borrow_mut().push((base.to_owned(), model.to_owned()));
        Arc::new(StubJudge)
    })
}

#[gpui_kit::gpui::test]
fn reset_all_rebuilds_the_judge_with_the_default_endpoint(cx: &mut TestAppContext) {
    use mail_classifier::judge::jev::{DEFAULT_BASE_URL, DEFAULT_MODEL};
    let seen = Rc::new(RefCell::new(Vec::new()));
    let store = Rc::new(ContactStore::open_in_memory().expect("in-memory address book"));
    let mut h = harness_with_judge_prefs(cx, spam_mailbox(), store, None);
    h.app.update(h.cx, |a, _| {
        a.set_judge_factory(recording_factory(seen.clone()));
    });
    h.settings_open_page(AI);
    fill(&mut h, "jev-key", "sk-test");
    fill(&mut h, "jev-base", "https://example.test/v1");
    fill(&mut h, "jev-model", "custom-1");
    h.settings_click("jev-save");
    assert_eq!(
        seen.borrow().last(),
        Some(&("https://example.test/v1".to_owned(), "custom-1".to_owned())),
        "Save builds the judge from the fields"
    );

    h.settings_click("settings-reset-all");
    h.settings_keys("enter");
    assert_eq!(
        seen.borrow().last(),
        Some(&(DEFAULT_BASE_URL.to_owned(), DEFAULT_MODEL.to_owned())),
        "Reset all rebuilds the judge on the shipped defaults"
    );
}
