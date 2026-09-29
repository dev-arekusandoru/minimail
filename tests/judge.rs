use mail_classifier::judge::{
    Answer, AnswerValue, Judge, JudgeError, JudgePolicy, Kind, Mode, Question, QuestionKey, Routed,
    StubJudge, classify, message_state, triage_questions,
};
use mail_classifier::model::{Message, TriageState};
use mail_classifier::summary::{StubSummarizer, Summarizer, SummaryError};

fn fixture() -> Vec<Message> {
    serde_json::from_str(include_str!("../fixtures/mailbox.json")).unwrap()
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
    }
}

fn answers(m: &Message) -> Vec<(QuestionKey, Answer)> {
    StubJudge
        .judge(&message_state(m), &triage_questions())
        .unwrap()
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
        let qs = triage_questions();
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
fn classify_produces_all_routes_on_fixture() {
    let all = fixture();
    let refs: Vec<&Message> = all.iter().collect();
    let routed = classify(&StubJudge, &JudgePolicy::default(), &refs);
    assert_eq!(routed.len(), all.len() * 5);
    assert!(routed.iter().any(|r| matches!(r, Routed::Auto(_))));
    assert!(routed.iter().any(|r| matches!(r, Routed::Review(_))));
    assert!(routed.iter().any(|r| matches!(r, Routed::Drop)));
}

#[test]
fn message_state_trims_body_to_limit() {
    let long = format!("  {}  ", "é".repeat(4000));
    let m = msg(1, "a@b.io", "s", &long);
    let st = message_state(&m);
    let body = st["body"].as_str().unwrap();
    assert_eq!(body.chars().count(), 1500);
    assert!(!body.starts_with(' '));
    assert!(st["sender"].as_str().unwrap().contains("a@b.io"));
    assert_eq!(st["subject"], "s");
    assert_eq!(st.as_object().unwrap().len(), 3);
}

fn spam_route(spam_yes: f32, policy: &JudgePolicy) -> Vec<Routed> {
    let m = msg(1, "a@b.io", "s", "b");
    classify(&Fixed { spam_yes }, policy, &[&m])
}

#[test]
fn threshold_is_inclusive() {
    let mut p = JudgePolicy::default();
    p.set_mode(QuestionKey::Spam, Mode::Auto { threshold: 0.75 });
    // Spam is the first question.
    assert!(matches!(spam_route(0.75, &p)[0], Routed::Auto(_)));
    assert!(matches!(spam_route(0.7499, &p)[0], Routed::Review(_)));
    assert!(matches!(spam_route(0.9, &p)[0], Routed::Auto(_)));
}

#[test]
fn review_mode_never_auto_applies() {
    let mut p = JudgePolicy::default();
    p.set_mode(QuestionKey::Spam, Mode::Review);
    match &spam_route(1.0, &p)[0] {
        Routed::Review(s) => assert_eq!(s.key, QuestionKey::Spam),
        other => panic!("{other:?}"),
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
fn suggested_state_equal_to_current_drops() {
    let mut m = msg(1, "a@b.io", "Meeting?", "Can you make it?");
    let a = answers(&m);
    let st = get(&a, QuestionKey::SuggestedState);
    let AnswerValue::Choice(i) = st.value else {
        panic!()
    };
    let suggested = TriageState::ALL[i];
    let idx = 2; // SuggestedState position
    m.state = suggested;
    let mut p = JudgePolicy::default();
    p.set_mode(QuestionKey::SuggestedState, Mode::Auto { threshold: 0.0 });
    assert!(matches!(
        classify(&StubJudge, &p, &[&m])[idx],
        Routed::Drop
    ));
    m.state = TriageState::ALL[(i + 1) % 4];
    assert!(matches!(
        classify(&StubJudge, &p, &[&m])[idx],
        Routed::Auto(_)
    ));
}

#[test]
fn judge_error_drops_every_question() {
    let m = msg(1, "a@b.io", "s", "b");
    let routed = classify(&Failing, &JudgePolicy::default(), &[&m]);
    assert_eq!(routed.len(), 5);
    assert!(routed.iter().all(|r| matches!(r, Routed::Drop)));
}

#[test]
fn spam_kind_urgency_examples() {
    let spam = msg(1, "aisha@talentloop.com", "2-minute fit check", "Hi, hiring roles.");
    let a = answers(&spam);
    assert_eq!(get(&a, QuestionKey::Spam).value, AnswerValue::Bool(true));
    assert!(get(&a, QuestionKey::Spam).confidence >= 0.9);

    let receipt = msg(2, "receipts@stripe.com", "Receipt from X", "Payment received.");
    let a = answers(&receipt);
    assert_eq!(get(&a, QuestionKey::Spam).value, AnswerValue::Bool(false));
    let kind = get(&a, QuestionKey::Kind).value;
    assert_eq!(kind, AnswerValue::Choice(1));
    assert_eq!(Kind::from_index(1), Some(Kind::Receipt));

    let fail = msg(3, "notifications@vercel.com", "Deployment failed", "Build failed.");
    let AnswerValue::Score(s) = get(&answers(&fail), QuestionKey::Urgency).value else {
        panic!()
    };
    let AnswerValue::Score(r) = get(&a, QuestionKey::Urgency).value else {
        panic!()
    };
    assert!(s > r + 1.0, "failure {s} vs receipt {r}");
}

#[test]
fn policy_defaults_setters_and_toggle() {
    let mut p = JudgePolicy::default();
    assert_eq!(p.mode(QuestionKey::Spam), Mode::Auto { threshold: 0.9 });
    assert_eq!(p.mode(QuestionKey::SuggestedState), Mode::Review);
    assert_eq!(p.mode(QuestionKey::NeedsReply), Mode::Auto { threshold: 0.8 });
    assert_eq!(p.mode(QuestionKey::Urgency), Mode::Auto { threshold: 0.7 });
    assert_eq!(p.mode(QuestionKey::Kind), Mode::Auto { threshold: 0.6 });

    p.set_threshold(QuestionKey::Kind, 1.7);
    assert_eq!(p.mode(QuestionKey::Kind), Mode::Auto { threshold: 1.0 });
    p.set_threshold(QuestionKey::Kind, 0.55);
    p.toggle_mode(QuestionKey::Kind);
    assert_eq!(p.mode(QuestionKey::Kind), Mode::Review);
    p.toggle_mode(QuestionKey::Kind);
    assert_eq!(p.mode(QuestionKey::Kind), Mode::Auto { threshold: 0.55 });

    p.nudge_threshold(QuestionKey::Kind, -0.05);
    p.nudge_threshold(QuestionKey::Kind, -0.05);
    assert_eq!(p.threshold(QuestionKey::Kind), 0.45);
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
