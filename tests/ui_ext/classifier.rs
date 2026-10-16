//! The Jev classifier's background worker, driven headlessly through fake judges.
//!
//! No network and no keychain: the harness injects a judge (or none), and the fake judges
//! answer from the offline [`StubJudge`] while counting their calls.

use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use gpui_kit::TestAppContext;
use mail_classifier::contacts::ContactStore;
use mail_classifier::judge::{
    Answer, Judge, JudgeError, JudgePolicy, JudgeStatus, Question, QuestionKey, StubJudge,
    incoming_questions, legacy_judgement_key, message_state,
};
use mail_classifier::model::Tag;
use parking_lot::Mutex;
use serde_json::Value;

use crate::harness::{
    Harness, harness, harness_no_judge, harness_with_judge, harness_with_judge_prefs, mailbox, msg,
};

/// A [`StubJudge`] that counts its calls, records the questions asked, and can be made to fail
/// on demand.
struct FakeJudge {
    inner: StubJudge,
    model: String,
    calls: Arc<AtomicUsize>,
    fail: Arc<AtomicBool>,
    seen: Arc<Mutex<HashSet<QuestionKey>>>,
}

impl FakeJudge {
    #[allow(clippy::type_complexity)]
    fn new(model: &str) -> (Self, Arc<AtomicUsize>, Arc<AtomicBool>, Arc<Mutex<HashSet<QuestionKey>>>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let fail = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(Mutex::new(HashSet::new()));
        let judge = FakeJudge {
            inner: StubJudge,
            model: model.to_owned(),
            calls: calls.clone(),
            fail: fail.clone(),
            seen: seen.clone(),
        };
        (judge, calls, fail, seen)
    }
}

impl Judge for FakeJudge {
    fn judge(
        &self,
        state: &Value,
        questions: &[(QuestionKey, Question)],
    ) -> Result<Vec<(QuestionKey, Answer)>, JudgeError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen.lock().extend(questions.iter().map(|(k, _)| *k));
        if self.fail.load(Ordering::SeqCst) {
            return Err(JudgeError::Unavailable("boom".into()));
        }
        self.inner.judge(state, questions)
    }

    fn model(&self) -> &str {
        &self.model
    }
}

/// A message the stub judge labels `Possible spam` with high confidence, so it auto-applies.
fn spam_message() -> mail_classifier::model::Mailbox {
    mailbox(&[msg(1, 1, "deals@talentloop.com", "Is this a fit?", 1, "Inbox")])
}

/// Whether message 1 was classified: it carries a tag or has a pending suggestion.
fn classified(h: &mut Harness<'_>) -> bool {
    let (tags, pending) = h.read(|a| (a.mailbox.tags(1).len(), a.mailbox.pending(1).len()));
    tags > 0 || pending > 0
}

#[gpui_kit::gpui::test]
fn no_key_means_no_labels_and_the_status_asks_for_one(cx: &mut TestAppContext) {
    let mut h = harness_no_judge(cx);
    assert_eq!(h.read(|a| a.judge_status().clone()), JudgeStatus::NoKey);
    assert!(
        h.read(|a| a.mailbox.messages().iter().all(|m| a.mailbox.tags(m.id).is_empty())),
        "no classifier means no labels"
    );
}

#[gpui_kit::gpui::test]
fn installing_a_judge_classifies_each_inbox_message_once(cx: &mut TestAppContext) {
    let (judge, calls, _fail, _seen) = FakeJudge::new("counting");
    let mut h = harness_with_judge(cx, spam_message(), Some(Arc::new(judge)));
    assert_eq!(calls.load(Ordering::SeqCst), 1, "one request for the one Inbox message");
    assert!(classified(&mut h), "the answers were routed and applied");
    assert!(h.has_tag(1, Tag::PossibleSpam), "the spam label auto-applied");
}

