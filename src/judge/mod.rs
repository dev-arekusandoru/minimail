//! System 1 classifier interface, shaped after the TypeSafe AI "Jev" API, plus
//! a deterministic offline [`StubJudge`].
//!
//! # Mapping to TypeSafe Jev (for the future real provider)
//!
//! Jev answers natural-language *criteria* about a JSON `state`, returning
//! per-option probabilities rather than free text. Our [`Question`] variants
//! map one-to-one onto its endpoints:
//!
//! | here                | Jev endpoint | request                          | response used                          |
//! |---------------------|--------------|----------------------------------|----------------------------------------|
//! | `Question::Bool`    | `noul`       | `state`, `criteria`              | `[p_no, p_yes]`, argmax -> `Bool`      |
//! | `Question::Choice`  | `choice`     | `state`, `criteria`, `options`   | one probability per option, argmax     |
//! | `Question::Score`   | `score`      | `state`, `criteria`, `levels`    | one probability per level; `Score` is  |
//! |                     |              |                                  | the probability-weighted level (1-based)|
//!
//! `Answer::confidence` is the probability of the winning option (for scores:
//! of the most likely level). A real provider implements [`Judge`] by sending
//! [`message_state`] as `state` and each question's `criteria`; nothing else in
//! the app changes. Batching all questions for a message in one `judge` call
//! lets a provider share the state upload.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

mod policy;
mod stub;

pub use policy::*;
pub use stub::StubJudge;

use crate::model::{Message, MessageId};

/// Maximum characters of message body sent to a judge.
pub const MAX_BODY_CHARS: usize = 1500;

#[derive(Clone, Debug, PartialEq)]
pub enum Question {
    /// Yes/no criteria. Jev `noul`.
    Bool { criteria: String },
    /// Pick one of `options`. Jev `choice`.
    Choice {
        criteria: String,
        options: Vec<String>,
    },
    /// Rate 1..=`levels`. Jev `score`.
    Score { criteria: String, levels: u8 },
}

impl Question {
    /// Number of probabilities an answer to this question carries.
    pub fn arity(&self) -> usize {
        match self {
            Question::Bool { .. } => 2,
            Question::Choice { options, .. } => options.len(),
            Question::Score { levels, .. } => *levels as usize,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AnswerValue {
    Bool(bool),
    /// Index into the question's options.
    Choice(usize),
    /// Probability-weighted level, in `1.0..=levels`.
    Score(f32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Answer {
    /// Bool = `[p_no, p_yes]`; Choice = per option; Score = per level (level 1 first).
    pub probabilities: Vec<f32>,
    pub value: AnswerValue,
    /// Probability of the winning option / most likely level.
    pub confidence: f32,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
pub enum QuestionKey {
    Spam,
    NeedsReply,
    ExpectsReply,
    Urgency,
    Kind,
}

impl QuestionKey {
    pub const ALL: [QuestionKey; 5] = [
        QuestionKey::Spam,
        QuestionKey::NeedsReply,
        QuestionKey::ExpectsReply,
        QuestionKey::Urgency,
        QuestionKey::Kind,
    ];

    pub fn label(self) -> &'static str {
        match self {
            QuestionKey::Spam => "Spam",
            QuestionKey::NeedsReply => "Needs reply",
            QuestionKey::ExpectsReply => "Expects reply",
            QuestionKey::Urgency => "Urgency",
            QuestionKey::Kind => "Kind",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum Kind {
    Person,
    Receipt,
    Newsletter,
    Notification,
    Other,
}

impl Kind {
    /// Order matches the options of the Kind question.
    pub const ALL: [Kind; 5] = [
        Kind::Person,
        Kind::Receipt,
        Kind::Newsletter,
        Kind::Notification,
        Kind::Other,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Kind::Person => "person",
            Kind::Receipt => "receipt",
            Kind::Newsletter => "newsletter",
            Kind::Notification => "notification",
            Kind::Other => "other",
        }
    }

    pub fn from_index(i: usize) -> Option<Kind> {
        Kind::ALL.get(i).copied()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JudgeError {
    /// The provider could not be reached or failed.
    Unavailable(String),
    /// The provider returned something that does not fit the questions.
    Malformed(String),
}

impl fmt::Display for JudgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JudgeError::Unavailable(m) => write!(f, "judge unavailable: {m}"),
            JudgeError::Malformed(m) => write!(f, "malformed judge answer: {m}"),
        }
    }
}

impl std::error::Error for JudgeError {}

/// Answers questions about a JSON state. Answers come back in question order.
pub trait Judge {
    fn judge(
        &self,
        state: &Value,
        questions: &[(QuestionKey, Question)],
    ) -> Result<Vec<(QuestionKey, Answer)>, JudgeError>;
}

/// The five triage questions with literal one-sentence criteria.
pub fn triage_questions() -> Vec<(QuestionKey, Question)> {
    vec![
        (
            QuestionKey::Spam,
            Question::Bool {
                criteria: "The email is unsolicited bulk, recruiting or promotional outreach the recipient never asked for.".into(),
            },
        ),
        (
            QuestionKey::NeedsReply,
            Question::Bool {
                criteria: "The sender is a person who is asking the recipient a question or expecting a written response.".into(),
            },
        ),
        (
            QuestionKey::ExpectsReply,
            Question::Bool {
                criteria: "Does this outgoing reply ask the recipient for a response?".into(),
            },
        ),
        (
            QuestionKey::Urgency,
            Question::Score {
                criteria: "How urgently does this email need the recipient's attention, from 1 (not at all) to 5 (immediately)?".into(),
                levels: 5,
            },
        ),
        (
            QuestionKey::Kind,
            Question::Choice {
                criteria: "What kind of email is this: a person writing personally, a receipt or invoice, a newsletter, an automated notification, or something else?".into(),
                options: Kind::ALL.iter().map(|k| k.label().to_string()).collect(),
            },
        ),
    ]
}

/// Determines whether an outgoing reply asks for a response.
pub fn expects_reply(judge: &dyn Judge, body: &str) -> bool {
    let state = json!({"sender": "", "subject": "", "body": body.trim()});
    let question = Question::Bool {
        criteria: "Does this outgoing reply ask the recipient for a response?".into(),
    };
    judge
        .judge(&state, &[(QuestionKey::ExpectsReply, question)])
        .ok()
        .and_then(|answers| {
            answers
                .into_iter()
                .find(|(key, _)| *key == QuestionKey::ExpectsReply)
        })
        .and_then(|(_, answer)| match answer.value {
            AnswerValue::Bool(value) => Some(value),
            _ => None,
        })
        .unwrap_or(false)
}

/// The JSON state a judge sees: sender, subject and a trimmed body only (the text part,
/// or text extracted from the HTML for HTML-only mail).
pub fn message_state(m: &Message) -> Value {
    let text = crate::reading::reader_text(m);
    let body = text.trim();
    let body: String = match body.char_indices().nth(MAX_BODY_CHARS) {
        Some((end, _)) => body[..end].to_string(),
        None => body.to_string(),
    };
    json!({
        "sender": format!("{} <{}>", m.from_name, m.from_email),
        "subject": m.subject,
        "body": body,
    })
}
