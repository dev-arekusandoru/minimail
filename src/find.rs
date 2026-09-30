//! Find in a thread: the text the reader shows, searched with case / whole-word / regex
//! options, as ordered match ranges. No GPUI types; the reader tab's bar only holds a [`Find`].

use std::ops::Range;

use regex::{Regex, RegexBuilder};

use crate::model::{Message, MessageId};
use crate::reading;
use crate::threads::thread_order;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
}

/// Which displayed string of a message a match lies in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Segment {
    /// The subject line of the header.
    Subject,
    /// The body without its quoted history.
    Main,
    /// The folded quoted history.
    Quoted,
}

/// One searched string, tagged with the message it belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub msg: MessageId,
    pub segment: Segment,
    pub text: String,
}

/// A match: `range` is a byte range into the [`Source`] text of (`msg`, `segment`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    pub msg: MessageId,
    pub segment: Segment,
    pub range: Range<usize>,
}

/// The query is not a valid regular expression (or too large to compile).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidPattern(pub String);

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The compiled query; `None` for an empty one, which matches nothing.
fn compile(query: &str, options: Options) -> Result<Option<Regex>, InvalidPattern> {
    if query.is_empty() {
        return Ok(None);
    }
    let mut pattern = if options.regex { query.to_owned() } else { regex::escape(query) };
    if options.whole_word {
        // A literal only needs a boundary on a side that starts or ends with a word character
        // (`c++` ends in punctuation, where `\b` would demand a word character after it).
        let (start, end) = if options.regex {
            (true, true)
        } else {
            (query.chars().next().is_some_and(is_word), query.chars().next_back().is_some_and(is_word))
        };
        pattern = format!("{}(?:{pattern}){}", if start { r"\b" } else { "" }, if end { r"\b" } else { "" });
    }
    RegexBuilder::new(&pattern)
        .case_insensitive(!options.case_sensitive)
        .build()
        .map(Some)
        .map_err(|e| InvalidPattern(e.to_string()))
}

/// Every non-empty, non-overlapping match in `sources`, in source order then text order.
/// An empty query has no matches; an invalid regex is an error.
pub fn find(sources: &[Source], query: &str, options: Options) -> Result<Vec<Match>, InvalidPattern> {
    let Some(re) = compile(query, options)? else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for s in sources {
        for m in re.find_iter(&s.text).filter(|m| !m.is_empty()) {
            out.push(Match { msg: s.msg, segment: s.segment, range: m.range() });
        }
    }
    Ok(out)
}

/// The subject exactly as the reader header shows it.
pub fn display_subject(m: &Message) -> String {
    reading::display_subject(m)
}

/// What the reader of `thread` displays, top to bottom: the opened message's subject, then each
/// message's body (oldest first), the quoted history as its own string. Collapsed messages and
/// folded quotes are included; the strings are the ones `reader_text` / `split_quoted` produce.
pub fn thread_sources(messages: &[Message], thread: u32, opened: MessageId) -> Vec<Source> {
    let mut out = Vec::new();
    if let Some(m) = messages.iter().find(|m| m.id == opened) {
        out.push(Source { msg: m.id, segment: Segment::Subject, text: display_subject(m) });
    }
    for id in thread_order(messages, thread) {
        let Some(m) = messages.iter().find(|m| m.id == id) else { continue };
        let text = reading::reader_text(m);
        let split = reading::split_quoted(&text);
        out.push(Source { msg: id, segment: Segment::Main, text: split.main.to_owned() });
        if let Some(quoted) = split.quoted {
            out.push(Source { msg: id, segment: Segment::Quoted, text: quoted.to_owned() });
        }
    }
    out
}

/// Matches of `query` over [`thread_sources`].
pub fn find_in_thread(
    messages: &[Message],
    thread: u32,
    opened: MessageId,
    query: &str,
    options: Options,
) -> Result<Vec<Match>, InvalidPattern> {
    find(&thread_sources(messages, thread, opened), query, options)
}

/// Highlight ranges inside one string: every match of (`msg`, `segment`), flagged when it is
/// the `current`-th match overall.
pub fn highlights(matches: &[Match], current: usize, msg: MessageId, segment: Segment) -> Vec<(Range<usize>, bool)> {
    matches
        .iter()
        .enumerate()
        .filter(|(_, m)| m.msg == msg && m.segment == segment)
        .map(|(i, m)| (m.range.clone(), i == current))
        .collect()
}

/// One tab's find state: the query, its options and which match is current.
#[derive(Clone, Debug, Default)]
pub struct Find {
    pub query: String,
    pub options: Options,
    current: usize,
}

impl Find {
    /// Index of the current match among `total`; 0 when there are none.
    pub fn current(&self, total: usize) -> usize {
        self.current.min(total.saturating_sub(1))
    }

    /// New query text. The current match goes back to the first one when the text changed.
    pub fn set_query(&mut self, query: &str) {
        if self.query != query {
            self.query = query.to_owned();
            self.current = 0;
        }
    }

    /// Change the options; the current match goes back to the first one.
    pub fn set_options(&mut self, options: Options) {
        if self.options != options {
            self.options = options;
            self.current = 0;
        }
    }

    /// Move the current match by `delta` among `total`, wrapping at both ends.
    pub fn step(&mut self, total: usize, delta: isize) {
        self.current = if total == 0 {
            0
        } else {
            (self.current(total) as isize + delta).rem_euclid(total as isize) as usize
        };
    }
}
