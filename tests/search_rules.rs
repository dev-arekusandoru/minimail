use mail_classifier::judge::{Answer, AnswerValue, QuestionKey, Suggestion};
use mail_classifier::model::{Mailbox, Message, TriageState};
use mail_classifier::rules::{Rule, RuleBook};
use mail_classifier::model::parse_rfc3339;
use mail_classifier::search::{Combinator, Field, Pill, Query};

/// 2026-09-29T12:00:00Z
const NOW: i64 = 1_790_683_200;

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
        cc: String::new(),
        bcc: String::new(),
        html: None,
        attachments: Vec::new(),
        read: false,
        partial: false,
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
    Query::parse(q).matches(&message, &mailbox(message.clone(), TriageState::Inbox), NOW)
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
        assert!(q(query).matches(&message, &mailbox(message.clone(), state), NOW));
    }
    let message = m();
    let mut snoozed = mailbox(message.clone(), TriageState::Inbox);
    snoozed.snooze(&[message.id], 30, 0);
    assert!(q("is:snoozed").matches(&message, &snoozed, NOW));
    let message = m();
    assert!(!q("is:waiting").matches(&message, &mailbox(message.clone(), TriageState::Inbox), NOW));
    assert!(!q("is:later").matches(&message, &mailbox(message.clone(), TriageState::Inbox), NOW));
    assert!(!q("is:done").matches(&message, &mailbox(message.clone(), TriageState::Inbox), NOW));
    assert!(!q("is:screener").matches(&message, &mailbox(message.clone(), TriageState::Inbox), NOW));
}

