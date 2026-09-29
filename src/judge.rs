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

use std::collections::HashMap;
use std::fmt;

use serde_json::{Value, json};

use crate::model::{Message, MessageId, TriageState};

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

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum QuestionKey {
    Spam,
    NeedsReply,
    SuggestedState,
    Urgency,
    Kind,
}

impl QuestionKey {
    pub const ALL: [QuestionKey; 5] = [
        QuestionKey::Spam,
        QuestionKey::NeedsReply,
        QuestionKey::SuggestedState,
        QuestionKey::Urgency,
        QuestionKey::Kind,
    ];

    pub fn label(self) -> &'static str {
        match self {
            QuestionKey::Spam => "Spam",
            QuestionKey::NeedsReply => "Needs reply",
            QuestionKey::SuggestedState => "Suggested state",
            QuestionKey::Urgency => "Urgency",
            QuestionKey::Kind => "Kind",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
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

/// Options of the SuggestedState question, in `TriageState::ALL` order.
fn state_options() -> Vec<String> {
    TriageState::ALL
        .iter()
        .map(|s| s.label().to_string())
        .collect()
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
            QuestionKey::SuggestedState,
            Question::Choice {
                criteria: "Which triage state fits this email best: needs attention now (Inbox), awaiting others (Waiting), can be read later (Later), or needs nothing more (Done)?".into(),
                options: state_options(),
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

/// The JSON state a judge sees: sender, subject and a trimmed body only.
pub fn message_state(m: &Message) -> Value {
    let body = m.body.trim();
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

// ---------------------------------------------------------------------------
// Stub judge
// ---------------------------------------------------------------------------

/// Deterministic keyword/heuristic judge. No randomness, no network.
/// Tuned to `fixtures/mailbox.json`.
#[derive(Clone, Copy, Debug, Default)]
pub struct StubJudge;

/// Signals extracted once per message.
struct Features {
    text: String,
    kind: Kind,
    spam_p: f32,
}

const AUTOMATED_LOCALS: [&str; 12] = [
    "no-reply",
    "noreply",
    "notifications",
    "alerts",
    "service",
    "calendar-notify",
    "receipts",
    "billing",
    "statements",
    "digest",
    "editors",
    "newsletter",
];

fn sender_email(sender: &str) -> String {
    match (sender.rfind('<'), sender.rfind('>')) {
        (Some(a), Some(b)) if a < b => sender[a + 1..b].trim().to_lowercase(),
        _ => sender.trim().to_lowercase(),
    }
}

fn has_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| text.contains(n))
}

impl Features {
    fn from_state(state: &Value) -> Features {
        let field = |k: &str| state.get(k).and_then(Value::as_str).unwrap_or("");
        let email = sender_email(field("sender"));
        let subject = field("subject").to_lowercase();
        let text = format!("{subject}\n{}", field("body").to_lowercase());
        let (local, domain) = email.split_once('@').unwrap_or((email.as_str(), ""));

        let spam_p = if domain == "talentloop.com" {
            0.96
        } else if domain == "hirestream.io" {
            if AUTOMATED_LOCALS.contains(&local) {
                0.6
            } else {
                0.92
            }
        } else if has_any(&text, &["is this a fit", "fit check", "hiring lead"]) {
            0.9
        } else {
            0.03
        };

        let kind = if spam_p >= 0.5 {
            Kind::Other
        } else if AUTOMATED_LOCALS.contains(&local) {
            if matches!(local, "receipts" | "billing" | "statements") {
                Kind::Receipt
            } else if matches!(local, "digest" | "editors" | "newsletter") {
                Kind::Newsletter
            } else if has_any(
                &subject,
                &["receipt", "invoice", "statement", "charged", "payment"],
            ) {
                Kind::Receipt
            } else {
                Kind::Notification
            }
        } else {
            Kind::Person
        };

        Features {
            text,
            kind,
            spam_p,
        }
    }

    fn needs_reply_p(&self) -> f32 {
        if self.kind != Kind::Person {
            return 0.04;
        }
        if has_any(
            &self.text,
            &[
                "?",
                "let me know",
                "need a number",
                "can you",
                "could you",
                "please reply",
            ],
        ) {
            0.88
        } else {
            0.3
        }
    }

    /// (level 1..=5, sharpness) — lower sharpness = more spread.
    fn urgency(&self, needs_reply: bool) -> (usize, f32) {
        if self.spam_p >= 0.5 {
            return (1, 0.1);
        }
        if has_any(
            &self.text,
            &["asap", "urgent", "failed", "deadline", "due friday", "new sign-in"],
        ) {
            return (4, 0.2);
        }
        match self.kind {
            Kind::Newsletter => (1, 0.1),
            Kind::Receipt => (1, 0.3),
            Kind::Notification if has_any(&self.text, &["tomorrow", "reminder", "declined"]) => {
                (3, 0.3)
            }
            Kind::Notification => (2, 0.3),
            Kind::Person if needs_reply && has_any(&self.text, &["today", "tomorrow", "by "]) => {
                (4, 0.25)
            }
            Kind::Person if needs_reply => (3, 0.3),
            _ => (2, 0.35),
        }
    }

    /// (state, confidence)
    fn suggested_state(&self, needs_reply: bool) -> (TriageState, f32) {
        if self.spam_p >= 0.5 {
            return (TriageState::Done, 0.8);
        }
        match self.kind {
            Kind::Newsletter => (TriageState::Later, 0.7),
            Kind::Receipt => (TriageState::Done, 0.65),
            Kind::Notification if has_any(&self.text, &["failed", "new sign-in", "tomorrow"]) => {
                (TriageState::Inbox, 0.75)
            }
            Kind::Notification => (TriageState::Done, 0.55),
            Kind::Person if needs_reply => (TriageState::Inbox, 0.85),
            _ => (TriageState::Later, 0.45),
        }
    }
}

/// `n` probabilities, `top` on `idx`, remainder spread evenly.
fn peaked(n: usize, idx: usize, top: f32) -> Vec<f32> {
    let rest = (1.0 - top) / (n - 1) as f32;
    (0..n).map(|i| if i == idx { top } else { rest }).collect()
}

/// Geometrically decaying distribution around level index `idx`, normalised.
fn spread(n: usize, idx: usize, sharpness: f32) -> Vec<f32> {
    let w: Vec<f32> = (0..n)
        .map(|i| sharpness.powi(i.abs_diff(idx) as i32))
        .collect();
    let sum: f32 = w.iter().sum();
    w.into_iter().map(|x| x / sum).collect()
}

fn argmax(p: &[f32]) -> usize {
    let mut best = 0;
    for (i, v) in p.iter().enumerate() {
        if *v > p[best] {
            best = i;
        }
    }
    best
}

fn bool_answer(p_yes: f32) -> Answer {
    let probabilities = vec![1.0 - p_yes, p_yes];
    let yes = p_yes > 0.5;
    Answer {
        confidence: if yes { p_yes } else { 1.0 - p_yes },
        value: AnswerValue::Bool(yes),
        probabilities,
    }
}

fn choice_answer(probabilities: Vec<f32>) -> Answer {
    let i = argmax(&probabilities);
    Answer {
        confidence: probabilities[i],
        value: AnswerValue::Choice(i),
        probabilities,
    }
}

fn score_answer(probabilities: Vec<f32>) -> Answer {
    let weighted = probabilities
        .iter()
        .enumerate()
        .map(|(i, p)| (i + 1) as f32 * p)
        .sum();
    Answer {
        confidence: probabilities[argmax(&probabilities)],
        value: AnswerValue::Score(weighted),
        probabilities,
    }
}

impl StubJudge {
    fn answer(f: &Features, key: QuestionKey, q: &Question) -> Result<Answer, JudgeError> {
        let needs_reply = f.needs_reply_p() > 0.5;
        let answer = match (key, q) {
            (QuestionKey::Spam, Question::Bool { .. }) => bool_answer(f.spam_p),
            (QuestionKey::NeedsReply, Question::Bool { .. }) => bool_answer(f.needs_reply_p()),
            (QuestionKey::SuggestedState, Question::Choice { options, .. }) => {
                let (state, conf) = f.suggested_state(needs_reply);
                let idx = TriageState::ALL
                    .iter()
                    .position(|s| *s == state)
                    .filter(|i| *i < options.len())
                    .ok_or_else(|| JudgeError::Malformed("state options".into()))?;
                choice_answer(peaked(options.len(), idx, conf))
            }
            (QuestionKey::Urgency, Question::Score { levels, .. }) if *levels >= 2 => {
                let (level, sharp) = f.urgency(needs_reply);
                let n = *levels as usize;
                score_answer(spread(n, (level - 1).min(n - 1), sharp))
            }
            (QuestionKey::Kind, Question::Choice { options, .. }) if options.len() == 5 => {
                let idx = Kind::ALL.iter().position(|k| *k == f.kind).unwrap_or(4);
                let top = match f.kind {
                    Kind::Other => 0.55,
                    Kind::Person => 0.8,
                    _ => 0.9,
                };
                choice_answer(peaked(5, idx, top))
            }
            _ => {
                return Err(JudgeError::Malformed(format!(
                    "question shape does not fit {key:?}"
                )));
            }
        };
        Ok(answer)
    }
}

impl Judge for StubJudge {
    fn judge(
        &self,
        state: &Value,
        questions: &[(QuestionKey, Question)],
    ) -> Result<Vec<(QuestionKey, Answer)>, JudgeError> {
        let f = Features::from_state(state);
        questions
            .iter()
            .map(|(k, q)| Ok((*k, Self::answer(&f, *k, q)?)))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Policy + routing
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mode {
    /// Apply automatically when `confidence >= threshold`; otherwise queue for review.
    Auto { threshold: f32 },
    /// Always queue for review.
    Review,
}

/// Per-question routing mode. Remembers each question's last threshold so
/// toggling Auto -> Review -> Auto restores it.
#[derive(Clone, Debug, PartialEq)]
pub struct JudgePolicy {
    modes: HashMap<QuestionKey, Mode>,
    thresholds: HashMap<QuestionKey, f32>,
}

impl Default for JudgePolicy {
    fn default() -> Self {
        let defaults = [
            (QuestionKey::Spam, Mode::Auto { threshold: 0.9 }),
            (QuestionKey::SuggestedState, Mode::Review),
            (QuestionKey::NeedsReply, Mode::Auto { threshold: 0.8 }),
            (QuestionKey::Urgency, Mode::Auto { threshold: 0.7 }),
            (QuestionKey::Kind, Mode::Auto { threshold: 0.6 }),
        ];
        let thresholds = defaults
            .iter()
            .map(|(k, m)| {
                let t = match m {
                    Mode::Auto { threshold } => *threshold,
                    Mode::Review => 0.8,
                };
                (*k, t)
            })
            .collect();
        JudgePolicy {
            modes: defaults.into_iter().collect(),
            thresholds,
        }
    }
}

impl JudgePolicy {
    pub fn mode(&self, key: QuestionKey) -> Mode {
        self.modes.get(&key).copied().unwrap_or(Mode::Review)
    }

    pub fn set_mode(&mut self, key: QuestionKey, mode: Mode) {
        if let Mode::Auto { threshold } = mode {
            self.thresholds.insert(key, threshold.clamp(0.0, 1.0));
        }
        let mode = match mode {
            Mode::Auto { threshold } => Mode::Auto {
                threshold: threshold.clamp(0.0, 1.0),
            },
            Mode::Review => Mode::Review,
        };
        self.modes.insert(key, mode);
    }

    /// Threshold last used (or currently used) for `key`.
    pub fn threshold(&self, key: QuestionKey) -> f32 {
        self.thresholds.get(&key).copied().unwrap_or(0.8)
    }

    /// Sets the threshold, clamped to `0.0..=1.0`. Under Review it is stored and
    /// takes effect when the question is switched back to Auto.
    pub fn set_threshold(&mut self, key: QuestionKey, threshold: f32) {
        let t = threshold.clamp(0.0, 1.0);
        self.thresholds.insert(key, t);
        if let Some(Mode::Auto { .. }) = self.modes.get(&key) {
            self.modes.insert(key, Mode::Auto { threshold: t });
        }
    }

    /// Adjusts the threshold by `delta`, rounded to 0.01 to avoid float drift.
    pub fn nudge_threshold(&mut self, key: QuestionKey, delta: f32) {
        let t = ((self.threshold(key) + delta) * 100.0).round() / 100.0;
        self.set_threshold(key, t);
    }

    /// Auto <-> Review, keeping the remembered threshold.
    pub fn toggle_mode(&mut self, key: QuestionKey) {
        let next = match self.mode(key) {
            Mode::Auto { .. } => Mode::Review,
            Mode::Review => Mode::Auto {
                threshold: self.threshold(key),
            },
        };
        self.modes.insert(key, next);
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

    /// Triage state carried by a SuggestedState answer.
    pub fn state(&self) -> Option<TriageState> {
        match (self.key, &self.answer.value) {
            (QuestionKey::SuggestedState, AnswerValue::Choice(i)) => {
                TriageState::ALL.get(*i).copied()
            }
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

/// True when applying this answer would change nothing.
fn is_noop(m: &Message, s: &Suggestion) -> bool {
    match (&s.key, &s.answer.value) {
        (QuestionKey::Spam | QuestionKey::NeedsReply, AnswerValue::Bool(b)) => !*b,
        (QuestionKey::SuggestedState, AnswerValue::Choice(i)) => {
            TriageState::ALL.get(*i).is_none_or(|st| *st == m.state)
        }
        (QuestionKey::Urgency, _) => s.urgency().is_none_or(|u| u <= 1),
        (QuestionKey::Kind, AnswerValue::Choice(i)) => {
            Kind::from_index(*i).is_none_or(|k| k == Kind::Other)
        }
        _ => true,
    }
}

/// Runs all triage questions over each message and routes every answer.
/// Output is flat: for each message (in order), one entry per triage question
/// (in `triage_questions` order). A judge error yields `Drop` for that message.
pub fn classify(judge: &dyn Judge, policy: &JudgePolicy, msgs: &[&Message]) -> Vec<Routed> {
    let questions = triage_questions();
    let mut out = Vec::with_capacity(msgs.len() * questions.len());
    for m in msgs {
        let answers = match judge.judge(&message_state(m), &questions) {
            Ok(a) => a,
            Err(_) => {
                out.extend(questions.iter().map(|_| Routed::Drop));
                continue;
            }
        };
        for (key, answer) in answers {
            let s = Suggestion {
                message: m.id,
                key,
                answer,
            };
            out.push(if is_noop(m, &s) {
                Routed::Drop
            } else {
                match policy.mode(key) {
                    Mode::Auto { threshold } if s.answer.confidence >= threshold => Routed::Auto(s),
                    _ => Routed::Review(s),
                }
            });
        }
    }
    out
}
