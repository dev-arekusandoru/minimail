//! TypeSafe Jev provider over the single `/systemone` endpoint.
//!
//! All questions for one message go in one request; [`request_body`] and
//! [`parse_answers`] are pure, so the wire mapping is unit-testable without a
//! network.

use std::fmt;
use std::time::Duration;

use serde_json::{Map, Value, json};
use url::{Host, Url};

use super::{Answer, AnswerValue, Judge, JudgeError, Question, QuestionKey};

/// Where Jev lives when the user has not configured another base URL.
pub const DEFAULT_BASE_URL: &str = "https://api.typesafe.ai/v1";
/// Model alias; the API resolves it to a pinned version.
pub const DEFAULT_MODEL: &str = "jev-latest";

/// Whether `base` is an acceptable Jev endpoint: HTTPS anywhere, or plain HTTP on loopback
/// (`localhost`, `127.0.0.1`, `::1`) only, for a local proxy. Anything else would send the
/// bearer key and message text in the clear.
pub fn valid_base_url(base: &str) -> bool {
    Url::parse(base).is_ok_and(|url| url.scheme() == "https" || is_loopback_http(&url))
}

fn is_loopback_http(url: &Url) -> bool {
    url.scheme() == "http" && is_loopback(url)
}

/// Exact loopback hosts, not the whole `127.0.0.0/8` range.
fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain("localhost")) => true,
        Some(Host::Ipv4(ip)) => ip == std::net::Ipv4Addr::LOCALHOST,
        Some(Host::Ipv6(ip)) => ip == std::net::Ipv6Addr::LOCALHOST,
        _ => false,
    }
}

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
        // Plain HTTP is only ever the loopback case the validator allows; every other host is
        // pinned to HTTPS, and redirects are off so a response cannot downgrade the transport.
        let https_only = !Url::parse(&self.base_url).is_ok_and(|u| is_loopback_http(&u));
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .https_only(https_only)
            .max_redirects(0)
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .into();
        let mut res = agent
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.key))
            .send_json(request_body(&self.model, state, questions))
            .map_err(|e| JudgeError::Unavailable(e.to_string()))?;
        let status = res.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(JudgeError::Unavailable(failure(status)));
        }
        let text = res
            .body_mut()
            .read_to_string()
            .map_err(|e| JudgeError::Unavailable(e.to_string()))?;
        let body: Value = serde_json::from_str(&text)
            .map_err(|_| JudgeError::Malformed("response is not JSON".into()))?;
        parse_answers(&body, questions)
    }
}

/// A human-readable reason for a non-2xx response. Status-only: the server's body is never
/// echoed, so an endpoint that mirrors the request cannot leak the key into the UI.
fn failure(status: u16) -> String {
    match status {
        401 | 403 => "invalid API key".to_owned(),
        429 => "rate limited".to_owned(),
        _ => format!("Jev returned HTTP {status}"),
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
                json!({"type": "score", "instructions": criteria, "criteria": levels})
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
                probabilities: (0..levels.len())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_requires_https_except_loopback() {
        for ok in [
            "https://api.typesafe.ai/v1",
            "http://localhost:8080",
            "http://127.0.0.1:11434/v1",
            "http://[::1]:8080",
        ] {
            assert!(valid_base_url(ok), "{ok} should be accepted");
        }
        for bad in [
            "http://api.typesafe.ai/v1",
            "http://192.168.1.10:8080",
            "http://[::2]:8080",
            "ftp://localhost",
            "not a url",
        ] {
            assert!(!valid_base_url(bad), "{bad} should be rejected");
        }
    }

    #[test]
    fn failure_is_status_only() {
        assert_eq!(failure(500), "Jev returned HTTP 500");
        assert_eq!(failure(401), "invalid API key");
        assert_eq!(failure(429), "rate limited");
    }
}
