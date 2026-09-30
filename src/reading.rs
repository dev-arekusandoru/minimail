//! Pure helpers for the reader: recipients, quoted-reply splitting, HTML
//! sanitising and text extraction, sizes, avatars initials, thread position and
//! the per-thread view state.
//!
//! No GPUI types live here, so all of it is headlessly testable.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use crate::model::{Message, MessageId};
use crate::threads::thread_order;

// ---------------------------------------------------------------- recipients

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recipient {
    pub name: Option<String>,
    pub email: String,
}

impl Recipient {
    fn display(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.email)
    }
}

/// Parse a comma-separated recipient field ("Name <email>" or bare email).
/// Tolerates quoted names containing commas, empty items and stray quotes.
pub fn parse_recipients(field: &str) -> Vec<Recipient> {
    let mut parts: Vec<&str> = Vec::new();
    let (mut in_quote, mut in_angle, mut start) = (false, false, 0);
    for (i, c) in field.char_indices() {
        match c {
            '"' if !in_angle => in_quote = !in_quote,
            '<' if !in_quote => in_angle = true,
            '>' if !in_quote => in_angle = false,
            ',' if !in_quote && !in_angle => {
                parts.push(&field[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&field[start..]);

    parts.into_iter().filter_map(parse_one_recipient).collect()
}

fn parse_one_recipient(part: &str) -> Option<Recipient> {
    let part = part.trim();
    if part.is_empty() {
        return None;
    }
    let quotes = |c: char| c == '"' || c == '\'';
    let (name, email) = match part.rfind('<') {
        Some(lt) => {
            let rest = &part[lt + 1..];
            let email = rest.find('>').map_or(rest, |gt| &rest[..gt]);
            (part[..lt].trim().trim_matches(quotes).trim(), email)
        }
        None => ("", part),
    };
    let email = email.trim().trim_matches(quotes).trim();
    if email.is_empty() {
        return None;
    }
    Some(Recipient {
        name: (!name.is_empty()).then(|| name.to_owned()),
        email: email.to_owned(),
    })
}

/// "to me", "to me, +3 others", "to Alice Chen, +1 other". Counts distinct
/// addresses across to, cc and bcc; `me` is matched case-insensitively and is
/// always named first. Empty when the message has no recipients.
pub fn recipient_summary(msg: &Message, me: &str) -> String {
    let mut seen = HashSet::new();
    let all: Vec<Recipient> = [&msg.to, &msg.cc, &msg.bcc]
        .into_iter()
        .flat_map(|field| parse_recipients(field))
        .filter(|r| seen.insert(r.email.to_ascii_lowercase()))
        .collect();
    let Some(first) = all.first() else {
        return String::new();
    };
    let me = me.trim();
    let is_me = |r: &Recipient| !me.is_empty() && r.email.eq_ignore_ascii_case(me);
    let head = if all.iter().any(is_me) { "me" } else { first.display() };
    match all.len() - 1 {
        0 => format!("to {head}"),
        1 => format!("to {head}, +1 other"),
        n => format!("to {head}, +{n} others"),
    }
}

// -------------------------------------------------------------------- quoted

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Quoted<'a> {
    pub main: &'a str,
    pub quoted: Option<&'a str>,
}

/// Split a reply body into the new text and the quoted history. The quote
/// starts at the first "On … wrote:" line, an "-----Original Message-----"
/// line, or the first line of a trailing block of `>` lines, whichever comes
/// first. `main` never ends in blank lines.
pub fn split_quoted(body: &str) -> Quoted<'_> {
    // (byte offset, trimmed line) for every line.
    let mut lines: Vec<(usize, &str)> = Vec::new();
    let mut offset = 0;
    for line in body.split('\n') {
        lines.push((offset, line.trim()));
        offset += line.len() + 1;
    }

    let marker = lines.iter().enumerate().position(|(i, (_, line))| {
        let lower = line.to_ascii_lowercase();
        lower.starts_with("-----original message")
            || (lower.starts_with("on ")
                && (lower.ends_with("wrote:")
                    // Clients wrap long attributions onto a second line.
                    || lines.get(i + 1).is_some_and(|(_, next)| {
                        let next = next.to_ascii_lowercase();
                        next.ends_with("wrote:") && next.split_whitespace().count() <= 2
                    })))
    });

    let mut end = lines.len();
    while end > 0 && lines[end - 1].1.is_empty() {
        end -= 1;
    }
    let mut block = end;
    while block > 0 && lines[block - 1].1.starts_with('>') {
        block -= 1;
    }
    let block = (block < end).then_some(block);

    let first = match (marker, block) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    match first {
        Some(i) => Quoted {
            main: body[..lines[i].0].trim_end(),
            quoted: Some(&body[lines[i].0..]),
        },
        None => Quoted { main: body.trim_end(), quoted: None },
    }
}

// ---------------------------------------------------------------------- HTML

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SafeHtml {
    pub html: String,
    pub blocked_images: usize,
}

const RAW_TEXT: [&str; 4] = ["script", "style", "textarea", "title"];

enum Kind {
    Text,
    Start { name: String, attrs: Vec<(String, String)> },
    End { name: String },
    Comment,
    /// Content of script/style/textarea/title, never parsed as markup.
    RawText { name: &'static str },
}

struct Tok<'a> {
    raw: &'a str,
    kind: Kind,
}

/// Forgiving HTML tokenizer following the spec's tag/attribute rules, so that
/// what we treat as an `<img>` agrees with what a browser would.
struct Scanner<'a> {
    s: &'a str,
    pos: usize,
    raw_text: Option<&'static str>,
}

fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0c)
}

