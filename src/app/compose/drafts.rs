//! The Drafts list, as the main pane shows it: every stored draft of one account with its
//! recipient, subject, snippet and when it was last touched. Pure functions, no UI.
//!
//! Closing a composer keeps its draft, and the durable store is the only thing that holds
//! it, so there has to be a way back to it. The sidebar's per-account *Drafts* row opens
//! this list; choosing a row reopens that draft in a composer tab. Nothing is fetched,
//! because the record is already local.

use crate::clock::Timestamp;
use crate::outgoing::{DraftId, DraftRecord, DraftState};
use crate::search::{Combinator, Field, Query};

/// One row of the Drafts pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DraftRow {
    pub id: DraftId,
    /// The account that owns the draft, so a row opened from the list binds to the right one.
    pub account: String,
    /// Recipients as written, e.g. `"cal@example.com, dee@example.com"`.
    pub recipients: String,
    pub subject: String,
    /// Plain-text snippet of the body, the muted line under the subject.
    pub snippet: String,
    /// When the draft was last touched, for the time column.
    pub updated_at: Timestamp,
    pub state: DraftState,
}

impl DraftRow {
    /// The recipient column: the addresses, or `"(no recipient)"` for a draft with none.
    pub fn recipient_label(&self) -> &str {
        if self.recipients.is_empty() {
            "(no recipient)"
        } else {
            &self.recipients
        }
    }

    /// The subject column: the subject, or `"(no subject)"` for a draft without one.
    pub fn subject_label(&self) -> &str {
        if self.subject.is_empty() {
            "(no subject)"
        } else {
            &self.subject
        }
    }

    /// How the draft's own state reads beside the subject.
    pub fn state_note(&self) -> &'static str {
        match self.state {
            DraftState::Editing => "editing",
            DraftState::Held => "held",
            DraftState::Sending => "sending",
            DraftState::Sent { .. } => "sent",
            DraftState::Uncertain => "not confirmed",
            DraftState::Failed { .. } => "not sent",
            DraftState::Conflict => "changed on the server",
        }
    }
}

/// Whether the Drafts list would offer this draft. A delivered message is not a draft to
/// finish: the Sent folder is where it is read, and offering a "send" on it would only fail.
/// Everything else is offered, with its state shown, including one whose send could not be
/// confirmed.
pub fn listed(record: &DraftRecord) -> bool {
    !matches!(record.state, DraftState::Sent { .. })
}

/// The drafts of `account` as the pane lists them: unfinished, matching `filter`, newest
/// edit first (ties by id, so the order is stable). One row per draft.
pub fn rows<'a>(
    records: impl IntoIterator<Item = &'a DraftRecord>,
    account: &str,
    filter: &Query,
) -> Vec<DraftRow> {
    let mut out: Vec<DraftRow> = records
        .into_iter()
        .filter(|record| record.account == account && listed(record) && matches(record, filter))
        .map(row)
        .collect();
    // Newest first; the id keeps drafts edited in the same second in a stable order.
    out.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then_with(|| b.id.cmp(&a.id))
    });
    out
}

/// Whether a draft satisfies `filter`, using the same query shape as message search: every
/// field group must match (its `and`/`or` combinator respected) and every free-text term
/// must appear. Drafts are not messages, so only the fields a draft has are matched —
/// subject, the To/Cc/Bcc recipients, the body and the account; `in:drafts` holds by
/// definition. A field a draft has no value for (a sender, a tag, a triage state, a date)
/// does not match, so those pills list no drafts rather than pretending otherwise.
pub fn matches(record: &DraftRecord, filter: &Query) -> bool {
    for field in Field::ALL {
        let values = filter.values(field);
        if values.is_empty() {
            continue;
        }
        let hit = |value: &String| field_matches(record, field, value);
        let ok = match filter.combinator(field) {
            Some(Combinator::Or) => values.iter().any(hit),
            _ => values.iter().all(hit),
        };
        if !ok {
            return false;
        }
    }
    let hay = haystack(record);
    filter.text().iter().all(|term| hay.contains(term))
}

fn field_matches(record: &DraftRecord, field: Field, value: &str) -> bool {
    let has = |text: &str| text.to_lowercase().contains(value);
    match field {
        Field::Subject => has(record.subject.trim()),
        Field::To => has(&record.to),
        Field::Cc => has(&record.cc),
        Field::Bcc => has(&record.bcc),
        Field::Body => has(&record.body.source),
        Field::Account => record.account.to_lowercase() == value,
        // The pane is the Drafts folder: its own `in:` value holds, no other place does.
        Field::In => value == "drafts",
        // A sender, tag, kind, state or date pill: a draft has none of these.
        _ => false,
    }
}

