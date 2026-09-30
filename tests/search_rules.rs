use mail_classifier::judge::{Answer, AnswerValue, QuestionKey, Suggestion};
use mail_classifier::model::{Mailbox, Message, TriageState};
use mail_classifier::rules::{Rule, RuleBook};
use mail_classifier::search::Query;

fn msg(name: &str, email: &str, subject: &str, body: &str, received: &str) -> Message {
    Message {
        id: 1,
        thread_id: 1,
        from_name: name.into(),
        from_email: email.into(),
        to: "me@x.com".into(),
        subject: subject.into(),
        body: body.into(),
        received: received.into(),
        state: TriageState::Inbox,
        account: "personal".into(),
        outgoing: false,
        snooze: None,
    }
}

fn m() -> Message {
    msg(
        "Alice Smith",
        "alice@Acme.com",
        "Quarterly Report",
        "Please review the budget numbers",
        "2026-09-14T08:12:00Z",
    )
}

fn mailbox(mut message: Message, state: TriageState) -> Mailbox {
    message.state = state;
    Mailbox::from_json(&serde_json::to_string(&[message]).unwrap()).unwrap()
}
fn hit(q: &str) -> bool {
    let message = m();
    Query::parse(q).matches(&message, &mailbox(message.clone(), TriageState::Inbox))
}

#[test]
fn from_matches_name_or_email_case_insensitively() {
    assert!(hit("from:alice"));
    assert!(hit("from:SMITH"));
    assert!(hit("from:acme.com"));
    assert!(!hit("from:bob"));
}

#[test]
fn subject_only_searches_subject() {
    assert!(hit("subject:quarterly"));
    assert!(!hit("subject:budget"));
    assert!(hit("subject:\"quarterly report\""));
}

#[test]
fn new_state_operators_and_removed_operators() {
    let q = |s: &str| Query::parse(s);
    for (query, state) in [
        ("is:inbox", TriageState::Inbox),
        ("is:archived", TriageState::Archived),
        ("is:filed", TriageState::Filed(4)),
        ("is:deleted", TriageState::Deleted),
    ] {
        let message = m();
        assert!(q(query).matches(&message, &mailbox(message.clone(), state)));
    }
    let message = m();
    let mut snoozed = mailbox(message.clone(), TriageState::Inbox);
    snoozed.snooze(&[message.id], 30, 0);
    assert!(q("is:snoozed").matches(&message, &snoozed));
    let message = m();
    assert!(!q("is:waiting").matches(&message, &mailbox(message.clone(), TriageState::Inbox)));
    assert!(!q("is:later").matches(&message, &mailbox(message.clone(), TriageState::Inbox)));
    assert!(!q("is:done").matches(&message, &mailbox(message.clone(), TriageState::Inbox)));
    assert!(!q("is:screener").matches(&message, &mailbox(message.clone(), TriageState::Inbox)));
}

#[test]
fn sent_operator_uses_outgoing_status() {
    let mut sent = m();
    sent.outgoing = true;
    assert!(Query::parse("is:sent").matches(&sent, &mailbox(sent.clone(), TriageState::Inbox)));
    let ordinary = m();
    assert!(
        !Query::parse("is:sent").matches(&ordinary, &mailbox(ordinary.clone(), TriageState::Inbox))
    );
    assert!(!Query::parse("is:sent").matches(&sent, &mailbox(sent.clone(), TriageState::Deleted)));
}

#[test]
fn tag_operators_match_mailbox_tags() {
    let m = m();
    let mut mailbox = mailbox(m.clone(), TriageState::Inbox);
    mailbox.apply_auto(
        Suggestion {
            message: m.id,
            key: QuestionKey::Spam,
            answer: Answer {
                probabilities: vec![0.01, 0.99],
                value: AnswerValue::Bool(true),
                confidence: 0.99,
            },
        },
        0,
    );
    assert!(Query::parse("tag:spam").matches(&m, &mailbox));
    assert!(!Query::parse("tag:needs-reply").matches(&m, &mailbox));
}

#[test]
fn date_boundaries_before_exclusive_after_inclusive() {
    assert!(!hit("before:2026-09-14"));
    assert!(hit("before:2026-09-15"));
    assert!(hit("after:2026-09-14"));
    assert!(!hit("after:2026-09-15"));
    assert!(hit("after:2026-09-01 before:2026-10-01"));
    assert!(!hit("after:2026-09-15 before:2026-10-01"));
}

#[test]
fn free_text_covers_subject_body_sender() {
    assert!(hit("budget"));
    assert!(hit("QUARTERLY"));
    assert!(hit("acme"));
    assert!(hit("\"review the budget\""));
    assert!(!hit("\"budget review\""));
}

