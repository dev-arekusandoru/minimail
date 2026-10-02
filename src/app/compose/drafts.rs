//! The Drafts picker: the way back to a draft that is not in a tab.
//!
//! Closing a composer keeps its draft, and the durable store is the only thing that holds
//! it, so there has to be a list the user can come back to from anywhere. This is that list:
//! every stored draft, its subject, its recipients and when it was last touched, filtered by
//! typing. Choosing a row reopens that draft in its own tab; nothing is fetched to do it,
//! because the record is already local.

use std::rc::Rc;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{
    base::IndexPath,
    component::command::{Command, CommandItem, CommandState},
    *,
};

use crate::outgoing::{DraftId, DraftRecord, DraftState};

/// What choosing a row means.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DraftsEvent {
    /// Reopen this draft in its own tab.
    Open(DraftId),
}

/// The drafts as the picker shows them: the record and a searchable label per draft.
pub fn rows(records: &[DraftRecord], query: &str) -> Vec<(DraftId, String)> {
    let needle = query.trim().to_lowercase();
    records
        .iter()
        // A delivered message is not a draft to finish: the Sent folder is where it is read,
        // and offering a "send" on it would only fail. Everything else is offered, with its
        // state shown, including one whose send could not be confirmed.
        .filter(|record| !matches!(record.state, DraftState::Sent { .. }))
        .filter_map(|record| {
            let subject = subject_of(record);
            let recipients = first_recipient(record);
            let label = if subject.is_empty() {
                if recipients.is_empty() {
                    "(no subject)".to_owned()
                } else {
                    format!("(no subject) · {recipients}")
                }
            } else if recipients.is_empty() {
                subject
            } else {
                format!("{subject} · {recipients}")
            };
            let matches = needle.is_empty()
                || label.to_lowercase().contains(&needle)
                || record.account.to_lowercase().contains(&needle);
            matches.then_some((record.id, label))
        })
        .collect()
}

fn subject_of(record: &DraftRecord) -> String {
    let subject = record.subject.trim();
    if subject.is_empty() {
        String::new()
    } else {
        format!("{subject}  ·  {}", state_note(&record.state))
    }
}

fn first_recipient(record: &DraftRecord) -> String {
    record
        .to
        .split(',')
        .map(str::trim)
        .find(|part| !part.is_empty())
        .unwrap_or_default()
        .to_owned()
}

fn state_note(state: &DraftState) -> &'static str {
    match state {
        DraftState::Editing => "editing",
        DraftState::Held => "held",
        DraftState::Sending => "sending",
        DraftState::Sent { .. } => "sent",
        DraftState::Uncertain => "not confirmed",
        DraftState::Failed { .. } => "not sent",
        DraftState::Conflict => "changed on the server",
    }
}

/// The drafts picker, hosted in a dialog by the owner.
pub struct DraftsPicker {
    state: Entity<CommandState>,
    drafts: Vec<(DraftId, String)>,
    query: String,
    /// Every stored draft, so the rows can be re-derived when the filter changes.
    records: Vec<DraftRecord>,
}

impl DraftsPicker {
    pub fn new(records: Vec<DraftRecord>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            state: cx.new(|cx| CommandState::new(window, cx)),
            drafts: rows(&records, ""),
            query: String::new(),
            records,
        }
    }

    /// The visible rows, top to bottom.
    pub fn rows(&self) -> Vec<String> {
        self.drafts.iter().map(|(_, label)| label.clone()).collect()
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.state
            .clone()
            .update(cx, |state, cx| state.focus(window, cx));
    }
}

impl EventEmitter<DraftsEvent> for DraftsPicker {}

impl Render for DraftsPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let weak = cx.weak_entity();
        let labels: Vec<SharedString> = self
            .drafts
            .iter()
            .map(|(_, label)| label.clone().into())
            .collect();
        let events: Rc<Vec<DraftsEvent>> = Rc::new(
            self.drafts
                .iter()
                .map(|(id, _)| DraftsEvent::Open(*id))
                .collect(),
        );
        let on_query = weak.clone();
        Command::new(&self.state)
            .bordered(false)
            .filterable(false)
            .max_h(px(360.))
            .placeholder("Filter drafts")
            .header(move |_, _, cx| {
                div()
                    .px_3()
                    .pt_3()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(cx.theme().foreground)
                    .child("Drafts")
            })
            .empty(|_, _, cx| {
                div()
                    .p_4()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("No drafts on this Mac.")
            })
            .items(
                labels
                    .into_iter()
                    .map(|label| CommandItem::new().label(label)),
            )
            .on_query(move |q, _, cx| {
                on_query
                    .update(cx, |picker, cx| {
                        let q = q.trim().to_owned();
                        if picker.query != q {
                            picker.query = q;
                            picker.drafts = rows(&picker.records, &picker.query);
                            cx.notify();
                        }
                    })
                    .ok();
            })
            .on_confirm(move |ix: IndexPath, _, cx| {
                let Some(DraftsEvent::Open(id)) = events.get(ix.row) else {
                    return;
                };
                weak.update(cx, |_, cx| cx.emit(DraftsEvent::Open(*id)))
                    .ok();
            })
    }
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
        let listed = rows(&[sent], "");
        assert!(
            listed.is_empty(),
            "Sent lives in the Sent folder, not in Drafts"
        );
    }

    #[test]
    fn the_filter_matches_the_subject_the_recipient_and_the_account() {
        let drafts = vec![draft(1, "Budget question", "cal@example.com")];
        assert_eq!(rows(&drafts, "budget").len(), 1, "subject");
        assert_eq!(
            rows(&drafts, "CAL@").len(),
            1,
            "recipient, case-insensitively"
        );
        assert_eq!(rows(&drafts, "personal").len(), 1, "account");
        assert!(
            rows(&drafts, "lunch").is_empty(),
            "an unmatched filter shows nothing"
        );
    }
}