#[gpui_kit::gpui::test]
fn reader_classify_button_disappears_after_answers_are_cached(cx: &mut TestAppContext) {
    let mut h = harness_with_judge(cx, spam_message(), None);
    h.keys("enter");
    assert!(h.has(("btn-classify", 1_usize)));

    h.set_judge(Some(Arc::new(StubJudge)));
    h.keys("escape");
    h.keys("enter");
    assert!(!h.has(("btn-classify", 1_usize)));
}

#[gpui_kit::gpui::test]
fn c_re_asks_jev_about_the_selected_message_even_when_cached(cx: &mut TestAppContext) {
    let (judge, calls, _fail, _seen) = FakeJudge::new("counting");
    let mut h = harness_with_judge(cx, spam_message(), Some(Arc::new(judge)));
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    h.keys("c");
    assert_eq!(calls.load(Ordering::SeqCst), 2, "manual classify skips the cache");
}

#[gpui_kit::gpui::test]
fn shift_c_asks_first_then_re_asks_every_message_in_the_list(cx: &mut TestAppContext) {
    let (judge, calls, _fail, _seen) = FakeJudge::new("counting");
    let two = mailbox(&[
        msg(1, 1, "deals@talentloop.com", "Is this a fit?", 1, "Inbox"),
        msg(2, 2, "alice@example.com", "Lunch?", 2, "Inbox"),
    ]);
    let mut h = harness_with_judge(cx, two, Some(Arc::new(judge)));
    assert_eq!(calls.load(Ordering::SeqCst), 2);

    h.keys("shift-c");
    h.keys("2");
    assert_eq!(calls.load(Ordering::SeqCst), 2, "Cancel sends nothing");

    h.keys("shift-c");
    h.keys("enter");
    assert_eq!(calls.load(Ordering::SeqCst), 4, "Confirm re-asks both messages");
}

#[gpui_kit::gpui::test]
fn a_failure_shows_in_the_status_and_the_next_success_clears_it(cx: &mut TestAppContext) {
    let (judge, _calls, fail, _seen) = FakeJudge::new("counting");
    fail.store(true, Ordering::SeqCst);
    let mut h = harness_with_judge(cx, spam_message(), Some(Arc::new(judge)));
    assert!(
        matches!(h.read(|a| a.judge_status().clone()), JudgeStatus::Error(_)),
        "the failure is shown"
    );
    assert!(!classified(&mut h), "a failed classification leaves no labels");

    // The next pass retries; flip the judge to succeed first.
    fail.store(false, Ordering::SeqCst);
    h.keys("c");
    assert!(classified(&mut h), "the retry classifies");
    assert!(matches!(
        h.read(|a| a.judge_status().clone()),
        JudgeStatus::Idle | JudgeStatus::Working { .. }
    ), "a success clears the error");
}

#[gpui_kit::gpui::test]
fn judgements_are_cached_in_sqlite_across_a_restart(cx: &mut TestAppContext) {
    let store = Rc::new(ContactStore::open_in_memory().expect("contacts store"));
    let (judge, calls, _fail, _seen) = FakeJudge::new("counting");
    let h = harness_with_judge_prefs(cx, spam_message(), store.clone(), Some(Arc::new(judge)));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    drop(h);

    // A second app over the same database: the stored answers are read back, not re-fetched.
    let (judge, calls, _fail, _seen) = FakeJudge::new("counting");
    let mut h = harness_with_judge_prefs(cx, spam_message(), store, Some(Arc::new(judge)));
    assert_eq!(calls.load(Ordering::SeqCst), 0, "the stored judgement is reused");
    assert!(h.has_tag(1, Tag::PossibleSpam));
}

#[gpui_kit::gpui::test]
fn a_stored_result_survives_a_model_change(cx: &mut TestAppContext) {
    let store = Rc::new(ContactStore::open_in_memory().expect("contacts store"));
    let (judge, calls, _fail, _seen) = FakeJudge::new("model-a");
    let mut h = harness_with_judge_prefs(cx, spam_message(), store, Some(Arc::new(judge)));
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    // A different model must not re-bill a message that already has a stored result.
    let (judge, calls_b, _fail, _seen) = FakeJudge::new("model-b");
    h.set_judge(Some(Arc::new(judge)));
    assert_eq!(calls_b.load(Ordering::SeqCst), 0, "a model change never re-bills");
    assert!(h.has_tag(1, Tag::PossibleSpam));
}

