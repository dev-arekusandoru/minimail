//! Structured search query: the single source of truth for list filtering.
//!
//! A [`Query`] is a set of per-field [`Group`]s plus free text. Different fields are always
//! ANDed; the values inside one field's group are combined with that group's [`Combinator`]
//! (default AND). Free-text terms are ANDed and match subject, body and sender. All values are
//! stored lowercase.
//!
//! # Syntax (case-insensitive, `parse` never fails)
//!
//! | field | value |
//! |---|---|
//! | `from:` `to:` `cc:` `bcc:` `subject:` `body:` | substring (`from` matches name or address) |
//! | `before:` `after:` `on:` | `yyyy-mm-dd`, or relative `<n>d`/`w`/`m`/`y` (`after:7d`, `before:3m`) |
//! | `is:` | `inbox snoozed archived filed deleted sent new` |
//! | `in:` | `inbox sent snoozed archived deleted`, or a folder name |
//! | `tag:` | `needs-reply awaiting follow-up reminder spam urgent` |
//! | `kind:` | `person receipt newsletter notification other` |
//! | `account:` | account id |
//!
//! `before:` is exclusive, `after:` inclusive; relative dates resolve to a UTC calendar day
//! against the `now` passed to [`Query::matches`]. Values containing spaces or commas are quoted
//! (`from:"ann lee"`). A leading `/` is ignored; a token with an unknown key or an invalid value
//! (`foo:bar`, `is:bogus`, `before:soon`) stays free text.
//!
//! # Combinators
//!
//! * `field:a,b` — one token, values ORed. A trailing comma (`field:a,`) is a single-value OR
//!   group, so the combinator survives removing values down to one.
//! * `field:a field:b` — repeated tokens, values ANDed.
//!
//! A field has exactly one group, so if any of its tokens uses commas the whole group is OR.
//! [`Display`](std::fmt::Display) emits this canonical form and `Query::parse(&q.to_string()) == q`.

mod dates;
mod matching;

use std::fmt;

use crate::judge::Kind;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Field {
    From,
    To,
    Cc,
    Bcc,
    Subject,
    Body,
    Before,
    After,
    On,
    Is,
    In,
    Tag,
    Kind,
    Account,
}

impl Field {
    pub const ALL: [Field; 14] = [
        Field::From,
        Field::To,
        Field::Cc,
        Field::Bcc,
        Field::Subject,
        Field::Body,
        Field::Before,
        Field::After,
        Field::On,
        Field::Is,
        Field::In,
        Field::Tag,
        Field::Kind,
        Field::Account,
    ];

    /// The operator as typed: `from`, `in`, ...
    pub fn key(self) -> &'static str {
        match self {
            Field::From => "from",
            Field::To => "to",
            Field::Cc => "cc",
            Field::Bcc => "bcc",
            Field::Subject => "subject",
            Field::Body => "body",
            Field::Before => "before",
            Field::After => "after",
            Field::On => "on",
            Field::Is => "is",
            Field::In => "in",
            Field::Tag => "tag",
            Field::Kind => "kind",
            Field::Account => "account",
        }
    }

    pub fn from_key(key: &str) -> Option<Field> {
        Field::ALL.into_iter().find(|f| f.key().eq_ignore_ascii_case(key))
    }

    /// Canonical lowercase form of `raw` for this field, or `None` if it is not a valid value.
    pub fn normalize(self, raw: &str) -> Option<String> {
        let v = raw.trim().to_lowercase();
        if v.is_empty() {
            return None;
        }
        let ok = match self {
            Field::Is => IS_VALUES.contains(&v.as_str()),
            Field::Tag => TAG_VALUES.contains(&v.as_str()),
            Field::Kind => Kind::ALL.iter().any(|k| k.label() == v),
            Field::Before | Field::After | Field::On => dates::is_valid(&v),
            _ => true,
        };
        if !ok {
            return None;
        }
        Some(match (self, v.as_str()) {
            (Field::In, "archive") => "archived".into(),
            (Field::In, "trash") => "deleted".into(),
            _ => v,
        })
    }
}

pub(crate) const IS_VALUES: [&str; 7] =
    ["inbox", "snoozed", "archived", "filed", "deleted", "sent", "new"];
pub(crate) const TAG_VALUES: [&str; 6] =
    ["needs-reply", "awaiting", "follow-up", "reminder", "spam", "urgent"];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Combinator {
    #[default]
    And,
    Or,
}

impl Combinator {
    pub fn toggled(self) -> Combinator {
        match self {
            Combinator::And => Combinator::Or,
            Combinator::Or => Combinator::And,
        }
    }
}

/// All values of one field. Never empty inside a [`Query`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub field: Field,
    pub values: Vec<String>,
    pub combinator: Combinator,
}

/// One removable pill: a single field value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pill {
    pub field: Field,
    pub value: String,
    /// How this pill joins the previous pill of the same field; `None` for the first.
    pub combinator: Option<Combinator>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    groups: Vec<Group>,
    text: Vec<String>,
}

