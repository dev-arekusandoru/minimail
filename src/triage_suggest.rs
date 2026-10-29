//! The Triage mode's action suggestion: a deterministic stand-in for the future AI one. From the
//! mailbox history alone it proposes what to do with a message, or nothing.
//!
//! * Possible Spam tag → Delete.
//! * The sender has at least [`MIN_HISTORY`] earlier messages that left the Inbox and
//!   [`MOSTLY`] of them were archived → Archive; filed to one folder → File to it.
//!
//! Anything else gets no suggestion. Snoozed and Inbox mail is not history: it was not decided.

use crate::model::{FolderId, Mailbox, MessageId, Tag, TriageState};

/// Earlier decided messages from the sender needed before history speaks.
const MIN_HISTORY: usize = 3;
/// Share of that history (in percent) that must agree.
const MOSTLY: usize = 80;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuggestedAction {
    Archive,
    Delete,
    File(FolderId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionSuggestion {
    pub action: SuggestedAction,
    /// Why, as the card shows it: "you've archived 9 of 9 from this sender".
    pub reason: String,
}

fn agrees(part: usize, whole: usize) -> bool {
    part * 100 >= whole * MOSTLY
}

pub fn suggest(mailbox: &Mailbox, id: MessageId) -> Option<ActionSuggestion> {
    let msg = mailbox.get(id)?;
    if msg.outgoing {
        return None;
    }
    if mailbox.tags(id).contains(&Tag::PossibleSpam) {
        return Some(ActionSuggestion {
            action: SuggestedAction::Delete,
            reason: "flagged as possible spam".to_owned(),
        });
    }
    let decided: Vec<TriageState> = mailbox
        .messages()
        .iter()
        .filter(|m| {
            m.id != id
                && !m.outgoing
                && m.from_email.eq_ignore_ascii_case(&msg.from_email)
                && matches!(m.state, TriageState::Archived | TriageState::Filed(_) | TriageState::Deleted)
        })
        .map(|m| m.state)
        .collect();
    let total = decided.len();
    if total < MIN_HISTORY {
        return None;
    }
    let archived = decided.iter().filter(|s| **s == TriageState::Archived).count();
    if agrees(archived, total) {
        return Some(ActionSuggestion {
            action: SuggestedAction::Archive,
            reason: format!("you've archived {archived} of {total} from this sender"),
        });
    }
    let (folder, filed) = decided
        .iter()
        .filter_map(|s| match s {
            TriageState::Filed(f) => Some(*f),
            _ => None,
        })
        .map(|f| (f, decided.iter().filter(|s| **s == TriageState::Filed(f)).count()))
        .max_by_key(|(f, n)| (*n, std::cmp::Reverse(*f)))?;
    let name = mailbox.folder(folder)?.name.clone();
    agrees(filed, total).then(|| ActionSuggestion {
        action: SuggestedAction::File(folder),
        reason: format!("you've filed {filed} of {total} from this sender to {name}"),
    })
}
