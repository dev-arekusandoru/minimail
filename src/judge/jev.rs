//! TypeSafe Jev provider over the single `/systemone` endpoint.
//!
//! All questions for one message go in one request; [`request_body`] and
//! [`parse_answers`] are pure, so the wire mapping is unit-testable without a
//! network.

use std::fmt;
use std::time::Duration;

use serde_json::{Map, Value, json};

use super::{Answer, AnswerValue, Judge, JudgeError, Question, QuestionKey};

/// Where Jev lives when the user has not configured another base URL.
pub const DEFAULT_BASE_URL: &str = "https://api.typesafe.ai/v1";
/// Model alias; the API resolves it to a pinned version.
pub const DEFAULT_MODEL: &str = "jev-latest";

/// One request's worth of answers, per the Jev API.
pub struct JevJudge {
    base_url: String,
    model: String,
    key: String,
}

impl JevJudge {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>, key: impl Into<String>) -> Self {
        JevJudge {
            base_url: base_url.into(),
            model: model.into(),
            key: key.into(),
        }
    }

    /// The model alias this judge sends (e.g. `jev-latest`).
    pub fn model(&self) -> &str {
        &self.model
    }
}

impl fmt::Debug for JevJudge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never let the key reach a log line.
        f.debug_struct("JevJudge")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("key", &"<redacted>")
            .finish()
    }
}

impl Judge for JevJudge {
    fn model(&self) -> &str {
        &self.model
    }

    fn judge(
        &self,
        state: &Value,
        questions: &[(QuestionKey, Question)],
    ) -> Result<Vec<(QuestionKey, Answer)>, JudgeError> {
        let url = format!("{}/systemone", self.base_url.trim_end_matches('/'));
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .into();
        let mut res = agent
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.key))
            .send_json(request_body(&self.model, state, questions))
            .map_err(|e| JudgeError::Unavailable(e.to_string()))?;
        let status = res.status().as_u16();
        let text = res
            .body_mut()
            .read_to_string()
            .map_err(|e| JudgeError::Unavailable(e.to_string()))?;
        if !(200..300).contains(&status) {
            return Err(JudgeError::Unavailable(failure(status, &text)));
        }
        let body: Value = serde_json::from_str(&text)
            .map_err(|_| JudgeError::Malformed("response is not JSON".into()))?;
        parse_answers(&body, questions)
    }
}

/// A human-readable reason for a non-2xx response; never includes the key.
fn failure(status: u16, body: &str) -> String {
    match status {
        401 | 403 => "invalid API key".to_owned(),
        429 => "rate limited".to_owned(),
        _ => {
            let excerpt: String = body.trim().chars().take(200).collect();
            if excerpt.is_empty() {
                format!("Jev returned HTTP {status}")
            } else {
                format!("Jev returned HTTP {status}: {excerpt}")
            }
        }
    }
}

/// The `/systemone` request body for `questions` over `state`.
pub fn request_body(model: &str, state: &Value, questions: &[(QuestionKey, Question)]) -> Value {
    let mut map = Map::new();
    for (key, question) in questions {
        let entry = match question {
            Question::Bool { criteria } => json!({"type": "noul", "instructions": criteria}),
            Question::Choice { criteria, options } => {
                let rubric: Map<String, Value> = options
                    .iter()
                    .map(|o| (o.clone(), Value::String(o.clone())))
                    .collect();
                json!({"type": "choice", "instructions": criteria, "criteria": rubric})
            }
            Question::Score { criteria, levels } => {
                let rubric: Vec<String> = (1..=*levels).map(|i| i.to_string()).collect();
                json!({"type": "score", "instructions": criteria, "criteria": rubric})
            }
        };
        map.insert(format!("{key:?}"), entry);
    }
    json!({"state": state, "model": model, "questions": map})
}

/// The API response body → one answer per question, in question order.
pub fn parse_answers(
    body: &Value,
    questions: &[(QuestionKey, Question)],
) -> Result<Vec<(QuestionKey, Answer)>, JudgeError> {
    let answers = body
        .get("answers")
        .and_then(Value::as_object)
        .ok_or_else(|| JudgeError::Malformed("response has no `answers` object".into()))?;
    questions
        .iter()
        .map(|(key, question)| {
            let name = format!("{key:?}");
            let raw = answers
                .get(&name)
                .ok_or_else(|| JudgeError::Malformed(format!("no answer for `{name}`")))?;
            let answer = parse_one(raw, question)
                .map_err(|why| JudgeError::Malformed(format!("`{name}`: {why}")))?;
            Ok((*key, answer))
        })
        .collect()
}

fn parse_one(raw: &Value, question: &Question) -> Result<Answer, String> {
    match question {
        Question::Bool { .. } => {
            expect_type(raw, "noul")?;
            let p = number(raw, "noul")?;
            Ok(Answer {
                probabilities: vec![1.0 - p, p],
                value: AnswerValue::Bool(p > 0.5),
                confidence: p.max(1.0 - p),
            })
        }
        Question::Choice { options, .. } => {
            expect_type(raw, "choice")?;
            let choice = raw
                .get("choice")
                .and_then(Value::as_str)
                .ok_or("choice answer has no `choice`")?;
            let index = options
                .iter()
                .position(|o| o == choice)
                .ok_or_else(|| format!("unknown choice `{choice}`"))?;
            let probabilities = raw
                .get("probabilities")
                .and_then(Value::as_object)
                .ok_or("choice answer has no `probabilities`")?;
            Ok(Answer {
                probabilities: options
                    .iter()
                    .map(|o| opt_number(probabilities, o))
                    .collect(),
                value: AnswerValue::Choice(index),
                confidence: number(raw, "confidence")?,
            })
        }
        Question::Score { levels, .. } => {
            expect_type(raw, "score")?;
            let score = number(raw, "score")?;
            let probabilities = raw
                .get("probabilities")
                .and_then(Value::as_object)
                .ok_or("score answer has no `probabilities`")?;
            Ok(Answer {
                probabilities: (0..*levels)
                    .map(|i| opt_number(probabilities, &i.to_string()))
                    .collect(),
                // Jev scores are 0-based; ours start at 1.
                value: AnswerValue::Score(score + 1.0),
                confidence: number(raw, "confidence")?,
            })
        }
    }
}

fn expect_type(raw: &Value, want: &str) -> Result<(), String> {
    match raw.get("type").and_then(Value::as_str) {
        Some(t) if t == want => Ok(()),
        _ => Err(format!("expected a `{want}` answer")),
    }
}

fn number(raw: &Value, field: &str) -> Result<f32, String> {
    raw.get(field)
        .and_then(Value::as_f64)
        .map(|n| n as f32)
        .ok_or_else(|| format!("answer has no `{field}`"))
}

/// A probability that may be absent (Jev omits zero-probability options).
fn opt_number(map: &Map<String, Value>, key: &str) -> f32 {
    map.get(key).and_then(Value::as_f64).unwrap_or(0.0) as f32
}
