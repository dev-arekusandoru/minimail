use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::*;

/// How sure the judge must be before a suggestion is applied without review.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    High,
    Medium,
    Low,
}

impl Confidence {
    pub const ALL: [Confidence; 3] = [Confidence::High, Confidence::Medium, Confidence::Low];

    /// Minimum answer confidence for automatic handling (inclusive).
    pub fn threshold(self) -> f32 {
        match self {
            Confidence::High => 0.9,
            Confidence::Medium => 0.75,
            Confidence::Low => 0.6,
        }
    }

    /// The preset whose threshold is nearest to `threshold`; ties go to the stricter preset.
    pub fn from_threshold(threshold: f32) -> Confidence {
        Self::ALL
            .into_iter()
            .min_by(|a, b| {
                let (da, db) = ((a.threshold() - threshold).abs(), (b.threshold() - threshold).abs());
                da.total_cmp(&db)
            })
            .unwrap_or(Confidence::Medium)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Apply automatically when the answer's confidence reaches the preset; otherwise queue for review.
    Auto(Confidence),
    /// Always queue for review.
    Review,
    /// The check is disabled: nothing is suggested, applied or queued.
    Off,
}

/// Per-question routing mode.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JudgePolicy {
    modes: HashMap<QuestionKey, Mode>,
}

impl Default for JudgePolicy {
    fn default() -> Self {
        JudgePolicy {
            modes: HashMap::from([
                (QuestionKey::Spam, Mode::Auto(Confidence::High)),
                (QuestionKey::NeedsReply, Mode::Auto(Confidence::Medium)),
                (QuestionKey::ExpectsReply, Mode::Review),
                (QuestionKey::Urgency, Mode::Auto(Confidence::Medium)),
                (QuestionKey::Kind, Mode::Auto(Confidence::Low)),
                (QuestionKey::Folder, Mode::Review),
            ]),
        }
    }
}

impl JudgePolicy {
    pub fn mode(&self, key: QuestionKey) -> Mode {
        self.modes.get(&key).copied().unwrap_or(Mode::Review)
    }

    pub fn set_mode(&mut self, key: QuestionKey, mode: Mode) {
        self.modes.insert(key, mode);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    pub message: MessageId,
    pub key: QuestionKey,
    pub answer: Answer,
}

impl Suggestion {
    /// Kind carried by a Kind answer.
    pub fn kind(&self) -> Option<Kind> {
        match (self.key, &self.answer.value) {
            (QuestionKey::Kind, AnswerValue::Choice(i)) => Kind::from_index(*i),
            _ => None,
        }
    }

    /// Rounded urgency level (1..=5) carried by an Urgency answer.
    pub fn urgency(&self) -> Option<u8> {
        match (self.key, &self.answer.value) {
            (QuestionKey::Urgency, AnswerValue::Score(s)) => Some(s.round().max(1.0) as u8),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Routed {
    Auto(Suggestion),
    Review(Suggestion),
    Drop,
}

fn is_noop(_m: &Message, s: &Suggestion) -> bool {
    match (&s.key, &s.answer.value) {
        (
            QuestionKey::Spam | QuestionKey::NeedsReply | QuestionKey::ExpectsReply,
            AnswerValue::Bool(b),
        ) => !*b,
        (QuestionKey::Urgency, _) => s.urgency().is_none_or(|u| u <= 1),
        (QuestionKey::Kind, AnswerValue::Choice(i)) => {
            Kind::from_index(*i).is_none_or(|k| k == Kind::Other)
        }
        // The last Folder option is "none": a no-op.
        (QuestionKey::Folder, AnswerValue::Choice(i)) => *i + 1 >= s.answer.probabilities.len(),
        _ => true,
    }
}

/// Runs the incoming questions over each message and routes every answer. Output is flat:
/// for each message (in order), one entry per question. A judge error yields `Drop` for that
/// message. Folders are unknown here, so the Folder question is not asked.
pub fn classify(judge: &dyn Judge, policy: &JudgePolicy, msgs: &[&Message]) -> Vec<Routed> {
    let specs = incoming_questions(policy, false, &[]);
    let questions: Vec<(QuestionKey, Question)> =
        specs.iter().map(|s| (s.key, s.question.clone())).collect();
    let mut out = Vec::with_capacity(msgs.len() * questions.len());
    for m in msgs {
        match judge.judge(&message_state(m, ""), &questions) {
            Ok(answers) => out.extend(route(policy, m, answers)),
            Err(_) => out.extend(questions.iter().map(|_| Routed::Drop)),
        }
    }
    out
}

/// Routes already-fetched answers for one message, in answer order. Used both
/// by [`classify`] and when replaying cached answers at read time. Folder is always routed to
/// `Review` (never auto-applied) unless it is Off.
pub fn route(policy: &JudgePolicy, m: &Message, answers: Vec<(QuestionKey, Answer)>) -> Vec<Routed> {
    answers
        .into_iter()
        .map(|(key, answer)| {
            let s = Suggestion {
                message: m.id,
                key,
                answer,
            };
            match policy.mode(key) {
                _ if is_noop(m, &s) => Routed::Drop,
                Mode::Off => Routed::Drop,
                Mode::Auto(c)
                    if key != QuestionKey::Folder && s.answer.confidence >= c.threshold() =>
                {
                    Routed::Auto(s)
                }
                Mode::Auto(_) | Mode::Review => Routed::Review(s),
            }
        })
        .collect()
}
