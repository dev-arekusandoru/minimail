//! What the list header's filter pills and picker offer, without any UI: the number-key quick
//! filters, the fields and values the `+ Filter` picker lists, and how a pill's value reads.
//!
//! The filters themselves are plain [`Query`] edits; this module only knows which edits to
//! offer. Matching typed text against the offers reuses [`crate::fuzzy`].

use crate::fuzzy;
use crate::judge::Kind;
use crate::known_senders::KNOWN_SENDERS_ENABLED;
use crate::model::Mailbox;
use crate::search::{Field, IS_VALUES, TAG_VALUES};

/// How the picker asks for a field's value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueInput {
    /// A fuzzy list of the values that exist.
    Pick,
    /// Free text: names, addresses, words.
    Text,
    /// A calendar, or a typed date (`2026-09-01`, `7d`).
    Date,
}

/// One row of the picker's first step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldChoice {
    pub field: Field,
    pub label: &'static str,
    pub input: ValueInput,
}

const fn field(field: Field, label: &'static str, input: ValueInput) -> FieldChoice {
    FieldChoice { field, label, input }
}

/// The picker's first step, in display order.
pub const FIELD_CHOICES: [FieldChoice; 14] = [
    field(Field::Tag, "Tag", ValueInput::Pick),
    field(Field::From, "From", ValueInput::Text),
    field(Field::To, "To", ValueInput::Text),
    field(Field::Cc, "Cc", ValueInput::Text),
    field(Field::Bcc, "Bcc", ValueInput::Text),
    field(Field::Subject, "Subject", ValueInput::Text),
    field(Field::Body, "Body", ValueInput::Text),
    field(Field::Before, "Date before", ValueInput::Date),
    field(Field::After, "Date after", ValueInput::Date),
    field(Field::On, "Date on", ValueInput::Date),
    field(Field::Is, "Is", ValueInput::Pick),
    field(Field::Kind, "Kind", ValueInput::Pick),
    field(Field::Account, "Account", ValueInput::Pick),
    field(Field::In, "In", ValueInput::Pick),
];

/// How `field`'s value is entered. Every field has a row in [`FIELD_CHOICES`].
pub fn value_input(field: Field) -> ValueInput {
    FIELD_CHOICES
        .iter()
        .find(|c| c.field == field)
        .map_or(ValueInput::Text, |c| c.input)
}

/// The picker's field rows matching `query`, best match first.
pub fn rank_fields(query: &str) -> Vec<FieldChoice> {
    let mut found: Vec<(fuzzy::Rank, FieldChoice)> = FIELD_CHOICES
        .iter()
        .filter_map(|c| fuzzy::rank(c.label, c.field.key(), query).map(|r| (r, *c)))
        .collect();
    found.sort_by_key(|(rank, _)| *rank);
    found.into_iter().map(|(_, c)| c).collect()
}

/// One selectable value: what goes into the query and how the picker reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub value: String,
    pub label: String,
}

fn choice(value: &str, label: &str) -> Choice {
    Choice { value: value.to_owned(), label: label.to_owned() }
}

fn tag_label(value: &str) -> &'static str {
    match value {
        "needs-reply" => "Needs reply",
        "awaiting" => "Awaiting reply",
        "follow-up" => "Follow up",
        "reminder" => "Reminder",
        "spam" => "Possible spam",
        "urgent" => "Urgent",
        _ => "",
    }
}

fn is_label(value: &str) -> &'static str {
    match value {
        "inbox" => "Inbox",
        "snoozed" => "Snoozed",
        "archived" => "Archived",
        "filed" => "Filed",
        "deleted" => "Deleted",
        "sent" => "Sent",
        "new" => "New sender",
        _ => "",
    }
}

fn title_case(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map_or_else(String::new, |c| c.to_uppercase().chain(chars).collect())
}