fn find_ci(hay: &str, from: usize, needle: &str) -> Option<usize> {
    let (h, n) = (hay.as_bytes(), needle.as_bytes());
    (from..h.len().saturating_sub(n.len() - 1)).find(|&i| h[i..i + n.len()].eq_ignore_ascii_case(n))
}

impl<'a> Scanner<'a> {
    fn new(s: &'a str) -> Self {
        Scanner { s, pos: 0, raw_text: None }
    }

    /// Parse a tag whose `<` sits at `self.pos`. Returns the token; an
    /// unterminated tag swallows the rest of the input.
    fn tag(&mut self, end_tag: bool) -> Tok<'a> {
        let s = self.s;
        let b = s.as_bytes();
        let n = b.len();
        let begin = self.pos;
        let mut i = begin + if end_tag { 2 } else { 1 };
        let name_start = i;
        while i < n && !is_ws(b[i]) && b[i] != b'/' && b[i] != b'>' {
            i += 1;
        }
        let name = s[name_start..i].to_ascii_lowercase();
        let mut attrs: Vec<(String, String)> = Vec::new();
        let end = 'tag: loop {
            while i < n && (is_ws(b[i]) || b[i] == b'/') {
                i += 1;
            }
            if i >= n {
                break n;
            }
            if b[i] == b'>' {
                break i + 1;
            }
            let attr_start = i;
            i += 1; // a leading '=' belongs to the name
            while i < n && !is_ws(b[i]) && !matches!(b[i], b'/' | b'>' | b'=') {
                i += 1;
            }
            let attr = s[attr_start..i].to_ascii_lowercase();
            while i < n && is_ws(b[i]) {
                i += 1;
            }
            let mut value = "";
            if i < n && b[i] == b'=' {
                i += 1;
                while i < n && is_ws(b[i]) {
                    i += 1;
                }
                if i >= n {
                    break n;
                }
                match b[i] {
                    q @ (b'"' | b'\'') => match s[i + 1..].find(q as char) {
                        Some(len) => {
                            value = &s[i + 1..i + 1 + len];
                            i += len + 2;
                        }
                        None => break 'tag n,
                    },
                    b'>' => {}
                    _ => {
                        let v = i;
                        while i < n && !is_ws(b[i]) && b[i] != b'>' {
                            i += 1;
                        }
                        value = &s[v..i];
                    }
                }
            }
            if !attrs.iter().any(|(k, _)| *k == attr) {
                attrs.push((attr, value.to_owned()));
            }
        };
        self.pos = end;
        let raw = &s[begin..end];
        if end_tag {
            return Tok { raw, kind: Kind::End { name } };
        }
        if let Some(&raw_name) = RAW_TEXT.iter().find(|r| **r == name) {
            self.raw_text = Some(raw_name);
        }
        Tok { raw, kind: Kind::Start { name, attrs } }
    }
}

