//! Drag payloads: what a drag out of the message list carries.
//!
//! Plain data, so the rules below are testable without a UI: which messages a drag applies to,
//! and which message a drop on the reader tab strip opens. Views own the chip and the wiring.

use crate::model::{Location, MessageId, TriageState};

/// A drag that started on a sidebar folder row: the view it stands for, and the label the
/// chip that follows the pointer shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderDrag {
    /// The location a drop on the list strip opens.
    pub location: Location,
    /// The row's label, e.g. `"Projects"`, for the drag chip.
    pub label: String,
}

/// A drag that started on a list row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MailDrag {
    /// Messages a drop applies to: the selection when the dragged row is part of it, else the
    /// row's own messages. Snapshotted when the row is built, so a drop can never pick up a
    /// selection change from mid-drag.
    pub ids: Vec<MessageId>,
    /// The row's own message: what a drop on the tab strip opens, even when the drag carries a
    /// whole selection.
    pub anchor: MessageId,
    /// Thread the row belongs to, when the list knows one.
    pub thread: Option<u32>,
}

impl MailDrag {
    /// A drag from a row showing `row_ids`, given the currently `selected` messages. `None` when
    /// the row shows nothing, so no row ever carries an empty payload.
    pub fn for_row(selected: &[MessageId], row_ids: &[MessageId], thread: Option<u32>) -> Option<Self> {
        let anchor = *row_ids.first()?;
        Some(Self { ids: payload_ids(selected, row_ids), anchor, thread })
    }

    /// Messages the drop would move.
    pub fn count(&self) -> usize {
        self.ids.len()
    }
}

/// Messages a drag that started on a row showing `row_ids` carries: the whole selection when the
/// row is part of it, else just the row.
pub fn payload_ids(selected: &[MessageId], row_ids: &[MessageId]) -> Vec<MessageId> {
    if !selected.is_empty() && row_ids.iter().any(|id| selected.contains(id)) {
        return selected.to_vec();
    }
    row_ids.to_vec()
}

/// What dropping messages on a list tab (or the list pane, standing for its active tab) does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropAction {
    /// Move the messages to this state.
    File(TriageState),
    /// Open the snooze picker for the messages.
    Snooze,
    /// The location accepts no messages.
    Nothing,
}

/// The action a message drop on a tab showing `location` performs.
pub fn drop_action(location: &Location) -> DropAction {
    match location {
        Location::Inbox(_) => DropAction::File(TriageState::Inbox),
        Location::Archive(_) | Location::AllArchive => DropAction::File(TriageState::Archived),
        Location::Trash(_) | Location::AllTrash => DropAction::File(TriageState::Deleted),
        Location::Folder(id) => DropAction::File(TriageState::Filed(*id)),
        Location::Snoozed(_) => DropAction::Snooze,
        Location::AllInboxes | Location::Sent(_) | Location::AllSent | Location::AllSnoozed => DropAction::Nothing,
    }
}