/// Every value the picker lists for a `Pick` field, in display order. Empty for fields that
/// take free text or a date.
pub fn value_choices(field: Field, mailbox: &Mailbox) -> Vec<Choice> {
    match field {
        Field::Tag => TAG_VALUES.iter().map(|v| choice(v, tag_label(v))).collect(),
        Field::Is => IS_VALUES
            .iter()
            .filter(|v| KNOWN_SENDERS_ENABLED || **v != "new")
            .map(|v| choice(v, is_label(v)))
            .collect(),
        Field::Kind => Kind::ALL.iter().map(|k| choice(k.label(), &title_case(k.label()))).collect(),
        Field::Account => mailbox
            .accounts()
            .iter()
            .map(|a| choice(&a.id.to_lowercase(), crate::account_style::display_name(a)))
            .collect(),
        Field::In => {
            let mut out: Vec<Choice> = [
                ("inbox", "Inbox"),
                ("sent", "Sent"),
                ("snoozed", "Snoozed"),
                ("archived", "Archive"),
                ("deleted", "Trash"),
            ]
            .iter()
            .map(|(v, l)| choice(v, l))
            .collect();
            for account in mailbox.accounts() {
                for folder in mailbox.folders(&account.id) {
                    let path = mailbox.folder_path(folder.id);
                    let value = path.to_lowercase();
                    if !out.iter().any(|c| c.value == value) {
                        out.push(Choice { value, label: path });
                    }
                }
            }
            out
        }
        _ => Vec::new(),
    }
}

/// `choices` matching `query` by label or value, best match first.
pub fn rank_choices(choices: Vec<Choice>, query: &str) -> Vec<Choice> {
    let mut found: Vec<(fuzzy::Rank, Choice)> = choices
        .into_iter()
        .filter_map(|c| {
            let rank = fuzzy::rank(&c.label, &c.value, query).or_else(|| fuzzy::rank(&c.value, "", query))?;
            Some((rank, c))
        })
        .collect();
    found.sort_by_key(|(rank, _)| *rank);
    found.into_iter().map(|(_, c)| c).collect()
}

/// What a pill shows for one value: accounts by name, everything else as stored.
pub fn value_label(field: Field, value: &str, mailbox: &Mailbox) -> String {
    match field {
        Field::Account => mailbox
            .accounts()
            .iter()
            .find(|a| a.id.to_lowercase() == value)
            .map_or_else(|| value.to_owned(), |a| crate::account_style::display_name(a).to_owned()),
        _ => value.to_owned(),
    }
}

/// What number key `1`–`6` (index `0`–`5`) does to the filters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuickFilter {
    /// Drop every pill except the folder.
    Clear,
    /// Add the pill, or remove it when already there.
    Toggle(Field, &'static str),
}

/// The quick filter on key `index + 1`: all, needs reply, follow up, urgent, new senders
/// (only while the known-sender distinction is on) and possible spam.
pub fn quick_filter(index: usize) -> Option<QuickFilter> {
    match index {
        0 => Some(QuickFilter::Clear),
        1 => Some(QuickFilter::Toggle(Field::Tag, "needs-reply")),
        2 => Some(QuickFilter::Toggle(Field::Tag, "follow-up")),
        3 => Some(QuickFilter::Toggle(Field::Tag, "urgent")),
        4 if KNOWN_SENDERS_ENABLED => Some(QuickFilter::Toggle(Field::Is, "new")),
        5 => Some(QuickFilter::Toggle(Field::Tag, "spam")),
        _ => None,
    }
}

/// Palette names of the quick filters, by key index.
pub const QUICK_FILTER_NAMES: [&str; 6] = [
    "Filter: clear",
    "Filter: needs reply",
    "Filter: follow up",
    "Filter: urgent",
    "Filter: new senders",
    "Filter: possible spam",
];

/// `(year, month, day)` of an absolute `yyyy-mm-dd` value.
pub fn parse_ymd(value: &str) -> Option<(i32, u32, u32)> {
    let mut parts = value.split('-');
    let (y, m, d) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || y.len() != 4 || m.len() != 2 || d.len() != 2 {
        return None;
    }
    Some((y.parse().ok()?, m.parse().ok()?, d.parse().ok()?))
}

/// `yyyy-mm-dd` of a calendar day.
pub fn format_ymd(year: i32, month: u32, day: u32) -> String {
    format!("{year:04}-{month:02}-{day:02}")
}
