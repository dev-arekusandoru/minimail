use mail_classifier::judge::{
    Answer, AnswerValue, Confidence, Judge, JudgeError, JudgePolicy, Kind, Mode, Question,
    QuestionKey, Routed, StoredAnswer, StubJudge, classify, expects_reply, incoming_questions,
    message_state, questions_to_ask, reply_question, route,
};
use mail_classifier::model::{Message, TriageState};
use mail_classifier::summary::{StubSummarizer, Summarizer, SummaryError};

fn fixture() -> Vec<Message> {
    serde_json::from_str(include_str!("fixtures/mailbox.json")).unwrap()
}

fn msg(id: u32, email: &str, subject: &str, body: &str) -> Message {
    Message {
        id,
        thread_id: id,
        from_name: "N".into(),
        from_email: email.into(),
        to: "me@x.io".into(),
        subject: subject.into(),
        body: body.into(),
        received: "2026-09-28T08:00:00Z".into(),
        state: TriageState::Inbox,
        account: "personal".into(),
        outgoing: false,
        snooze: None,
        cc: String::new(),
        bcc: String::new(),
        bulk: false,
        html: None,
        attachments: Vec::new(),
        read: false,
        partial: false,
    }
}

fn answers(m: &Message) -> Vec<(QuestionKey, Answer)> {
    StubJudge.judge(&message_state(m, ""), &questions()).unwrap()
}

/// The default incoming questions, with no account folders.
fn questions() -> Vec<(QuestionKey, Question)> {
    incoming_questions(&JudgePolicy::default(), false, &[])
        .into_iter()
        .map(|s| (s.key, s.question))
        .collect()
}

fn get(a: &[(QuestionKey, Answer)], k: QuestionKey) -> &Answer {
    &a.iter().find(|(key, _)| *key == k).unwrap().1
}

/// Judge returning a fixed answer for every question, for routing tests.
struct Fixed {
    spam_yes: f32,
}

impl Judge for Fixed {
    fn judge(
        &self,
        _: &serde_json::Value,
        qs: &[(QuestionKey, Question)],
    ) -> Result<Vec<(QuestionKey, Answer)>, JudgeError> {
        Ok(qs
            .iter()
            .map(|(k, _)| {
                let a = match k {
                    QuestionKey::Spam => Answer {
                        probabilities: vec![1.0 - self.spam_yes, self.spam_yes],
                        value: AnswerValue::Bool(true),
                        confidence: self.spam_yes,
                    },
                    _ => Answer {
                        probabilities: vec![1.0],
                        value: AnswerValue::Bool(false),
                        confidence: 1.0,
                    },
                };
                (*k, a)
            })
            .collect())
    }
}

struct Failing;
impl Judge for Failing {
    fn judge(
        &self,
        _: &serde_json::Value,
        _: &[(QuestionKey, Question)],
    ) -> Result<Vec<(QuestionKey, Answer)>, JudgeError> {
        Err(JudgeError::Unavailable("offline".into()))
    }
}

#[test]
fn stub_is_deterministic_and_probabilities_sum_to_one() {
    for m in fixture() {
        let a = answers(&m);
        assert_eq!(a, answers(&m));
        let qs = questions();
        assert_eq!(a.len(), qs.len());
        for ((k, q), (ak, ans)) in qs.iter().zip(&a) {
            assert_eq!(k, ak);
            assert_eq!(ans.probabilities.len(), q.arity());
            let sum: f32 = ans.probabilities.iter().sum();
            assert!((sum - 1.0).abs() < 1e-4, "{k:?} sums to {sum}");
            assert!((0.0..=1.0).contains(&ans.confidence));
        }
    }
}