impl<'a> Iterator for Scanner<'a> {
    type Item = Tok<'a>;

    fn next(&mut self) -> Option<Tok<'a>> {
        let s = self.s;
        if self.pos >= s.len() {
            return None;
        }
        if let Some(name) = self.raw_text.take() {
            let close = format!("</{name}");
            let end = find_ci(s, self.pos, &close).unwrap_or(s.len());
            if end > self.pos {
                let raw = &s[self.pos..end];
                self.pos = end;
                return Some(Tok { raw, kind: Kind::RawText { name } });
            }
        }
        let b = s.as_bytes();
        let at = self.pos;
        if b[at] == b'<' {
            let next = b.get(at + 1).copied().unwrap_or(0);
            if next.is_ascii_alphabetic() {
                return Some(self.tag(false));
            }
            if next == b'/' && b.get(at + 2).is_some_and(u8::is_ascii_alphabetic) {
                return Some(self.tag(true));
            }
            if next == b'!' || next == b'?' || next == b'/' {
                let end = if s[at..].starts_with("<!--") {
                    find_ci(s, at + 2, "-->").map_or(s.len(), |e| e + 3)
                } else {
                    s[at..].find('>').map_or(s.len(), |e| at + e + 1)
                };
                self.pos = end;
                return Some(Tok { raw: &s[at..end], kind: Kind::Comment });
            }
        }
        // Text up to the next '<' (a lone '<' is literal text).
        let first = s[at..].chars().next().map_or(1, char::len_utf8);
        let end = s[at + first..].find('<').map_or(s.len(), |e| at + first + e);
        self.pos = end;
        Some(Tok { raw: &s[at..end], kind: Kind::Text })
    }
}

fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
    out
}

fn attr<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
}

/// Replace every `<img>` whose `src` is not a `data:` URL with its (escaped)
/// alt text, or nothing, and count them. Everything else is copied verbatim.
pub fn safe_html(html: &str) -> SafeHtml {
    let mut out = String::with_capacity(html.len());
    let mut blocked = 0;
    for tok in Scanner::new(html) {
        match &tok.kind {
            Kind::Start { name, attrs } if name == "img" || name == "image" => {
                let src = attr(attrs, "src").map(|v| decode_entities(v));
                let inline = src
                    .as_deref()
                    .map(|v| v.trim_start_matches(|c: char| c.is_whitespace() || c.is_control()))
                    .is_some_and(|v| v.get(..5).is_some_and(|p| p.eq_ignore_ascii_case("data:")));
                if inline {
                    out.push_str(tok.raw);
                    continue;
                }
                blocked += 1;
                let alt = decode_entities(attr(attrs, "alt").unwrap_or(""));
                out.push_str(&escape_text(alt.trim()));
            }
            _ => out.push_str(tok.raw),
        }
    }
    SafeHtml { html: out, blocked_images: blocked }
}

fn named_entity(name: &str) -> Option<&'static str> {
    Some(match name {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        "nbsp" => " ",
        "mdash" => "—",
        "ndash" => "–",
        "hellip" => "…",
        "copy" => "©",
        "reg" => "®",
        "trade" => "™",
        "lsquo" => "‘",
        "rsquo" => "’",
        "ldquo" => "“",
        "rdquo" => "”",
        "bull" => "•",
        "middot" => "·",
        "euro" => "€",
        "pound" => "£",
        "laquo" => "«",
        "raquo" => "»",
        "times" => "×",
        "rarr" => "→",
        "larr" => "←",
        _ => return None,
    })
}

