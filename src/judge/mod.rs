//! System 1 classifier interface: one judge call per message, backed by a
//! provider, plus a deterministic offline [`StubJudge`].
//!
//! # Mapping to TypeSafe Jev
//!
//! [`JevJudge`] sends every question for a message in one `POST
//! {base_url}/systemone` request whose `state` is [`message_state`] and whose
//! `questions` map is keyed by the stable `Debug` name of each
//! [`QuestionKey`] (e.g. `"Spam"`). Our [`Question`] variants map as:
//!
//! | here               | Jev `type` | request `criteria`            | answer → [`Answer`]                    |
//! |--------------------|------------|-------------------------------|----------------------------------------|
//! | `Question::Bool`   | `noul`     | —                             | `p = noul`; `Bool(p > 0.5)`; conf `max(p, 1-p)` |
//! | `Question::Choice` | `choice`   | `{option: option}`            | `Choice(index of returned choice)`     |
//! | `Question::Score`  | `score`    | `["1", .., "levels"]`         | `Score(score + 1.0)` (Jev is 0-based)  |
//!
//! `Answer::confidence` is Jev's `confidence` for choice/score answers, and
//! `max(p, 1-p)` for noul (Jev returns none). See [`jev`] for the wire format.

use std::fmt;
use std::rc::Rc;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// Jev wire format: the `/systemone` request/response mapping.
pub mod jev;
mod policy;
mod stub;

pub use jev::JevJudge;
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

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum AnswerValue {
    Bool(bool),
    /// Index into the question's options.
    Choice(usize),
    /// Probability-weighted level, in `1.0..=levels`.
    Score(f32),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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

    /// The model this judge uses; part of the judgement cache key.
    fn model(&self) -> &str {
        "stub"
    }
}

/// Builds a judge from `(base_url, model, key)`. The app holds one so tests can swap the real
/// [`JevJudge`] for a fake.
pub type JudgeFactory = Rc<dyn Fn(&str, &str, &str) -> Arc<dyn Judge + Send + Sync>>;

/// The classifier's state, for the status bar and the settings UI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JudgeStatus {
    /// No API key is configured: classification is off.
    NoKey,
    /// A key is configured and nothing is queued.
    Idle,
    /// `n` messages are waiting to be judged, including the one in flight.
    Working(usize),
    /// The last request failed; the next success clears it.
    Error(String),
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

/// Cache key for one message's answers: a hex SHA-256 over the model, the
/// request's questions JSON, and the state JSON. Editing a question's criteria
/// (or the state) changes the key, so stale answers are never reused.
pub fn judgement_key(model: &str, state: &Value, questions: &[(QuestionKey, Question)]) -> String {
    let body = jev::request_body(model, state, questions);
    let mut hasher = Sha256::new();
    hasher.update(model.as_bytes());
    hasher.update(b"\n");
    hasher.update(body["questions"].to_string().as_bytes());
    hasher.update(b"\n");
    hasher.update(state.to_string().as_bytes());
    format!("{:x}", hasher.finalize())
}