/// What free text searches, lowercased: the subject, every recipient field and the body.
fn haystack(record: &DraftRecord) -> String {
    format!(
        "{}\n{}\n{}\n{}\n{}",
        record.subject, record.to, record.cc, record.bcc, record.body.source
    )
    .to_lowercase()
}

fn row(record: &DraftRecord) -> DraftRow {
    DraftRow {
        id: record.id,
        account: record.account.clone(),
        recipients: recipients(record),
        subject: record.subject.trim().to_owned(),
        snippet: crate::preview::snippet(&record.body.source),
        updated_at: record.updated_at,
        state: record.state.clone(),
    }
}

/// The recipient column's text: To, else Cc, else Bcc, as written.
fn recipients(record: &DraftRecord) -> String {
    [&record.to, &record.cc, &record.bcc]
        .into_iter()
        .map(|s| s.trim())
        .find(|s| !s.is_empty())
        .unwrap_or_default()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::rows;
    use crate::outgoing::{DraftId, DraftRecord, DraftState};
    use crate::search::Query;

    fn draft(id: i64, subject: &str, to: &str) -> DraftRecord {
        let mut r = DraftRecord::new_email("personal", 1);
        r.id = DraftId(id);
        r.subject = subject.to_owned();
        r.to = to.to_owned();
        r.updated_at = id;
        r
    }

    #[test]
    fn a_delivered_message_is_not_listed_as_a_draft_to_finish() {
        let mut sent = draft(1, "Lunch", "cal@example.com");
        sent.state = DraftState::Sent {
            remote_id: "mid".into(),
        };
        assert!(
            rows([&sent], "personal", &Query::default()).is_empty(),
            "Sent lives in the Sent folder, not in Drafts"
        );
    }

    #[test]
    fn only_the_named_accounts_drafts_are_listed() {
        let mut other = draft(2, "Work note", "boss@corp.io");
        other.account = "work".into();
        let rows = rows(
            [&draft(1, "Personal note", "cal@example.com"), &other],
            "personal",
            &Query::default(),
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].subject, "Personal note");
    }

    #[test]
    fn newest_edit_comes_first() {
        let rows = rows(
            [
                &draft(1, "Older", "a@x.com"),
                &draft(3, "Newest", "b@x.com"),
                &draft(2, "Middle", "c@x.com"),
            ],
            "personal",
            &Query::default(),
        );
        let subjects: Vec<&str> = rows.iter().map(|r| r.subject.as_str()).collect();
        assert_eq!(subjects, ["Newest", "Middle", "Older"]);
    }

    #[test]
    fn subject_and_recipient_fall_back_to_placeholders() {
        let rows = rows([&draft(1, "", "")], "personal", &Query::default());
        assert_eq!(rows[0].subject_label(), "(no subject)");
        assert_eq!(rows[0].recipient_label(), "(no recipient)");
    }

    #[test]
    fn the_snippet_drops_quoted_lines_and_signatures() {
        let mut quoted = draft(1, "Re: Budget", "cal@example.com");
        quoted.body.source = "Here is the answer.\n\n> quoted text\n-- \nsig".into();
        let rows = rows([&quoted], "personal", &Query::default());
        assert_eq!(rows[0].snippet, "Here is the answer.");
    }

    /// The pane narrows by the same query the search box builds: free text matches subject,
    /// recipients and body (case-insensitively), and recipient fields, the account and the
    /// pane's own `in:drafts` hold. A place a draft is not in matches nothing.
    #[test]
    fn the_query_narrows_the_drafts() {
        let mut a = draft(1, "Budget question", "cal@example.com");
        a.body.source = "the Quarterly numbers".into();
        let mut b = draft(2, "Lunch", "dee@example.com");
        b.cc = "chef@example.com".into();
        let listed = |q: &str| rows([&a, &b], "personal", &Query::parse(q)).len();

        assert_eq!(listed(""), 2, "no query lists every draft");
        assert_eq!(listed("budget"), 1, "subject, case-insensitively");
        assert_eq!(listed("QUARTERLY"), 1, "body text");
        assert_eq!(listed("dee"), 1, "recipient free text");
        assert_eq!(listed("chef"), 1, "cc free text");
        assert_eq!(listed("to:cal"), 1, "a to: field");
        assert_eq!(listed("account:personal"), 2, "the account matches");
        assert_eq!(listed("in:drafts"), 2, "the pane's own place holds");
        assert_eq!(listed("in:inbox"), 0, "a draft is not in the inbox");
        assert_eq!(listed("lunch cheese"), 0, "every term must match");
    }
}