#[test]
fn sent_operator_uses_outgoing_status() {
    let mut sent = m();
    sent.outgoing = true;
    assert!(Query::parse("is:sent").matches(&sent, &mailbox(sent.clone(), TriageState::Inbox), NOW));
    let ordinary = m();
    assert!(
        !Query::parse("is:sent").matches(&ordinary, &mailbox(ordinary.clone(), TriageState::Inbox), NOW)
    );
    assert!(!Query::parse("is:sent").matches(&sent, &mailbox(sent.clone(), TriageState::Deleted), NOW));
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
    assert!(Query::parse("tag:spam").matches(&m, &mailbox, NOW));
    assert!(!Query::parse("tag:needs-reply").matches(&m, &mailbox, NOW));
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
    assert!(Query::parse("foo:bar").matches(&mm, &mailbox(mm.clone(), TriageState::Inbox), NOW));
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

// ------------------------------------------------------------ structured query

fn full() -> Message {
    let mut x = m();
    x.to = "Bob Jones <bob@corp.io>, carol@corp.io".into();
    x.cc = "dave@other.org".into();
    x.bcc = "erin@secret.net".into();
    x
}

fn hit_msg(q: &str, x: &Message, now: i64) -> bool {
    Query::parse(q).matches(x, &mailbox(x.clone(), TriageState::Inbox), now)
}

#[test]
fn now_constant_is_the_documented_instant() {
    assert_eq!(parse_rfc3339("2026-09-29T12:00:00Z"), Some(NOW));
}

#[test]
fn recipient_and_body_fields_match_their_own_header_only() {
    let x = full();
    for (yes, no) in [
        ("to:bob", "to:dave"),
        ("to:CAROL@corp", "to:erin"),
        ("cc:other.org", "cc:bob"),
        ("bcc:erin", "bcc:dave"),
        ("body:budget", "body:quarterly"),
        ("subject:quarterly", "subject:budget"),
    ] {
        assert!(hit_msg(yes, &x, NOW), "{yes}");
        assert!(!hit_msg(no, &x, NOW), "{no}");
    }
}

#[test]
fn on_matches_exactly_one_day() {
    assert!(hit("on:2026-09-14"));
    assert!(!hit("on:2026-09-13"));
    assert!(!hit("on:2026-09-15"));
    assert!(hit_msg("on:15d", &m(), NOW));
    assert!(!hit_msg("on:14d", &m(), NOW));
}

#[test]
fn relative_dates_resolve_against_the_passed_now() {
    let x = m(); // received 2026-09-14, 15 days before NOW
    // after is inclusive of the resolved day, before exclusive.
    assert!(hit_msg("after:15d", &x, NOW));
    assert!(!hit_msg("after:14d", &x, NOW));
    assert!(hit_msg("after:3w", &x, NOW));
    assert!(!hit_msg("after:2w", &x, NOW));
    assert!(!hit_msg("before:15d", &x, NOW));
    assert!(hit_msg("before:14d", &x, NOW));
    assert!(hit_msg("after:1m", &x, NOW));
    assert!(!hit_msg("before:1m", &x, NOW));
    assert!(hit_msg("after:1y", &x, NOW) && !hit_msg("before:1y", &x, NOW));
    // Moving `now` moves the window.
    let later = NOW + 20 * 86_400;
    assert!(!hit_msg("after:7d", &x, later));
    assert!(hit_msg("before:7d", &x, later));
    // Absolute and relative combine.
    assert!(hit_msg("after:2026-09-01 before:1d", &x, NOW));
}

#[test]
fn relative_months_clamp_to_month_length() {
    let end_of_feb = msg("A", "a@x.com", "s", "b", "2026-02-28T10:00:00Z");
    let mar31 = parse_rfc3339("2026-03-31T12:00:00Z").unwrap();
    // 1 month before Mar 31 is Feb 28 (clamped).
    assert!(hit_msg("after:1m", &end_of_feb, mar31));
    assert!(!hit_msg("before:1m", &end_of_feb, mar31));
    let leap = parse_rfc3339("2028-02-29T12:00:00Z").unwrap();
    let last_year = msg("A", "a@x.com", "s", "b", "2027-02-28T10:00:00Z");
    assert!(hit_msg("after:1y", &last_year, leap));
    assert!(!hit_msg("before:1y", &last_year, leap));
}

#[test]
fn malformed_relative_dates_stay_text() {
    for bad in ["after:d", "after:7x", "after:-3d", "before:99999d", "on:1.5d"] {
        let q = Query::parse(bad);
        assert!(q.groups().is_empty(), "{bad}");
        assert_eq!(q.text().len(), 1, "{bad}");
    }
}

#[test]
fn or_group_matches_any_and_group_requires_all() {
    let x = full();
    assert!(hit_msg("from:alice from:smith", &x, NOW));
    assert!(!hit_msg("from:alice from:bob", &x, NOW));
    assert!(hit_msg("from:alice,bob", &x, NOW));
    assert!(hit_msg("from:bob,alice", &x, NOW));
    assert!(!hit_msg("from:bob,carol", &x, NOW));
    // Different fields are ANDed even when each group is OR.
    assert!(hit_msg("from:alice,bob to:bob,zed", &x, NOW));
    assert!(!hit_msg("from:alice,bob to:zed,yan", &x, NOW));
    // Single-value OR group behaves like that value.
    assert!(hit_msg("from:alice,", &x, NOW));
    assert!(!hit_msg("from:bob,", &x, NOW));
}

#[test]
fn set_combinator_changes_matching() {
    let x = full();
    let mut q = Query::parse("from:alice from:bob");
    let mb = mailbox(x.clone(), TriageState::Inbox);
    assert!(!q.matches(&x, &mb, NOW));
    assert!(q.set_combinator(Field::From, Combinator::Or));
    assert!(q.matches(&x, &mb, NOW));
    assert!(!q.set_combinator(Field::To, Combinator::Or), "no such group");
}

#[test]
fn kind_account_and_in_use_existing_semantics() {
    let x = m();
    let mut mb = mailbox(x.clone(), TriageState::Inbox);
    mb.apply_auto(
        Suggestion {
            message: x.id,
            key: QuestionKey::Kind,
            answer: Answer {
                probabilities: vec![0.0, 0.9, 0.0, 0.0, 0.1],
                value: AnswerValue::Choice(1),
                confidence: 0.9,
            },
        },
        0,
    );
    let q = |s: &str| Query::parse(s).matches(&x, &mb, NOW);
    assert!(q("kind:receipt"));
    assert!(!q("kind:person"));
    assert!(q("kind:person,receipt"));
    assert!(!q("kind:person kind:receipt"));
    assert!(!q("kind:bogus"), "invalid kind stays text");
    assert!(q("account:personal"));
    assert!(q("account:PERSONAL"));
    assert!(!q("account:work"));
    assert!(q("account:work,personal"));
    assert!(q("in:inbox"));
    assert!(!q("in:sent"));
    assert!(!q("in:archived"));
    assert!(q("in:inbox,archived"));
    assert!(q("in:inbox account:personal kind:receipt"));
}

#[test]
fn in_locations_follow_triage_state_and_direction() {
    let x = m();
    let at = |q: &str, st: TriageState| {
        Query::parse(q).matches(&x, &mailbox(x.clone(), st), NOW)
    };
    assert!(at("in:archived", TriageState::Archived));
    assert!(at("in:archive", TriageState::Archived));
    assert!(at("in:deleted", TriageState::Deleted));
    assert!(at("in:trash", TriageState::Deleted));
    assert!(!at("in:inbox", TriageState::Archived));
    let mut sent = m();
    sent.outgoing = true;
    assert!(Query::parse("in:sent").matches(&sent, &mailbox(sent.clone(), TriageState::Inbox), NOW));
    assert!(!Query::parse("in:inbox").matches(&sent, &mailbox(sent.clone(), TriageState::Inbox), NOW));
    assert!(!Query::parse("in:sent").matches(&sent, &mailbox(sent.clone(), TriageState::Deleted), NOW));
    let mut snoozed = mailbox(x.clone(), TriageState::Inbox);
    snoozed.snooze(&[x.id], 30, 0);
    assert!(Query::parse("in:snoozed").matches(&x, &snoozed, NOW));
}

#[test]
fn in_folder_name_matches_filed_mail_of_that_account() {
    let x = m();
    let mut mb = mailbox(x.clone(), TriageState::Inbox);
    let (_, moved) = mb.create_folder_and_file("personal", "Receipts 2026", None, &[x.id]);
    assert_eq!(moved, 1);
    assert!(Query::parse("in:\"receipts 2026\"").matches(&x, &mb, NOW));
    assert!(!Query::parse("in:other").matches(&x, &mb, NOW));
    assert!(!Query::parse("in:inbox").matches(&x, &mb, NOW));
    assert!(Query::parse("is:filed").matches(&x, &mb, NOW));
}

fn rt(s: &str) -> Query {
    let q = Query::parse(s);
    assert_eq!(Query::parse(&q.to_string()), q, "round trip of {s:?} via {:?}", q.to_string());
    assert_eq!(q.describe(), q.to_string());
    q
}

#[test]
fn display_round_trips_including_combinators() {
    for s in [
        "",
        "hello world",
        "from:alice",
        "from:alice from:bob",
        "from:alice,bob",
        "from:alice,",
        "from:\"ann lee\",bob to:x cc:y bcc:z",
        "subject:\"q3 report\" body:budget \"two words\" extra",
        "after:7d before:2026-10-01 on:2w",
        "is:inbox,sent tag:spam,urgent kind:receipt account:work in:archived",
        "in:\"my folder\"",
        "from:a to:b from:c",
        "tag:bogus foo:bar from:",
    ] {
        rt(s);
    }
    assert_eq!(rt("from:a,b from:c").to_string(), "from:a,b,c");
    assert_eq!(rt("from:a from:b").to_string(), "from:a from:b");
    assert_eq!(rt("from:a,").to_string(), "from:a,");
    assert_eq!(rt("FROM:Alice /").to_string(), "from:alice \"/\"");
}

#[test]
fn parse_is_case_insensitive_quote_aware_and_ignores_leading_slash() {
    let q = Query::parse("  /From:\"Ann Lee\" SUBJECT:Hi,There Free");
    assert_eq!(q.values(Field::From), ["ann lee"]);
    assert_eq!(q.values(Field::Subject), ["hi", "there"]);
    assert_eq!(q.combinator(Field::Subject), Some(Combinator::Or));
    assert_eq!(q.text(), ["free"]);
    assert_eq!(Query::parse("\"a,b\" x").text(), ["a,b", "x"]);
    assert_eq!(Query::parse("from:\"a,b\"").values(Field::From), ["a,b"]);
}

#[test]
fn unknown_operator_stays_text_and_round_trips() {
    let q = rt("foo:bar is:bogus before:soon from:");
    assert!(q.groups().is_empty());
    assert_eq!(q.text(), ["foo:bar", "is:bogus", "before:soon", "from:"]);
    assert!(!q.is_empty());
}

#[test]
fn editing_api_toggles_and_orders_pills() {
    let mut q = Query::default();
    assert!(q.is_empty());
    assert!(q.toggle(Field::From, "Alice"), "first toggle adds");
    assert!(q.has(Field::From, "alice"));
    assert!(!q.toggle(Field::From, "alice"), "second toggle removes");
    assert!(q.is_empty(), "empty group is dropped");

    assert!(q.add(Field::From, "alice"));
    assert!(!q.add(Field::From, "ALICE"), "duplicate is a no-op");
    assert!(q.add(Field::Is, "inbox"));
    assert!(q.add(Field::From, "bob"));
    assert!(!q.add(Field::Is, "bogus"), "invalid value rejected");
    assert!(!q.add(Field::After, "yesterday"));
    assert!(q.add(Field::After, "7D"));
    assert!(q.set_combinator(Field::From, Combinator::Or));
    assert_eq!(
        q.pills(),
        vec![
            Pill { field: Field::From, value: "alice".into(), combinator: None },
            Pill { field: Field::From, value: "bob".into(), combinator: Some(Combinator::Or) },
            Pill { field: Field::Is, value: "inbox".into(), combinator: None },
            Pill { field: Field::After, value: "7d".into(), combinator: None },
        ]
    );
    assert_eq!(q.to_string(), "from:alice,bob is:inbox after:7d");

    // Removing down to one value keeps the combinator choice for later adds.
    assert!(q.remove(Field::From, "bob"));
    assert!(!q.remove(Field::From, "bob"));
    assert_eq!(q.combinator(Field::From), Some(Combinator::Or));
    assert_eq!(Query::parse(&q.to_string()), q);
    assert!(q.add(Field::From, "carol"));
    assert_eq!(q.to_string(), "from:alice,carol is:inbox after:7d");

    assert!(q.clear_field(Field::From));
    assert!(!q.clear_field(Field::From));
    assert_eq!(q.to_string(), "is:inbox after:7d");
    assert_eq!(Combinator::And.toggled(), Combinator::Or);
}

#[test]
fn editing_preserves_free_text() {
    let mut q = Query::parse("budget from:alice");
    q.clear_field(Field::From);
    assert_eq!(q.to_string(), "\"budget\"");
    assert!(!q.is_empty());
}