#[test]
fn fixture_yields_realistic_mix() {
    let all = fixture();
    let mut spam = 0;
    let mut reply = 0;
    let mut kinds = std::collections::HashSet::new();
    let mut urgency = std::collections::HashSet::new();
    for m in &all {
        let a = answers(m);
        if get(&a, QuestionKey::Spam).value == AnswerValue::Bool(true) {
            spam += 1;
        }
        if get(&a, QuestionKey::NeedsReply).value == AnswerValue::Bool(true) {
            reply += 1;
        }
        if let AnswerValue::Choice(i) = get(&a, QuestionKey::Kind).value {
            kinds.insert(i);
        }
        if let AnswerValue::Score(s) = get(&a, QuestionKey::Urgency).value {
            assert!((1.0..=5.0).contains(&s));
            urgency.insert(s.round() as u8);
        }
    }
    assert!((2..=10).contains(&spam), "spam={spam}");
    assert!(reply >= 5, "needs-reply={reply}");
    assert_eq!(kinds.len(), 5, "kinds={kinds:?}");
    assert!(urgency.len() >= 3, "urgency={urgency:?}");
}

#[test]
fn classify_applies_labels_and_drops_noops_on_fixture() {
    let all = fixture();
    let refs: Vec<&Message> = all.iter().collect();
    let routed = classify(&StubJudge, &JudgePolicy::default(), &refs);
    assert_eq!(routed.len(), all.len() * 4);
    assert!(routed.iter().any(|r| matches!(r, Routed::Auto(_))));
    assert!(routed.iter().any(|r| matches!(r, Routed::Drop)));
    assert!(
        !routed.iter().any(|r| matches!(r, Routed::Review(_))),
        "the fixture has no folders, so nothing is queued for review"
    );
}

#[test]
fn message_state_carries_context_and_trims_body() {
    let long = format!("  {}  ", "é".repeat(9000));
    let m = msg(1, "a@b.io", "s", &long);
    let st = message_state(&m, "me@x.io");
    let body = st["body"].as_str().unwrap();
    assert_eq!(body.chars().count(), 8000);
    assert!(!body.starts_with(' '));
    assert!(st["sender"].as_str().unwrap().contains("a@b.io"));
    assert_eq!(st["subject"], "s");
    assert_eq!(st["recipient_role"], "to");
    assert_eq!(st["bulk"], false);
    assert_eq!(st.as_object().unwrap().len(), 8);
}

fn spam_route(spam_yes: f32, policy: &JudgePolicy) -> Vec<Routed> {
    let m = msg(1, "a@b.io", "s", "b");
    classify(&Fixed { spam_yes }, policy, &[&m])
}

#[test]
fn a_non_folder_label_is_applied_however_it_routed() {
    let mut p = JudgePolicy::default();
    for c in Confidence::ALL {
        p.set_mode(QuestionKey::Spam, Mode::Auto(c));
        // Spam is the first question; a low-confidence answer is applied just the same.
        assert!(matches!(spam_route(c.threshold() - 0.01, &p)[0], Routed::Auto(_)), "{c:?} below");
    }
    // Review applies too: the row menu replaces the y/n prompt for a non-folder label.
    p.set_mode(QuestionKey::Spam, Mode::Review);
    assert!(matches!(spam_route(0.51, &p)[0], Routed::Auto(_)));
}

#[test]
fn off_suggests_nothing_even_for_a_certain_answer() {
    let mut p = JudgePolicy::default();
    p.set_mode(QuestionKey::Spam, Mode::Off);
    assert!(matches!(spam_route(1.0, &p)[0], Routed::Drop));
}

#[test]
fn only_a_folder_suggestion_is_queued_for_review() {
    let p = JudgePolicy::default();
    let m = msg(1, "a@b.io", "s", "b");
    let folder = Answer {
        probabilities: vec![0.9, 0.1],
        value: AnswerValue::Choice(0),
        confidence: 0.9,
    };
    let spam_yes = Answer {
        probabilities: vec![0.01, 0.99],
        value: AnswerValue::Bool(true),
        confidence: 0.99,
    };
    let routed = route(
        &p,
        &m,
        vec![(QuestionKey::Spam, spam_yes), (QuestionKey::Folder, folder)],
    );
    assert!(matches!(routed[0], Routed::Auto(_)), "the spam label applies at once");
    match &routed[1] {
        Routed::Review(s) => assert_eq!(s.key, QuestionKey::Folder),
        other => panic!("a folder suggestion is reviewed, not {other:?}"),
    }
}

