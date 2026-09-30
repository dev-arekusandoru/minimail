use serde_json::Value;

use super::*;

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
