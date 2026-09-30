use std::collections::HashMap;

use super::*;

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

