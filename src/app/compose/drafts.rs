//! The Drafts list, as the main pane shows it: every stored draft of one account with its
//! recipient, subject, snippet and when it was last touched. Pure functions, no UI.
//!
//! Closing a composer keeps its draft, and the durable store is the only thing that holds
//! it, so there has to be a way back to it. The sidebar's per-account *Drafts* row opens
//! this list; choosing a row reopens that draft in a composer tab. Nothing is fetched,
//! because the record is already local.

use crate::clock::Timestamp;
use crate::outgoing::{DraftId, DraftRecord, DraftState};

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

/// The drafts of `account` as the pane lists them: unfinished, newest edit first (ties by
/// id, so the order is stable). One row per draft.
pub fn rows<'a>(records: impl IntoIterator<Item = &'a DraftRecord>, account: &str) -> Vec<DraftRow> {
    let mut out: Vec<DraftRow> = records
        .into_iter()
        .filter(|record| record.account == account && listed(record))
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
            rows([&sent], "personal").is_empty(),
            "Sent lives in the Sent folder, not in Drafts"
        );
    }

    #[test]
    fn only_the_named_accounts_drafts_are_listed() {
        let mut other = draft(2, "Work note", "boss@corp.io");
        other.account = "work".into();
        let rows = rows([&draft(1, "Personal note", "cal@example.com"), &other], "personal");
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
        );
        let subjects: Vec<&str> = rows.iter().map(|r| r.subject.as_str()).collect();
        assert_eq!(subjects, ["Newest", "Middle", "Older"]);
    }

    #[test]
    fn subject_and_recipient_fall_back_to_placeholders() {
        let rows = rows([&draft(1, "", "")], "personal");
        assert_eq!(rows[0].subject_label(), "(no subject)");
        assert_eq!(rows[0].recipient_label(), "(no recipient)");
    }

    #[test]
    fn the_snippet_drops_quoted_lines_and_signatures() {
        let mut quoted = draft(1, "Re: Budget", "cal@example.com");
        quoted.body.source = "Here is the answer.\n\n> quoted text\n-- \nsig".into();
        let rows = rows([&quoted], "personal");
        assert_eq!(rows[0].snippet, "Here is the answer.");
    }
}