/// Decode the common named entities and numeric references; anything else is
/// left untouched.
fn decode_entities(s: &str) -> Cow<'_, str> {
    if !s.contains('&') {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let decoded = rest[1..].find(';').filter(|&len| len <= 10).and_then(|len| {
            let body = &rest[1..1 + len];
            let text = match body.strip_prefix('#') {
                Some(num) => {
                    let code = match num.strip_prefix(['x', 'X']) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                        None => num.parse().ok()?,
                    };
                    let c = if code == 0 { '\u{fffd}' } else { char::from_u32(code)? };
                    c.to_string()
                }
                None => named_entity(body)?.to_owned(),
            };
            Some((text, len + 2))
        });
        match decoded {
            Some((text, used)) => {
                out.push_str(&text);
                rest = &rest[used..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// Accumulates readable text, tracking pending breaks and whitespace.
struct TextOut {
    out: String,
    /// Newlines wanted before the next text (0, 1 or 2).
    want: usize,
    space: bool,
    line_start: bool,
}

impl TextOut {
    fn new() -> Self {
        TextOut { out: String::new(), want: 0, space: false, line_start: true }
    }

    fn request(&mut self, newlines: usize) {
        self.want = self.want.max(newlines);
    }

    fn newline(&mut self) {
        self.out.push('\n');
        self.want = 0;
        self.space = false;
        self.line_start = true;
    }

    fn flush(&mut self) {
        if self.out.is_empty() {
            self.want = 0;
            self.space = false;
            return;
        }
        let trailing = self.out.bytes().rev().take_while(|&b| b == b'\n').count();
        for _ in trailing..self.want {
            self.out.push('\n');
        }
        if self.want > 0 {
            self.space = false;
            self.line_start = true;
        }
        self.want = 0;
    }

    fn text(&mut self, text: &str, pre: bool) {
        for c in text.chars() {
            if pre {
                self.flush();
                match c {
                    '\n' => self.newline(),
                    '\r' => {}
                    '\u{a0}' => self.push(' '),
                    c => self.push(c),
                }
            } else if c.is_whitespace() {
                self.space = true;
            } else {
                self.flush();
                if self.space && !self.line_start {
                    self.out.push(' ');
                }
                self.space = false;
                self.push(c);
            }
        }
    }

    fn push(&mut self, c: char) {
        self.out.push(c);
        self.line_start = false;
    }

    fn finish(self) -> String {
        let mut lines: Vec<&str> = Vec::new();
        let mut blank = false;
        for line in self.out.split('\n') {
            let line = line.trim_end();
            if line.is_empty() {
                blank = true;
                continue;
            }
            if blank && !lines.is_empty() {
                lines.push("");
            }
            blank = false;
            lines.push(line);
        }
        lines.join("\n")
    }
}

/// Readable plain text from HTML: head/script/style content is dropped, block
/// elements become line breaks, list items become "- ", common entities are
/// decoded and runs of blank lines collapse to one.
pub fn html_to_text(html: &str) -> String {
    const PARAGRAPH: [&str; 13] = [
        "p", "h1", "h2", "h3", "h4", "h5", "h6", "blockquote", "table", "ul", "ol", "pre", "dl",
    ];
    const LINE: [&str; 18] = [
        "div", "tr", "hr", "section", "article", "header", "footer", "nav", "aside", "main",
        "form", "fieldset", "address", "figure", "dt", "dd", "thead", "tbody",
    ];
    let mut t = TextOut::new();
    let (mut in_head, mut template_depth, mut pre_depth) = (false, 0usize, 0usize);
    for tok in Scanner::new(html) {
        match tok.kind {
            Kind::Comment => {}
            Kind::RawText { name } => {
                if name == "textarea" && !in_head && template_depth == 0 {
                    t.text(&decode_entities(tok.raw), true);
                }
            }
            Kind::Text => {
                if !in_head && template_depth == 0 {
                    t.text(&decode_entities(tok.raw), pre_depth > 0);
                }
            }
            Kind::Start { name, .. } => {
                if in_head
                    && !matches!(
                        name.as_str(),
                        "head" | "html" | "meta" | "link" | "title" | "style" | "script" | "base"
                            | "noscript" | "template"
                    )
                {
                    // Anything else implies the head ended, even without `</head>`.
                    in_head = false;
                }
                match name.as_str() {
                    "head" => in_head = true,
                    "template" => template_depth += 1,
                    _ => {}
                }
                if in_head || template_depth > 0 {
                    continue;
                }
                let name = name.as_str();
                if name == "br" {
                    t.flush();
                    t.newline();
                } else if name == "li" {
                    t.request(1);
                    t.flush();
                    t.out.push_str("- ");
                    t.line_start = true;
                    t.space = false;
                } else if name == "td" || name == "th" {
                    if !t.line_start && t.want == 0 {
                        t.space = true;
                        t.out.push(' ');
                    }
                } else if PARAGRAPH.contains(&name) {
                    if name == "pre" {
                        pre_depth += 1;
                    }
                    t.request(2);
                } else if LINE.contains(&name) {
                    t.request(1);
                }
            }
            Kind::End { name } => {
                match name.as_str() {
                    "head" => in_head = false,
                    "template" => template_depth = template_depth.saturating_sub(1),
                    _ => {}
                }
                if in_head || template_depth > 0 {
                    continue;
                }
                let name = name.as_str();
                if name == "li" {
                    t.request(1);
                } else if PARAGRAPH.contains(&name) {
                    if name == "pre" {
                        pre_depth = pre_depth.saturating_sub(1);
                    }
                    t.request(2);
                } else if LINE.contains(&name) {
                    t.request(1);
                }
            }
        }
    }
    t.finish()
}

/// The text to show for a message: its text part when it has any, else the
/// HTML converted to text, else nothing.
pub fn reader_text(msg: &Message) -> Cow<'_, str> {
    if !msg.body.trim().is_empty() {
        return Cow::Borrowed(&msg.body);
    }
    match &msg.html {
        Some(html) => Cow::Owned(html_to_text(html)),
        None => Cow::Borrowed(""),
    }
}

// ------------------------------------------------------------------ labels

/// "812 B", "14 KB", "1.2 MB": one decimal below 10, none from 10 up.
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    let round = |v: f64| if v < 10.0 { (v * 10.0).round() / 10.0 } else { v.round() };
    while unit < UNITS.len() - 1 && round(value) >= 1024.0 {
        value /= 1024.0;
        unit += 1;
    }
    let shown = round(value);
    if shown.fract() == 0.0 {
        format!("{shown:.0} {}", UNITS[unit])
    } else {
        format!("{shown:.1} {}", UNITS[unit])
    }
}

/// Up to two uppercase letters: first and last word of the name, falling back
/// to the words of the email's local part ("dana.whitfield@x" → "DW").
pub fn initials(name: &str, email: &str) -> String {
    fn words(s: &str, separators: &[char]) -> Vec<char> {
        s.split(|c: char| c.is_whitespace() || separators.contains(&c))
            .filter_map(|w| w.trim_start_matches(|c: char| !c.is_alphanumeric()).chars().next())
            .collect()
    }
    let name = name.trim();
    let mut letters = if name.is_empty() || name.contains('@') {
        Vec::new()
    } else {
        words(name, &[])
    };
    if letters.is_empty() {
        let local = email.trim().split('@').next().unwrap_or("");
        letters = words(local, &['.', '_', '-', '+']);
    }
    let picked: Vec<char> = match letters.as_slice() {
        [] => return "?".to_owned(),
        [only] => vec![*only],
        [first, .., last] => vec![*first, *last],
    };
    picked.iter().flat_map(|c| c.to_uppercase()).take(2).collect()
}

// ------------------------------------------------------------------ threads

/// 1-based chronological position of `opened` in its thread (received, then
/// id) and the thread's message count. An `opened` missing from `messages` is
/// counted as if it were part of the thread.
pub fn thread_position(messages: &[Message], opened: &Message) -> (usize, usize) {
    let order = thread_order(messages, opened.thread_id);
    if let Some(at) = order.iter().position(|&id| id == opened.id) {
        return (at + 1, order.len());
    }
    let before = messages
        .iter()
        .filter(|m| m.thread_id == opened.thread_id)
        .filter(|m| (&m.received, m.id) < (&opened.received, opened.id))
        .count();
    (before + 1, order.len() + 1)
}

/// The other messages of `opened`'s thread, newest first.
pub fn thread_others(messages: &[Message], opened: &Message) -> Vec<MessageId> {
    let mut order = thread_order(messages, opened.thread_id);
    order.retain(|&id| id != opened.id);
    order.reverse();
    order
}

// -------------------------------------------------------------- view state

/// Reader toggles of every thread that has one. Each thread keeps its own state, so a tab
/// switch never loses it; `forget` drops a thread's state when its tab goes. None of it is an
/// undo step.
#[derive(Default, Clone, Debug)]
pub struct ReaderView {
    threads: HashMap<u32, ThreadView>,
}

#[derive(Default, Clone, Debug)]
struct ThreadView {
    expanded: HashSet<MessageId>,
    recipients: HashSet<MessageId>,
    quoted: HashSet<MessageId>,
    plain: HashSet<MessageId>,
}

fn flip(set: &mut HashSet<MessageId>, id: MessageId) {
    if !set.remove(&id) {
        set.insert(id);
    }
}

impl ReaderView {
    fn of(&mut self, thread: u32) -> &mut ThreadView {
        self.threads.entry(thread).or_default()
    }

    /// Drop everything stored for `thread`.
    pub fn forget(&mut self, thread: u32) {
        self.threads.remove(&thread);
    }

    pub fn is_expanded(&self, thread: u32, id: MessageId) -> bool {
        self.threads.get(&thread).is_some_and(|v| v.expanded.contains(&id))
    }

    pub fn toggle_expanded(&mut self, thread: u32, id: MessageId) {
        flip(&mut self.of(thread).expanded, id);
    }

    /// Expand `id` (no-op when it already is); never collapses anything.
    pub fn expand(&mut self, thread: u32, id: MessageId) {
        self.of(thread).expanded.insert(id);
    }

    /// Expand every id, unless all are already expanded: then collapse them.
    pub fn toggle_all(&mut self, thread: u32, ids: &[MessageId]) {
        if ids.is_empty() {
            return;
        }
        let v = self.of(thread);
        if ids.iter().all(|id| v.expanded.contains(id)) {
            for id in ids {
                v.expanded.remove(id);
            }
        } else {
            v.expanded.extend(ids.iter().copied());
        }
    }

    pub fn recipients_open(&self, thread: u32, id: MessageId) -> bool {
        self.threads.get(&thread).is_some_and(|v| v.recipients.contains(&id))
    }

    pub fn toggle_recipients(&mut self, thread: u32, id: MessageId) {
        flip(&mut self.of(thread).recipients, id);
    }

    pub fn quoted_open(&self, thread: u32, id: MessageId) -> bool {
        self.threads.get(&thread).is_some_and(|v| v.quoted.contains(&id))
    }

    pub fn toggle_quoted(&mut self, thread: u32, id: MessageId) {
        flip(&mut self.of(thread).quoted, id);
    }

    /// Reader mode (plain text instead of HTML) is on for this message.
    pub fn plain(&self, thread: u32, id: MessageId) -> bool {
        self.threads.get(&thread).is_some_and(|v| v.plain.contains(&id))
    }

    pub fn toggle_plain(&mut self, thread: u32, id: MessageId) {
        flip(&mut self.of(thread).plain, id);
    }
}
