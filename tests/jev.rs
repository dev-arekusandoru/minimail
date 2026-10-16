//! Jev wire mapping: request shape, answer parsing (recorded JSON, no network),
//! malformed cases, and the judgement cache key.

use mail_classifier::judge::jev::{self, DEFAULT_BASE_URL, DEFAULT_MODEL, JevJudge};
use mail_classifier::judge::{
    Answer, AnswerValue, Judge, JudgeError, JudgePolicy, Question, QuestionKey, incoming_questions,
    legacy_judgement_key,
};
use mail_classifier::model::{Message, TriageState};
use serde_json::{Value, json};

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-6
}

/// The default incoming questions, with no account folders.
fn questions() -> Vec<(QuestionKey, Question)> {
    incoming_questions(&JudgePolicy::default(), false, &[])
        .into_iter()
        .map(|s| (s.key, s.question))
        .collect()
}

fn answers(body: Value) -> Result<Vec<(QuestionKey, Answer)>, JudgeError> {
    jev::parse_answers(&body, &questions())
}

fn get(a: &[(QuestionKey, Answer)], k: QuestionKey) -> &Answer {
    &a.iter().find(|(key, _)| *key == k).unwrap().1
}

fn choice_question() -> Question {
    Question::Choice {
        criteria: "Which kind?".into(),
        options: ["person", "receipt", "newsletter"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
    }
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
        html: None,
        attachments: Vec::new(),
        read: false,
        partial: false,
        bulk: false,
    }
}

#[test]
fn request_body_shapes_each_question_type() {
    let questions = questions();
    let body = jev::request_body("jev-latest", &json!({"sender": "a@b.c"}), &questions);

    assert_eq!(body["model"], "jev-latest");
    assert_eq!(body["state"]["sender"], "a@b.c");

    let q = &body["questions"];
    // Names are the stable Debug form of each key.
    let Question::Bool { criteria } = &questions[0].1 else {
        panic!("Spam is a bool question");
    };
    assert_eq!(q["Spam"]["type"], "noul");
    assert_eq!(q["Spam"]["instructions"], criteria.as_str());
    assert!(q["Spam"].get("criteria").is_none());

    assert_eq!(q["Kind"]["type"], "choice");
    // Each option maps to itself as its own rubric description.
    assert_eq!(q["Kind"]["criteria"]["receipt"], "receipt");
    assert_eq!(q["Kind"]["criteria"].as_object().unwrap().len(), 5);

    assert_eq!(q["Urgency"]["type"], "score");
    // The five level descriptions go as the criteria list, not "1".."5".
    let levels = q["Urgency"]["criteria"].as_array().unwrap();
    assert_eq!(levels.len(), 5);
    assert_eq!(levels[0], "No action or time pressure.");
}

#[test]
fn noul_answer_maps_to_bool_and_confidence() {
    let out = answers(json!({"answers": {
        "Spam": {"type": "noul", "noul": 0.93},
        "NeedsReply": {"type": "noul", "noul": 0.2},
        "ExpectsReply": {"type": "noul", "noul": 0.5},
        "Urgency": {"type": "score", "score": 1.0, "confidence": 0.5, "probabilities": {"0": 1.0}},
        "Kind": {"type": "choice", "choice": "receipt", "confidence": 0.8, "probabilities": {"receipt": 0.8}},
    }}))
    .expect("well-formed response");

    let spam = get(&out, QuestionKey::Spam);
    assert_eq!(spam.value, AnswerValue::Bool(true));
    assert!(close(spam.confidence, 0.93));
    assert_eq!(spam.probabilities.len(), 2);
    assert!(close(spam.probabilities[0], 0.07));
    assert!(close(spam.probabilities[1], 0.93));

    let needs = get(&out, QuestionKey::NeedsReply);
    assert_eq!(needs.value, AnswerValue::Bool(false));
    assert!(close(needs.confidence, 0.8));
}

#[test]
fn choice_answer_indexes_options_with_missing_options_at_zero() {
    let body = json!({"answers": {"Kind": {
        "type": "choice",
        "choice": "receipt",
        "confidence": 0.78,
        "probabilities": {"person": 0.1, "receipt": 0.9},
    }}});
    let out = jev::parse_answers(&body, &[(QuestionKey::Kind, choice_question())]).expect("parse");
    let a = &out[0].1;
    assert_eq!(a.value, AnswerValue::Choice(1));
    assert!(close(a.confidence, 0.78));
    assert!(close(a.probabilities[0], 0.1));
    assert!(close(a.probabilities[1], 0.9));
    assert!(close(a.probabilities[2], 0.0)); // newsletter absent from the map
}

#[test]
fn score_answer_is_shifted_to_one_based() {
    let question = Question::Score {
        criteria: "How urgent?".into(),
        levels: vec!["calm".into(), "frustrated".into(), "angry".into()],
    };
    let body = json!({"answers": {"Urgency": {
        "type": "score",
        "score": 1.05,
        "legend": {"0": "calm", "1": "frustrated", "2": "angry"},
        "confidence": 0.92,
        "probabilities": {"0": 0.0, "1": 0.95, "2": 0.05},
    }}});
    let out = jev::parse_answers(&body, &[(QuestionKey::Urgency, question)]).expect("parse");
    let a = &out[0].1;
    assert_eq!(a.value, AnswerValue::Score(2.05));
    assert!(close(a.confidence, 0.92));
    assert_eq!(a.probabilities.len(), 3);
    assert!(close(a.probabilities[1], 0.95));
}

#[test]
fn malformed_responses_are_rejected() {
    let cases = [
        json!({}),                                        // no answers object
        json!({"answers": {}}),                           // missing every question
        json!({"answers": {"Spam": {"type": "choice", "choice": "x", "confidence": 0.5, "probabilities": {}}}}),
        json!({"answers": {"Kind": {"type": "choice", "choice": "nope", "confidence": 0.5, "probabilities": {}}}}),
        json!({"answers": {"Urgency": {"type": "score", "score": 1.0, "probabilities": {}}}}), // no confidence
    ];
    for body in cases {
        assert!(
            matches!(answers(body), Err(JudgeError::Malformed(_))),
            "expected Malformed"
        );
    }
}

#[test]
fn legacy_judgement_key_is_stable_and_sensitive_to_inputs() {
    let m = msg(1, "a@b.c", "hi", "hello");
    let key = legacy_judgement_key("jev-latest", &m);
    assert_eq!(key, legacy_judgement_key("jev-latest", &m));
    assert_eq!(key.len(), 64);
    assert_ne!(key, legacy_judgement_key("other-model", &m));
    assert_ne!(key, legacy_judgement_key("jev-latest", &msg(1, "a@b.c", "hi", "different")));
}

#[test]
fn judge_debug_redacts_the_key_and_accessors_work() {
    let judge = JevJudge::new(DEFAULT_BASE_URL, DEFAULT_MODEL, "super-secret");
    assert_eq!(judge.model(), DEFAULT_MODEL);
    let debug = format!("{judge:?}");
    assert!(!debug.contains("super-secret"), "{debug}");
    assert!(debug.contains("redacted"));

    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<JevJudge>();
    let _: &dyn Judge = &judge;
}