#[test]
fn noop_answers_drop_even_in_auto() {
    // Fixed says every non-spam Bool is false -> Drop.
    let routed = spam_route(0.99, &JudgePolicy::default());
    assert!(matches!(routed[0], Routed::Auto(_)));
    // NeedsReply false -> Drop; the remaining wrongly-shaped answers are no-ops too.
    assert!(matches!(routed[1], Routed::Drop));
}

#[test]
fn expects_reply_heuristic_and_api() {
    assert!(expects_reply(&StubJudge, "Let me know what you think."));
    assert!(expects_reply(&StubJudge, "Can you send this today?"));
    assert!(expects_reply(&StubJudge, "Thoughts"));
    assert!(!expects_reply(&StubJudge, "Thanks, received."));
    // The reply question is the only one asked on the reply path.
    let (key, q) = reply_question();
    assert_eq!(key, QuestionKey::ExpectsReply);
    let a = StubJudge
        .judge(&serde_json::json!({"body": "Could you?"}), &[(key, q)])
        .unwrap();
    assert_eq!(a[0].1.value, AnswerValue::Bool(true));
}

#[test]
fn judge_error_drops_every_question() {
    let m = msg(1, "a@b.io", "s", "b");
    let routed = classify(&Failing, &JudgePolicy::default(), &[&m]);
    assert_eq!(routed.len(), 4);
    assert!(routed.iter().all(|r| matches!(r, Routed::Drop)));
}

#[test]
fn spam_kind_urgency_examples() {
    let spam = msg(
        1,
        "aisha@talentloop.com",
        "2-minute fit check",
        "Hi, hiring roles.",
    );
    let a = answers(&spam);
    assert_eq!(get(&a, QuestionKey::Spam).value, AnswerValue::Bool(true));
    assert!(get(&a, QuestionKey::Spam).confidence >= 0.9);

    let receipt = msg(
        2,
        "receipts@stripe.com",
        "Receipt from X",
        "Payment received.",
    );
    let a = answers(&receipt);
    assert_eq!(get(&a, QuestionKey::Spam).value, AnswerValue::Bool(false));
    let kind = get(&a, QuestionKey::Kind).value;
    assert_eq!(kind, AnswerValue::Choice(1));
    assert_eq!(Kind::from_index(1), Some(Kind::Receipt));

    let fail = msg(
        3,
        "notifications@vercel.com",
        "Deployment failed",
        "Build failed.",
    );
    let AnswerValue::Score(s) = get(&answers(&fail), QuestionKey::Urgency).value else {
        panic!()
    };
    let AnswerValue::Score(r) = get(&a, QuestionKey::Urgency).value else {
        panic!()
    };
    assert!(s > r + 1.0, "failure {s} vs receipt {r}");
}

#[test]
fn nearest_preset_for_a_threshold() {
    assert_eq!(Confidence::from_threshold(0.95), Confidence::High);
    assert_eq!(Confidence::from_threshold(0.8), Confidence::Medium);
    assert_eq!(Confidence::from_threshold(0.7), Confidence::Medium);
    assert_eq!(Confidence::from_threshold(0.5), Confidence::Low);
    for c in Confidence::ALL {
        assert_eq!(Confidence::from_threshold(c.threshold()), c);
    }
}

#[test]
fn a_policy_survives_a_json_round_trip() {
    let mut p = JudgePolicy::default();
    p.set_mode(QuestionKey::Spam, Mode::Off);
    p.set_mode(QuestionKey::Kind, Mode::Auto(Confidence::High));
    let back: JudgePolicy = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
    assert_eq!(back, p);
}

fn thread(bodies: &[&str]) -> Vec<Message> {
    bodies
        .iter()
        .enumerate()
        .map(|(i, b)| msg(i as u32, "a@b.io", "Plan", b))
        .collect()
}