#[gpui_kit::gpui::test]
fn a_stored_result_survives_removing_and_re_adding_the_key(cx: &mut TestAppContext) {
    let store = Rc::new(ContactStore::open_in_memory().expect("contacts store"));
    let (judge, calls, _fail, _seen) = FakeJudge::new("counting");
    let mut h = harness_with_judge_prefs(cx, spam_message(), store, Some(Arc::new(judge)));
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    h.set_judge(None); // key removed
    assert_eq!(h.read(|a| a.judge_status().clone()), JudgeStatus::NoKey);

    let (judge, calls_b, _fail, _seen) = FakeJudge::new("counting");
    h.set_judge(Some(Arc::new(judge))); // key re-added
    assert_eq!(calls_b.load(Ordering::SeqCst), 0, "a re-added key never re-bills");
    assert!(h.has_tag(1, Tag::PossibleSpam));
}

/// A temporary database path, so a test can seed a legacy row from a second connection.
fn temp_db(tag: &str) -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("mail-classifier-jev-{}-{tag}.db", std::process::id()));
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
    path
}

#[gpui_kit::gpui::test]
fn a_legacy_hash_row_is_carried_over_to_the_message_id(cx: &mut TestAppContext) {
    let path = temp_db("carry-over");
    let store = Rc::new(ContactStore::open(&path).expect("contacts store"));
    // Seed a legacy row keyed by the old content hash, exactly as the pre-MIGRATION_4 code did.
    let message = spam_message().get(1).cloned().expect("message 1");
    let key = legacy_judgement_key("jev-latest", &message);
    let questions: Vec<(QuestionKey, Question)> = incoming_questions(&JudgePolicy::default(), false, &[])
        .into_iter()
        .map(|s| (s.key, s.question))
        .collect();
    let answers = StubJudge.judge(&message_state(&message, ""), &questions).expect("stub answers");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute(
        "INSERT INTO judgements_legacy (key, answers, judged_at) VALUES (?1, ?2, ?3)",
        rusqlite::params![key, serde_json::to_string(&answers).unwrap(), 0],
    )
    .unwrap();
    drop(conn);

    // The app copies it onto message 1 at startup and re-asks only the changed questions.
    let (judge, calls, _fail, seen) = FakeJudge::new("counting");
    let mut h = harness_with_judge_prefs(cx, spam_message(), store, Some(Arc::new(judge)));
    assert_eq!(calls.load(Ordering::SeqCst), 1, "one request fills the stale/missing questions");
    let seen = seen.lock().clone();
    assert!(seen.contains(&QuestionKey::Spam), "the reworded Spam question is re-asked");
    assert!(
        !seen.contains(&QuestionKey::NeedsReply) && !seen.contains(&QuestionKey::Kind),
        "the carried-over NeedsReply/Kind answers are reused"
    );
    assert!(h.has_tag(1, Tag::PossibleSpam), "the legacy answer was routed");

    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
}

#[gpui_kit::gpui::test]
fn a_reply_expecting_an_answer_is_tagged_awaiting(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let id = h.cursor().unwrap();
    h.keys("r");
    h.keys("t h a n k s space j e space ?");
    h.keys("cmd-enter");
    assert!(h.has_tag(id, Tag::AwaitingReply), "the worker tagged the thread");
}

#[gpui_kit::gpui::test]
fn without_a_judge_a_reply_is_not_tagged(cx: &mut TestAppContext) {
    let mut h = harness_no_judge(cx);
    let id = h.cursor().unwrap();
    h.keys("r");
    h.keys("t h a n k s space j e space ?");
    h.keys("cmd-enter");
    assert!(!h.has_tag(id, Tag::AwaitingReply), "no key means no tag");
}
