//! The Jev classifier's background worker, driven headlessly through fake judges.
//!
//! No network and no keychain: the harness injects a judge (or none), and the fake judges
//! answer from the offline [`StubJudge`] while counting their calls.

use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use gpui_kit::TestAppContext;
use mail_classifier::contacts::ContactStore;
use mail_classifier::judge::{Answer, Judge, JudgeError, JudgeStatus, Question, QuestionKey, StubJudge};
use mail_classifier::model::Tag;
use serde_json::Value;

use crate::harness::{
    Harness, harness, harness_no_judge, harness_with_judge, harness_with_judge_prefs, mailbox, msg,
};

/// A [`StubJudge`] that counts its calls and can be made to fail on demand.
struct FakeJudge {
    inner: StubJudge,
    model: String,
    calls: Arc<AtomicUsize>,
    fail: Arc<AtomicBool>,
}

impl FakeJudge {
    fn new(model: &str) -> (Self, Arc<AtomicUsize>, Arc<AtomicBool>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let fail = Arc::new(AtomicBool::new(false));
        let judge = FakeJudge {
            inner: StubJudge,
            model: model.to_owned(),
            calls: calls.clone(),
            fail: fail.clone(),
        };
        (judge, calls, fail)
    }
}

impl Judge for FakeJudge {
    fn judge(
        &self,
        state: &Value,
        questions: &[(QuestionKey, Question)],
    ) -> Result<Vec<(QuestionKey, Answer)>, JudgeError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
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
fn installing_a_judge_classifies_once_and_the_second_pass_hits_the_cache(cx: &mut TestAppContext) {
    let (judge, calls, _fail) = FakeJudge::new("counting");
    let mut h = harness_with_judge(cx, spam_message(), Some(Arc::new(judge)));
    assert_eq!(calls.load(Ordering::SeqCst), 1, "one request for the one Inbox message");
    assert!(classified(&mut h), "the answers were routed and applied");
    assert!(h.has_tag(1, Tag::PossibleSpam), "the spam label auto-applied");

    h.keys("c");
    assert_eq!(calls.load(Ordering::SeqCst), 1, "the second pass replays the cache");
    assert_eq!(h.read(|a| a.judge_status().clone()), JudgeStatus::Idle);
}

#[gpui_kit::gpui::test]
fn a_failure_shows_in_the_status_and_the_next_success_clears_it(cx: &mut TestAppContext) {
    let (judge, _calls, fail) = FakeJudge::new("counting");
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
        JudgeStatus::Idle | JudgeStatus::Working(_)
    ), "a success clears the error");
}

#[gpui_kit::gpui::test]
fn judgements_are_cached_in_sqlite_across_a_restart(cx: &mut TestAppContext) {
    let store = Rc::new(ContactStore::open_in_memory().expect("contacts store"));
    let (judge, calls, _fail) = FakeJudge::new("counting");
    let h = harness_with_judge_prefs(cx, spam_message(), store.clone(), Some(Arc::new(judge)));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    drop(h);

    // A second app over the same database: the stored answers are read back, not re-fetched.
    let (judge, calls, _fail) = FakeJudge::new("counting");
    let mut h = harness_with_judge_prefs(cx, spam_message(), store, Some(Arc::new(judge)));
    assert_eq!(calls.load(Ordering::SeqCst), 0, "the stored judgement is reused");
    assert!(h.has_tag(1, Tag::PossibleSpam));
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