/// Splits on whitespace outside quotes, keeping the quotes.
fn raw_tokens(input: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut start = None;
    let mut quoted = false;
    for (i, c) in input.char_indices() {
        if c == '"' {
            quoted = !quoted;
            start.get_or_insert(i);
        } else if c.is_whitespace() && !quoted {
            if let Some(s) = start.take() {
                tokens.push(&input[s..i]);
            }
        } else {
            start.get_or_insert(i);
        }
    }
    if let Some(s) = start {
        tokens.push(&input[s..]);
    }
    tokens
}

/// Splits on commas outside quotes.
fn split_commas(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut quoted = false;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ',' if !quoted => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&s[start..]);
    parts
}

fn unquote(s: &str) -> String {
    s.chars().filter(|c| *c != '"').collect()
}

/// `(field, operand)` when `token` starts with a known `key:` and a non-empty operand.
fn split_key(token: &str) -> Option<(Field, &str)> {
    let (key, rest) = token.split_once(':')?;
    let field = Field::from_key(key)?;
    (!rest.is_empty()).then_some((field, rest))
}

impl Query {
    pub fn parse(input: &str) -> Query {
        let input = input.trim_start();
        let input = input.strip_prefix('/').unwrap_or(input);
        let mut q = Query::default();
        for token in raw_tokens(input) {
            if let Some((field, rest)) = split_key(token) {
                let pieces = split_commas(rest);
                let values: Option<Vec<String>> = pieces
                    .iter()
                    .map(|p| unquote(p))
                    .filter(|p| !p.trim().is_empty())
                    .map(|p| field.normalize(&p))
                    .collect();
                if let Some(values) = values.filter(|v| !v.is_empty()) {
                    let or = pieces.len() > 1;
                    for v in values {
                        q.insert(field, v, or);
                    }
                    continue;
                }
            }
            let term = unquote(token).trim().to_lowercase();
            if !term.is_empty() {
                q.text.push(term);
            }
        }
        q
    }

    /// Whether `input` should be treated as a search rather than a palette command.
    pub fn is_search(input: &str) -> bool {
        let input = input.trim_start();
        input.starts_with('/') || raw_tokens(input).into_iter().any(|t| split_key(t).is_some())
    }

    fn insert(&mut self, field: Field, value: String, or: bool) -> bool {
        let group = match self.groups.iter().position(|g| g.field == field) {
            Some(i) => &mut self.groups[i],
            None => {
                self.groups.push(Group { field, values: Vec::new(), combinator: Combinator::And });
                self.groups.last_mut().expect("just pushed")
            }
        };
        if or {
            group.combinator = Combinator::Or;
        }
        if group.values.contains(&value) {
            return false;
        }
        group.values.push(value);
        true
    }

    pub fn is_empty(&self) -> bool {
        self.groups.is_empty() && self.text.is_empty()
    }

    pub fn groups(&self) -> &[Group] {
        &self.groups
    }

    /// Free-text terms (lowercase), ANDed over subject, body and sender.
    pub fn text(&self) -> &[String] {
        &self.text
    }

    pub fn values(&self, field: Field) -> &[String] {
        self.groups
            .iter()
            .find(|g| g.field == field)
            .map_or(&[], |g| &g.values)
    }

    /// The group's combinator; `None` if the field has no values.
    pub fn combinator(&self, field: Field) -> Option<Combinator> {
        self.groups.iter().find(|g| g.field == field).map(|g| g.combinator)
    }

    pub fn has(&self, field: Field, value: &str) -> bool {
        field.normalize(value).is_some_and(|v| self.values(field).contains(&v))
    }

    /// Adds `value` to `field`'s group. `false` if invalid or already present.
    pub fn add(&mut self, field: Field, value: &str) -> bool {
        field.normalize(value).is_some_and(|v| self.insert(field, v, false))
    }

    /// Removes `value`, dropping the group when it empties. `false` if it was absent.
    pub fn remove(&mut self, field: Field, value: &str) -> bool {
        let Some(v) = field.normalize(value) else {
            return false;
        };
        let Some(i) = self.groups.iter().position(|g| g.field == field) else {
            return false;
        };
        let Some(j) = self.groups[i].values.iter().position(|x| *x == v) else {
            return false;
        };
        self.groups[i].values.remove(j);
        if self.groups[i].values.is_empty() {
            self.groups.remove(i);
        }
        true
    }

    /// Adds the value if absent, removes it if present. Returns whether it is now present
    /// (`false` also for an invalid value, which changes nothing).
    pub fn toggle(&mut self, field: Field, value: &str) -> bool {
        if self.remove(field, value) {
            false
        } else {
            self.add(field, value)
        }
    }

