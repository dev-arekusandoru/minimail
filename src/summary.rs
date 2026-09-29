//! Opt-in thread summaries. [`StubSummarizer`] is deterministic and offline.
//!
//! # Mapping to TypeSafe Jev (future real provider)
//! A real summarizer is not a Jev classification call; it would sit beside the
//! [`crate::judge::Judge`] provider. The mapping that does exist: `action_items`
//! is a per-line `noul` ("is this line a request for the recipient to act?")
//! over the thread's lines, and the summary text comes from the provider's
//! generation endpoint. State sent is the same trimmed
//! [`crate::judge::message_state`] JSON, one entry per message.

use std::fmt;

use crate::model::Message;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ThreadSummary {
    pub summary: String,
    pub action_items: Vec<String>,
    pub dates: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SummaryError {
    EmptyThread,
    Unavailable(String),
}

impl fmt::Display for SummaryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SummaryError::EmptyThread => write!(f, "nothing to summarize"),
            SummaryError::Unavailable(m) => write!(f, "summarizer unavailable: {m}"),
        }
    }
}

impl std::error::Error for SummaryError {}

pub trait Summarizer {
    /// `thread` is in display order (oldest first or newest first; both work).
    fn summarize(&self, thread: &[&Message]) -> Result<ThreadSummary, SummaryError>;
}

/// Deterministic heuristic summarizer: the first sentence of each message,
/// lines that ask something or start with an imperative, and date-looking tokens.
#[derive(Clone, Copy, Debug, Default)]
pub struct StubSummarizer;

const MAX_SUMMARY_MESSAGES: usize = 3;
const MAX_ACTION_ITEMS: usize = 5;
const MAX_DATES: usize = 6;

const IMPERATIVES: [&str; 12] = [
    "please", "send", "reply", "review", "confirm", "check", "let me know", "add", "approve",
    "pay", "sign", "book",
];

const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];
const WEEKDAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

fn first_sentence(body: &str) -> Option<String> {
    let para = body.split("\n\n").map(str::trim).find(|p| !p.is_empty())?;
    let flat = para.split_whitespace().collect::<Vec<_>>().join(" ");
    let end = flat
        .char_indices()
        .find(|(i, c)| {
            matches!(c, '.' | '?' | '!')
                && flat[i + c.len_utf8()..]
                    .chars()
                    .next()
                    .is_none_or(char::is_whitespace)
        })
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(flat.len());
    Some(flat[..end].to_string())
}

fn is_action_line(line: &str) -> bool {
    let l = line.to_lowercase();
    l.contains('?') || IMPERATIVES.iter().any(|w| l.starts_with(w))
}

fn word_clean(w: &str) -> &str {
    w.trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != ':')
}

fn is_iso_date(w: &str) -> bool {
    let b = w.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

fn is_month(w: &str) -> bool {
    let l = w.to_lowercase();
    l.len() >= 3 && MONTHS.iter().any(|m| l.starts_with(m)) && l.chars().all(char::is_alphabetic)
}

fn is_weekday(w: &str) -> bool {
    let l = w.to_lowercase();
    l.len() >= 3 && WEEKDAYS.iter().any(|d| l.starts_with(d)) && l.chars().all(char::is_alphabetic)
}

fn is_day_number(w: &str) -> bool {
    matches!(w.parse::<u8>(), Ok(1..=31))
}

fn is_time(w: &str) -> bool {
    match w.split_once(':') {
        Some((h, m)) => {
            h.len() <= 2
                && m.len() == 2
                && h.parse::<u8>().is_ok_and(|h| h < 24)
                && m.parse::<u8>().is_ok_and(|m| m < 60)
        }
        None => false,
    }
}

/// Date-looking tokens in `text`: ISO dates, "8 Oct", "Oct 8", "Thu 1 Oct",
/// bare weekdays, each optionally followed by a time.
fn find_dates(text: &str) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().map(word_clean).collect();
    let mut found = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let w = words[i];
        let mut len = 0;
        if is_iso_date(w) {
            len = 1;
        } else if (is_day_number(w) && words.get(i + 1).is_some_and(|n| is_month(n)))
            || (is_month(w) && words.get(i + 1).is_some_and(|n| is_day_number(n)))
        {
            len = 2;
        } else if is_weekday(w) {
            len = 1;
            if words.get(i + 1).is_some_and(|n| is_day_number(n))
                && words.get(i + 2).is_some_and(|n| is_month(n))
            {
                len = 3;
            }
        }
        if len > 0 {
            if words.get(i + len).is_some_and(|t| is_time(t)) {
                len += 1;
            }
            let d = words[i..i + len].join(" ");
            if !found.contains(&d) {
                found.push(d);
            }
            i += len;
        } else {
            i += 1;
        }
    }
    found
}

impl Summarizer for StubSummarizer {
    fn summarize(&self, thread: &[&Message]) -> Result<ThreadSummary, SummaryError> {
        if thread.is_empty() {
            return Err(SummaryError::EmptyThread);
        }
        let mut sentences = Vec::new();
        let mut action_items: Vec<String> = Vec::new();
        let mut dates: Vec<String> = Vec::new();
        for m in thread {
            if sentences.len() < MAX_SUMMARY_MESSAGES
                && let Some(s) = first_sentence(&m.body)
            {
                sentences.push(s);
            }
            for line in m.body.lines().map(str::trim).filter(|l| !l.is_empty()) {
                if is_action_line(line)
                    && action_items.len() < MAX_ACTION_ITEMS
                    && !action_items.iter().any(|a| a == line)
                {
                    action_items.push(line.to_string());
                }
            }
            for d in find_dates(&format!("{} {}", m.subject, m.body)) {
                if dates.len() < MAX_DATES && !dates.contains(&d) {
                    dates.push(d);
                }
            }
        }
        let summary = if sentences.is_empty() {
            thread[0].subject.clone()
        } else {
            sentences.join(" ")
        };
        Ok(ThreadSummary {
            summary,
            action_items,
            dates,
        })
    }
}
