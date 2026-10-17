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
//! | `Question::Score`  | `score`    | the level descriptions        | `Score(score + 1.0)` (Jev is 0-based)  |
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
mod age;
pub use age::{AGE_CHOICES, DEFAULT_MAX_AGE_DAYS, too_old_for_auto};
mod policy;
mod stub;

pub use jev::JevJudge;
pub use policy::*;
pub use stub::StubJudge;

use crate::model::{FolderId, Message, MessageId};

/// Maximum characters of message body sent to a judge. Quoted replies are kept; the body is
/// cut from the end.
pub const MAX_BODY_CHARS: usize = 8000;

#[derive(Clone, Debug, PartialEq)]
pub enum Question {
    /// Yes/no criteria. Jev `noul`.
    Bool { criteria: String },
    /// Pick one of `options`. Jev `choice`.
    Choice {
        criteria: String,
        options: Vec<String>,
    },
    /// Rate one of `levels`, each described. Jev `score`.
    Score {
        criteria: String,
        levels: Vec<String>,
    },
}

impl Question {
    /// Number of probabilities an answer to this question carries.
    pub fn arity(&self) -> usize {
        match self {
            Question::Bool { .. } => 2,
            Question::Choice { options, .. } => options.len(),
            Question::Score { levels, .. } => levels.len(),
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
    Folder,
}

impl QuestionKey {
    pub const ALL: [QuestionKey; 6] = [
        QuestionKey::Spam,
        QuestionKey::NeedsReply,
        QuestionKey::ExpectsReply,
        QuestionKey::Urgency,
        QuestionKey::Kind,
        QuestionKey::Folder,
    ];

    pub fn label(self) -> &'static str {
        match self {
            QuestionKey::Spam => "Spam",
            QuestionKey::NeedsReply => "Needs reply",
            QuestionKey::ExpectsReply => "Expects reply",
            QuestionKey::Urgency => "Urgency",
            QuestionKey::Kind => "Kind",
            QuestionKey::Folder => "Folder",
        }
    }
}

/// One stored answer to one question about one message, with the version of the question's
/// wording it answers. A question is only re-asked when its version changed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredAnswer {
    pub key: QuestionKey,
    pub version: String,
    pub answer: Answer,
}

/// The per-message classification status shown by the list row and the reader.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClassifyState {
    NotClassified,
    InProgress,
    Failed,
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

/// The name of an Urgency level (1..=5): the words the row menu and the palette use. Level 1 is
/// "None": it carries no label.
pub fn urgency_label(level: u8) -> &'static str {
    match level {
        1 => "None",
        2 => "Low",
        3 => "Normal",
        4 => "Urgent",
        _ => "Very urgent",
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
    /// A key is configured but automatic classification is off (stopped or never started).
    Paused,
    /// The current batch of classifier work: `done` of `total` requests have finished. The
    /// total grows if more work is queued mid-batch, and the batch resets when the queue drains.
    Working { done: usize, total: usize },
    /// The last request failed; the next success clears it.
    Error(String),
}

/// The one definition of the reply question. Incoming mail never asks it; only a reply does.
pub fn reply_question() -> (QuestionKey, Question) {
    (
        QuestionKey::ExpectsReply,
        Question::Bool {
            criteria: "Does this outgoing reply ask the recipient for a response?".into(),
        },
    )
}

/// One of a message's questions together with the version of its current wording.
#[derive(Clone, Debug, PartialEq)]
pub struct QuestionSpec {
    pub key: QuestionKey,
    pub version: String,
    pub question: Question,
}

/// The manual version of each question's wording. Bump only for a meaningful wording change.
const VERSION_SPAM: &str = "2";
const VERSION_NEEDS_REPLY: &str = "1";
const VERSION_URGENCY: &str = "2";
const VERSION_KIND: &str = "1";

/// The Spam criterion: real spam only; the cold-outreach checkbox widens it.
fn spam_criteria(cold_outreach: bool) -> String {
    let mut c = "The email is real spam: a scam, phishing, or unsolicited mass junk. \
                   Newsletters, promotions, receipts and notifications the recipient signed up \
                   for are not spam."
        .to_owned();
    if cold_outreach {
        c.push_str(
            " Recruiters and sales cold outreach from someone the recipient has never dealt with \
             also count as spam.",
        );
    }
    c
}

/// The five Urgency levels, sent to Jev as its `criteria` list (index + 1 is the level).
fn urgency_levels() -> Vec<String> {
    [
        "No action or time pressure.",
        "Worth reading sometime this week.",
        "Needs action within a few days.",
        "Needs action today, or a deadline in the next 24–48 hours.",
        "Something is blocked or broken, or a deadline within hours.",
    ]
    .iter()
    .map(ToString::to_string)
    .collect()
}

/// A stable fingerprint of an account's folder list, so a folder change re-asks only Folder.
pub fn folder_question_version(folders: &[(FolderId, String)]) -> String {
    let mut hasher = Sha256::new();
    for (_, path) in folders {
        hasher.update(path.as_bytes());
        hasher.update(b"\n");
    }
    format!("f{:x}", hasher.finalize())
}