#[test]
fn summarizer_extracts_sentence_actions_and_dates() {
    let t = thread(&[
        "Hi team. Here is the plan.\n\nPlease send the deck by 2026-10-01.\nCan you confirm Thu 1 Oct 15:00?\nNothing else.",
        "Second message. Flight is on 8 Oct.",
    ]);
    let refs: Vec<&Message> = t.iter().collect();
    let s = StubSummarizer.summarize(&refs).unwrap();
    assert!(s.summary.starts_with("Hi team."));
    assert!(s.summary.contains("Second message."));
    assert!(!s.summary.contains("Nothing else"));
    assert_eq!(
        s.action_items,
        vec![
            "Please send the deck by 2026-10-01.",
            "Can you confirm Thu 1 Oct 15:00?"
        ]
    );
    assert_eq!(s.dates, vec!["2026-10-01", "Thu 1 Oct 15:00", "8 Oct"]);
    assert_eq!(s, StubSummarizer.summarize(&refs).unwrap());
}

#[test]
fn summarizer_empty_thread_and_empty_body() {
    assert_eq!(
        StubSummarizer.summarize(&[]),
        Err(SummaryError::EmptyThread)
    );
    let t = thread(&["   "]);
    let s = StubSummarizer.summarize(&[&t[0]]).unwrap();
    assert_eq!(s.summary, "Plan");
    assert!(s.action_items.is_empty() && s.dates.is_empty());
}

#[test]
fn a_question_is_re_asked_only_when_missing_or_stale() {
    let folders = vec![(3u32, "Receipts".to_owned()), (7u32, "Projects".to_owned())];
    let policy = JudgePolicy::default();
    let specs = incoming_questions(&policy, false, &folders);
    let keys: Vec<QuestionKey> = specs.iter().map(|s| s.key).collect();
    assert_eq!(
        keys,
        vec![
            QuestionKey::Spam,
            QuestionKey::NeedsReply,
            QuestionKey::Urgency,
            QuestionKey::Kind,
            QuestionKey::Folder,
        ]
    );
    // No folders: Folder is not asked.
    assert_eq!(incoming_questions(&policy, false, &[]).len(), 4);

    let answer = Answer {
        probabilities: vec![0.5, 0.5],
        value: AnswerValue::Bool(true),
        confidence: 0.9,
    };
    let stored: Vec<StoredAnswer> = specs
        .iter()
        .map(|s| StoredAnswer {
            key: s.key,
            version: s.version.clone(),
            answer: answer.clone(),
        })
        .collect();
    assert!(questions_to_ask(&specs, &stored).is_empty(), "all current: nothing to ask");

    // Bumping one question's version re-asks only that question.
    let mut stale = stored.clone();
    stale[2].version = "9".to_owned();
    let ask = questions_to_ask(&specs, &stale);
    assert_eq!(ask.len(), 1);
    assert_eq!(ask[0].0, QuestionKey::Urgency);

    // Changing the account's folder list re-asks only Folder.
    let moved = vec![(7u32, "Projects".to_owned())];
    let ask = questions_to_ask(&incoming_questions(&policy, false, &moved), &stored);
    assert_eq!(ask.len(), 1);
    assert_eq!(ask[0].0, QuestionKey::Folder);

    // The cold-outreach checkbox is part of Spam's version, so toggling it re-asks Spam.
    let ask = questions_to_ask(&incoming_questions(&policy, true, &folders), &stored);
    assert_eq!(ask.len(), 1);
    assert_eq!(ask[0].0, QuestionKey::Spam);

    // An Off question is not asked at all.
    let mut off = JudgePolicy::default();
    off.set_mode(QuestionKey::Kind, Mode::Off);
    let keys: Vec<QuestionKey> =
        incoming_questions(&off, false, &folders).iter().map(|s| s.key).collect();
    assert!(!keys.contains(&QuestionKey::Kind));
}
