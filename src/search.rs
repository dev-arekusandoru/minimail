//! Search query parsing and matching for the palette.
//!
//! Syntax (terms are ANDed, everything case-insensitive):
//! `from:x` (name or email substring), `subject:x`, `is:inbox|waiting|later|done|screener`,
//! `before:YYYY-MM-DD` (exclusive), `after:YYYY-MM-DD` (inclusive), `"quoted phrase"`,
//! and free text over subject + body + sender. Values may be quoted (`subject:"a b"`).
//! Unknown keys, empty values and malformed dates degrade to free text.
//! Dates are compared against the date part of the RFC3339 `received` field
//! (no timezone conversion). A leading `/` is ignored.

use crate::model::{Message, TriageState};

const KEYS: [&str; 5] = ["from", "subject", "is", "before", "after"];

#[derive(Clone, Debug, PartialEq, Eq)]
enum Is {
    State(TriageState),
    Screener,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    from: Vec<String>,
    subject: Vec<String>,
    is: Vec<Is>,
    before: Vec<String>,
    after: Vec<String>,
    text: Vec<String>,
}

fn tokenize(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut any = false;
    for c in input.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any {
                    tokens.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            c => {
                cur.push(c);
                any = true;
            }
        }
    }
    if any {
        tokens.push(cur);
    }
    tokens
}

fn valid_date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
    if !(digits(0..4) && digits(5..7) && digits(8..10)) {
        return false;
    }
    let month: u32 = s[5..7].parse().unwrap_or(0);
    let day: u32 = s[8..10].parse().unwrap_or(0);
    (1..=12).contains(&month) && (1..=31).contains(&day)
}

fn is_key(token: &str) -> Option<(&'static str, &str)> {
    let (k, v) = token.split_once(':')?;
    let k = KEYS.iter().find(|key| key.eq_ignore_ascii_case(k))?;
    Some((k, v))
}

impl Query {
    pub fn parse(input: &str) -> Query {
        let input = input.trim_start();
        let input = input.strip_prefix('/').unwrap_or(input);
        let mut q = Query::default();
        for token in tokenize(input) {
            let lower = token.to_lowercase();
            let Some((key, value)) = is_key(&lower).filter(|(_, v)| !v.is_empty()) else {
                q.text.push(lower);
                continue;
            };
            let value = value.to_string();
            match key {
                "from" => q.from.push(value),
                "subject" => q.subject.push(value),
                "is" => match value.as_str() {
                    "inbox" => q.is.push(Is::State(TriageState::Inbox)),
                    "waiting" => q.is.push(Is::State(TriageState::Waiting)),
                    "later" => q.is.push(Is::State(TriageState::Later)),
                    "done" => q.is.push(Is::State(TriageState::Done)),
                    "screener" => q.is.push(Is::Screener),
                    _ => q.text.push(lower),
                },
                "before" if valid_date(&value) => q.before.push(value),
                "after" if valid_date(&value) => q.after.push(value),
                _ => q.text.push(lower),
            }
        }
        q
    }

    /// True if the input should be treated as a search: starts with `/` or
    /// contains a known `key:` token.
    pub fn is_search(input: &str) -> bool {
        let input = input.trim_start();
        input.starts_with('/')
            || tokenize(input)
                .iter()
                .any(|t| is_key(t).is_some_and(|(_, v)| !v.is_empty()))
    }

    /// `screened` is true when the sender has been allowed (is a known
    /// contact); `is:screener` matches messages with `screened == false`.
    pub fn matches(&self, m: &Message, state: TriageState, screened: bool) -> bool {
        let name = m.from_name.to_lowercase();
        let email = m.from_email.to_lowercase();
        let subject = m.subject.to_lowercase();
        let body = m.body.to_lowercase();
        let date = m.received.get(..10).unwrap_or(&m.received);

        self.from
            .iter()
            .all(|f| name.contains(f) || email.contains(f))
            && self.subject.iter().all(|s| subject.contains(s))
            && self.is.iter().all(|i| match i {
                Is::State(s) => *s == state,
                Is::Screener => !screened,
            })
            && self.before.iter().all(|d| date < d.as_str())
            && self.after.iter().all(|d| date >= d.as_str())
            && self.text.iter().all(|t| {
                subject.contains(t) || body.contains(t) || name.contains(t) || email.contains(t)
            })
    }

    /// Human-readable summary of the parsed terms, for the palette hint row.
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        parts.extend(self.from.iter().map(|v| format!("from:{v}")));
        parts.extend(self.subject.iter().map(|v| format!("subject:{v}")));
        for i in &self.is {
            parts.push(match i {
                Is::State(s) => format!("is:{}", s.label().to_lowercase()),
                Is::Screener => "is:screener".into(),
            });
        }
        parts.extend(self.after.iter().map(|v| format!("after:{v}")));
        parts.extend(self.before.iter().map(|v| format!("before:{v}")));
        parts.extend(self.text.iter().map(|v| format!("\"{v}\"")));
        parts.join(" ")
    }
}
