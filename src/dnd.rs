//! Drag payloads: what a drag out of the message list carries.
//!
//! Plain data, so the rules below are testable without a UI: which messages a drag applies to,
//! and which message a drop on the reader tab strip opens. Views own the chip and the wiring.

use crate::model::MessageId;

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