    /// Sets how `field`'s values combine. `false` if the field has no values.
    pub fn set_combinator(&mut self, field: Field, combinator: Combinator) -> bool {
        match self.groups.iter_mut().find(|g| g.field == field) {
            Some(g) => {
                g.combinator = combinator;
                true
            }
            None => false,
        }
    }

    /// Removes every value of `field`. `false` if there were none.
    pub fn clear_field(&mut self, field: Field) -> bool {
        let before = self.groups.len();
        self.groups.retain(|g| g.field != field);
        self.groups.len() != before
    }

    /// Replaces `old` with `new` in `field`'s group, keeping its position. Replacing with a
    /// value the group already holds just drops `old`. `false` if `old` is absent or `new`
    /// is not a valid value (nothing changes).
    pub fn replace(&mut self, field: Field, old: &str, new: &str) -> bool {
        let (Some(old), Some(new)) = (field.normalize(old), field.normalize(new)) else {
            return false;
        };
        let Some(group) = self.groups.iter_mut().find(|g| g.field == field) else {
            return false;
        };
        let Some(i) = group.values.iter().position(|x| *x == old) else {
            return false;
        };
        if old != new && group.values.contains(&new) {
            group.values.remove(i);
        } else {
            group.values[i] = new;
        }
        true
    }

    fn normalize_text(raw: &str) -> Option<String> {
        Some(unquote(raw).trim().to_lowercase()).filter(|t| !t.is_empty())
    }

    /// Adds a free-text term. `false` if blank or already present.
    pub fn add_text(&mut self, term: &str) -> bool {
        let Some(term) = Self::normalize_text(term) else {
            return false;
        };
        if self.text.contains(&term) {
            return false;
        }
        self.text.push(term);
        true
    }

    /// Removes a free-text term. `false` if it was absent.
    pub fn remove_text(&mut self, term: &str) -> bool {
        let Some(term) = Self::normalize_text(term) else {
            return false;
        };
        let before = self.text.len();
        self.text.retain(|t| *t != term);
        self.text.len() != before
    }

    /// Replaces a free-text term, keeping its position. `false` if `old` is absent or `new`
    /// is blank.
    pub fn replace_text(&mut self, old: &str, new: &str) -> bool {
        let (Some(old), Some(new)) = (Self::normalize_text(old), Self::normalize_text(new)) else {
            return false;
        };
        let Some(i) = self.text.iter().position(|t| *t == old) else {
            return false;
        };
        if old != new && self.text.contains(&new) {
            self.text.remove(i);
        } else {
            self.text[i] = new;
        }
        true
    }

    /// Drops every free-text term.
    pub fn clear_text(&mut self) -> bool {
        let had = !self.text.is_empty();
        self.text.clear();
        had
    }

    /// Lays `other` over this query: each field `other` constrains replaces this query's
    /// group for that field, and `other`'s free-text terms are appended.
    pub fn overlay(&mut self, other: &Query) {
        for g in &other.groups {
            self.clear_field(g.field);
            for v in &g.values {
                self.insert(g.field, v.clone(), g.combinator == Combinator::Or);
            }
        }
        for t in &other.text {
            if !self.text.contains(t) {
                self.text.push(t.clone());
            }
        }
    }

    /// Every field value in display order, grouped by field.
    pub fn pills(&self) -> Vec<Pill> {
        self.groups
            .iter()
            .flat_map(|g| {
                g.values.iter().enumerate().map(|(i, v)| Pill {
                    field: g.field,
                    value: v.clone(),
                    combinator: (i > 0).then_some(g.combinator),
                })
            })
            .collect()
    }

    /// Canonical query string; `Query::parse` of it yields an equal query.
    pub fn describe(&self) -> String {
        self.to_string()
    }
}

fn write_value(f: &mut fmt::Formatter<'_>, v: &str) -> fmt::Result {
    if v.chars().any(|c| c.is_whitespace() || c == ',' || c == '"') {
        write!(f, "\"{}\"", v.replace('"', ""))
    } else {
        f.write_str(v)
    }
}

impl fmt::Display for Query {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        let mut sep = |f: &mut fmt::Formatter<'_>| {
            let s = if first { Ok(()) } else { f.write_str(" ") };
            first = false;
            s
        };
        for g in &self.groups {
            match g.combinator {
                Combinator::And => {
                    for v in &g.values {
                        sep(f)?;
                        write!(f, "{}:", g.field.key())?;
                        write_value(f, v)?;
                    }
                }
                Combinator::Or => {
                    sep(f)?;
                    write!(f, "{}:", g.field.key())?;
                    for (i, v) in g.values.iter().enumerate() {
                        if i > 0 {
                            f.write_str(",")?;
                        }
                        write_value(f, v)?;
                    }
                    if g.values.len() == 1 {
                        f.write_str(",")?;
                    }
                }
            }
        }
        for t in &self.text {
            sep(f)?;
            write!(f, "\"{}\"", t.replace('"', ""))?;
        }
        Ok(())
    }
}