/// The questions to ask about one incoming message: the enabled ones only, each with its
/// version. `folders` are the message account's `(id, path)` options; Folder is skipped when
/// the account has none.
pub fn incoming_questions(
    policy: &JudgePolicy,
    cold_outreach: bool,
    folders: &[(FolderId, String)],
) -> Vec<QuestionSpec> {
    fn push(
        out: &mut Vec<QuestionSpec>,
        policy: &JudgePolicy,
        key: QuestionKey,
        version: String,
        question: Question,
    ) {
        if policy.mode(key) != Mode::Off {
            out.push(QuestionSpec { key, version, question });
        }
    }
    let mut out = Vec::new();
    push(
        &mut out,
        policy,
        QuestionKey::Spam,
        if cold_outreach { format!("{VERSION_SPAM}c") } else { VERSION_SPAM.to_owned() },
        Question::Bool { criteria: spam_criteria(cold_outreach) },
    );
    push(
        &mut out,
        policy,
        QuestionKey::NeedsReply,
        VERSION_NEEDS_REPLY.to_owned(),
        Question::Bool {
            criteria: "The sender is a person who is asking the recipient a question or expecting a written response.".into(),
        },
    );
    push(
        &mut out,
        policy,
        QuestionKey::Urgency,
        VERSION_URGENCY.to_owned(),
        Question::Score {
            criteria: "How urgently does this email need the recipient's attention?".into(),
            levels: urgency_levels(),
        },
    );
    push(
        &mut out,
        policy,
        QuestionKey::Kind,
        VERSION_KIND.to_owned(),
        Question::Choice {
            criteria: "What kind of email is this: a person writing personally, a receipt or invoice, a newsletter, an automated notification, or something else?".into(),
            options: Kind::ALL.iter().map(|k| k.label().to_string()).collect(),
        },
    );
    if !folders.is_empty() {
        let mut options: Vec<String> = folders.iter().map(|(_, path)| path.clone()).collect();
        options.push("none".to_owned());
        push(
            &mut out,
            policy,
            QuestionKey::Folder,
            folder_question_version(folders),
            Question::Choice {
                criteria: "Which of the recipient's folders best fits this email, or none?".into(),
                options,
            },
        );
    }
    out
}

/// The subset of `specs` whose stored answer is missing or at a different version.
pub fn questions_to_ask(
    specs: &[QuestionSpec],
    stored: &[StoredAnswer],
) -> Vec<(QuestionKey, Question)> {
    specs
        .iter()
        .filter(|s| {
            stored
                .iter()
                .find(|a| a.key == s.key)
                .is_none_or(|a| a.version != s.version)
        })
        .map(|s| (s.key, s.question.clone()))
        .collect()
}

/// Determines whether an outgoing reply asks for a response.
pub fn expects_reply(judge: &dyn Judge, body: &str) -> bool {
    let state = json!({"sender": "", "subject": "", "body": body.trim()});
    judge
        .judge(&state, &[reply_question()])
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

/// The JSON state a judge sees: sender, recipients and role, subject, received date, bulk flag,
/// and the body (quoted replies kept, cut to [`MAX_BODY_CHARS`] from the end).
pub fn message_state(m: &Message, me: &str) -> Value {
    let text = crate::reading::reader_text(m);
    let body = text.trim();
    let body: String = match body.char_indices().nth(MAX_BODY_CHARS) {
        Some((end, _)) => body[..end].to_string(),
        None => body.to_string(),
    };
    json!({
        "sender": format!("{} <{}>", m.from_name, m.from_email),
        "to": m.to,
        "cc": m.cc,
        "recipient_role": recipient_role(m, me),
        "subject": m.subject,
        "received": m.received,
        "bulk": m.bulk,
        "body": body,
    })
}

/// "to" when the account's address is a direct recipient, "cc" when it is only copied,
/// "other" otherwise.
fn recipient_role(m: &Message, me: &str) -> &'static str {
    let me = me.trim();
    if me.is_empty() {
        return "other";
    }
    let has = |field: &str| {
        crate::reading::parse_recipients(field)
            .iter()
            .any(|r| r.email.eq_ignore_ascii_case(me))
    };
    if has(&m.to) {
        "to"
    } else if has(&m.cc) {
        "cc"
    } else {
        "other"
    }
}

/// The **legacy** hash key for one message's answers, exactly as the pre-`MIGRATION_4` cache
/// keyed rows. Only the one-time carry-over ([`crate::app::MailApp::carry_over_judgements`])
/// uses it; new rows are keyed by message id. Frozen so old rows still map.
pub fn legacy_judgement_key(model: &str, m: &Message) -> String {
    let state = legacy_state(m);
    let questions = legacy_questions();
    let body = jev::request_body(model, &state, &questions);
    let mut hasher = Sha256::new();
    hasher.update(model.as_bytes());
    hasher.update(b"\n");
    hasher.update(body["questions"].to_string().as_bytes());
    hasher.update(b"\n");
    hasher.update(state.to_string().as_bytes());
    format!("{:x}", hasher.finalize())
}

/// The pre-`StoredAnswer` questions, frozen so the legacy hash still reproduces.
fn legacy_questions() -> Vec<(QuestionKey, Question)> {
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
                levels: vec!["1".into(), "2".into(), "3".into(), "4".into(), "5".into()],
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

/// The pre-change state: sender, subject and a 1500-char body only, frozen for the legacy hash.
fn legacy_state(m: &Message) -> Value {
    let text = crate::reading::reader_text(m);
    let body = text.trim();
    let body: String = match body.char_indices().nth(1500) {
        Some((end, _)) => body[..end].to_string(),
        None => body.to_string(),
    };
    json!({
        "sender": format!("{} <{}>", m.from_name, m.from_email),
        "subject": m.subject,
        "body": body,
    })
}