#[test]
fn terms_are_anded() {
    assert!(hit("from:alice is:inbox budget after:2026-09-01"));
    assert!(!hit("from:alice is:done"));
    assert!(!hit("from:alice budget nonexistent"));
}

#[test]
fn unknown_or_malformed_keys_are_free_text() {
    assert!(!hit("foo:bar"));
    assert!(!hit("is:bogus"));
    assert!(!hit("before:soon"));
    assert!(!hit("before:2026-13-01"));
    let mm = msg(
        "A",
        "a@x.com",
        "note",
        "see foo:bar here",
        "2026-01-01T00:00:00Z",
    );
    assert!(Query::parse("foo:bar").matches(&mm, &mailbox(mm.clone(), TriageState::Inbox)));
}

#[test]
fn empty_query_matches_everything_and_slash_is_ignored() {
    assert!(hit(""));
    assert!(hit("/"));
    assert!(hit("/budget"));
    assert!(!hit("/nothing"));
}

#[test]
fn is_search_detection() {
    assert!(Query::is_search("/foo"));
    assert!(Query::is_search("hello from:bob"));
    assert!(Query::is_search("is:inbox"));
    assert!(!Query::is_search("foo:bar"));
    assert!(!Query::is_search("archive all"));
    assert!(!Query::is_search("is:"));
    assert!(!Query::is_search("\"quoted\""));
}

fn r(s: &str, st: TriageState) -> Rule {
    Rule {
        sender: s.into(),
        state: st,
    }
}

#[test]
fn suggests_on_second_identical_action_only() {
    let mut b = RuleBook::new();
    assert_eq!(b.record("a@x.com", TriageState::Archived), None);
    assert_eq!(
        b.record("A@X.com", TriageState::Archived),
        Some(r("a@x.com", TriageState::Archived))
    );
    assert_eq!(
        b.record("a@x.com", TriageState::Archived),
        None,
        "third does not re-suggest"
    );
}
#[test]
fn differing_action_resets_streak() {
    let mut b = RuleBook::new();
    b.record("a@x.com", TriageState::Archived);
    assert_eq!(b.record("a@x.com", TriageState::Snoozed), None);
    assert_eq!(
        b.record("a@x.com", TriageState::Snoozed),
        Some(r("a@x.com", TriageState::Snoozed))
    );
}
#[test]
fn senders_are_independent() {
    let mut b = RuleBook::new();
    b.record("a@x.com", TriageState::Archived);
    assert_eq!(b.record("b@x.com", TriageState::Archived), None);
}
#[test]
fn dismissed_rule_never_resuggested_but_other_state_can() {
    let mut b = RuleBook::new();
    b.record("a@x.com", TriageState::Archived);
    let s = b.record("a@x.com", TriageState::Archived).unwrap();
    b.dismiss(s);
    b.record("a@x.com", TriageState::Snoozed);
    b.record("a@x.com", TriageState::Archived);
    assert_eq!(b.record("a@x.com", TriageState::Archived), None);
    b.record("a@x.com", TriageState::Snoozed);
    assert!(b.record("a@x.com", TriageState::Snoozed).is_some());
}
#[test]
fn existing_rule_blocks_suggestion_and_lookup_is_case_insensitive() {
    let mut b = RuleBook::new();
    b.accept(r("A@x.com", TriageState::Archived));
    assert_eq!(b.rule_for("a@X.COM"), Some(TriageState::Archived));
    assert_eq!(b.rule_for("other@x.com"), None);
    b.record("a@x.com", TriageState::Snoozed);
    assert_eq!(b.record("a@x.com", TriageState::Snoozed), None);
    b.accept(r("a@x.com", TriageState::Snoozed));
    assert_eq!(b.rules().len(), 1);
    assert_eq!(b.rule_for("a@x.com"), Some(TriageState::Snoozed));
}
#[test]
fn revoke_by_index_removes_and_allows_resuggestion() {
    let mut b = RuleBook::new();
    b.accept(r("a@x.com", TriageState::Archived));
    b.accept(r("b@x.com", TriageState::Snoozed));
    assert_eq!(b.revoke(5), None);
    assert_eq!(b.revoke(0), Some(r("a@x.com", TriageState::Archived)));
    assert_eq!(b.rules(), &[r("b@x.com", TriageState::Snoozed)]);
    assert_eq!(b.rule_for("a@x.com"), None);
    b.record("a@x.com", TriageState::Archived);
    assert!(b.record("a@x.com", TriageState::Archived).is_some());
}
