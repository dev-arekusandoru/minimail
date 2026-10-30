//! The send hold: a durable ten-second window with undo, and the honest
//! outcome of a send the app cannot be sure about.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::clock::Timestamp;
use crate::model::AccountId;
use crate::provider::RemoteId;

use super::draft::{DraftId, DraftRecord, DraftState};

/// How long a send waits before it goes out, giving the user time to undo.
pub const HOLD_SECONDS: Timestamp = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HoldReason {
    UserSend,
    /// Send later: held until the chosen time, and kept across a restart.
    Scheduled,
}

/// A scheduled send waiting out its hold. Persisted, so a restart can see it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingHold {
    pub draft: DraftId,
    pub account: AccountId,
    pub created_at: Timestamp,
    pub hold_until: Timestamp,
    pub reason: HoldReason,
}

impl PendingHold {
    pub fn start(draft: DraftId, account: AccountId, now: Timestamp) -> Self {
        Self {
            draft,
            account,
            created_at: now,
            hold_until: now + HOLD_SECONDS,
            reason: HoldReason::UserSend,
        }
    }

    /// A send held until `at` (never sooner than the undo window).
    pub fn scheduled(draft: DraftId, account: AccountId, now: Timestamp, at: Timestamp) -> Self {
        Self {
            hold_until: at.max(now + HOLD_SECONDS),
            reason: HoldReason::Scheduled,
            ..Self::start(draft, account, now)
        }
    }

    pub fn is_scheduled(&self) -> bool {
        self.reason == HoldReason::Scheduled
    }

    pub fn is_due(&self, now: Timestamp) -> bool {
        now >= self.hold_until
    }
}

/// What the countdown toast shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HoldProgress {
    pub remaining: Duration,
    /// 1.0 at the start of the hold, 0.0 when the send goes out.
    pub fraction: f32,
    pub due: bool,
}

/// Time left before the send goes out; `None` once the hold is over.
pub fn hold_remaining(hold: &PendingHold, now: Timestamp) -> Option<Duration> {
    if hold.is_due(now) {
        return None;
    }
    Some(Duration::from_secs((hold.hold_until - now).max(0) as u64))
}

pub fn hold_progress(hold: &PendingHold, now: Timestamp) -> HoldProgress {
    let remaining = hold_remaining(hold, now).unwrap_or_default();
    let fraction = if HOLD_SECONDS <= 0 {
        0.0
    } else {
        (remaining.as_secs_f32() / HOLD_SECONDS as f32).clamp(0.0, 1.0)
    };
    HoldProgress {
        remaining,
        fraction,
        due: hold.is_due(now),
    }
}

/// What came back from the provider when a send was attempted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SendOutcome {
    Sent {
        remote_id: RemoteId,
    },
    /// Never left the machine: the draft stays as it is.
    Offline,
    Error {
        message: String,
        retryable: bool,
    },
    /// The request may or may not have been delivered.
    Uncertain {
        remote_id: Option<RemoteId>,
    },
}

/// The result of looking for an uncertain send in the provider's sent mail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SentLookup {
    Found(RemoteId),
    NotFound,
    /// The check itself failed; it must be retried, not read as an answer.
    Failed(String),
}

/// What an uncertain send may safely become. A lookup that finds nothing is
/// **not** proof of non-delivery — providers index late — so the draft stays
/// uncertain until the user decides otherwise.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reconciliation {
    Confirmed { remote_id: RemoteId },
    Unconfirmed,
    Unavailable { message: String },
}

impl Reconciliation {
    pub fn is_confirmed(&self) -> bool {
        matches!(self, Reconciliation::Confirmed { .. })
    }

    pub fn draft_state(&self) -> DraftState {
        match self {
            Reconciliation::Confirmed { remote_id } => DraftState::Sent {
                remote_id: remote_id.clone(),
            },
            Reconciliation::Unconfirmed | Reconciliation::Unavailable { .. } => {
                DraftState::Uncertain
            }
        }
    }
}

pub fn reconcile_uncertain(lookup: SentLookup) -> Reconciliation {
    match lookup {
        SentLookup::Found(remote_id) => Reconciliation::Confirmed { remote_id },
        SentLookup::NotFound => Reconciliation::Unconfirmed,
        SentLookup::Failed(message) => Reconciliation::Unavailable { message },
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoldAction {
    /// The user hit undo: the whole draft, attachments included, comes back.
    Undo,
    /// The hold is over: the send goes out.
    Flush,
}

#[derive(Clone, Debug, PartialEq)]
pub enum HoldTransition {
    /// The complete draft, attachments and Message-ID included, editable again.
    Restored(Box<DraftRecord>),
    Sending {
        draft: DraftId,
    },
}

impl DraftRecord {
    /// Apply a hold action to the record, returning what should be persisted.
    pub fn apply_hold(
        &self,
        hold: &PendingHold,
        action: HoldAction,
        now: Timestamp,
    ) -> HoldTransition {
        match action {
            HoldAction::Undo => {
                let mut restored = self.clone();
                restored.state = DraftState::Editing;
                restored.touch(now);
                HoldTransition::Restored(Box::new(restored))
            }
            HoldAction::Flush => HoldTransition::Sending { draft: hold.draft },
        }
    }
}

/// The holds in flight, for the countdown UI and the send worker.
#[derive(Clone, Debug, Default)]
pub struct HoldRegistry {
    holds: Vec<PendingHold>,
}

impl HoldRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_holds(holds: impl IntoIterator<Item = PendingHold>) -> Self {
        Self {
            holds: holds.into_iter().collect(),
        }
    }

    pub fn insert(&mut self, hold: PendingHold) {
        self.holds.retain(|h| h.draft != hold.draft);
        self.holds.push(hold);
    }

    pub fn remove(&mut self, draft: DraftId) -> Option<PendingHold> {
        let index = self.holds.iter().position(|h| h.draft == draft)?;
        Some(self.holds.remove(index))
    }

    pub fn get(&self, draft: DraftId) -> Option<&PendingHold> {
        self.holds.iter().find(|h| h.draft == draft)
    }

    /// Holds whose time is up, oldest first.
    pub fn due(&self, now: Timestamp) -> Vec<PendingHold> {
        let mut due: Vec<PendingHold> = self
            .holds
            .iter()
            .filter(|h| h.is_due(now))
            .cloned()
            .collect();
        due.sort_by_key(|h| h.hold_until);
        due
    }

    pub fn iter(&self) -> impl Iterator<Item = &PendingHold> {
        self.holds.iter()
    }

    pub fn len(&self) -> usize {
        self.holds.len()
    }

    pub fn is_empty(&self) -> bool {
        self.holds.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outgoing::draft::{DraftKind, DraftRecord};

    fn draft() -> DraftRecord {
        let mut draft = DraftRecord::new_email("me@x.test", 100);
        draft.kind = DraftKind::Reply;
        draft.to = "sam@x.test".to_owned();
        draft.subject = "Re: Lunch".to_owned();
        draft
    }

    #[test]
    fn a_scheduled_hold_waits_for_its_time_but_never_less_than_the_undo_window() {
        let later = PendingHold::scheduled(DraftId(1), "me@x.test".to_owned(), 100, 5_000);
        assert!(later.is_scheduled());
        assert_eq!(later.hold_until, 5_000);
        assert!(!later.is_due(4_999));
        assert!(later.is_due(5_000));
        let past = PendingHold::scheduled(DraftId(1), "me@x.test".to_owned(), 100, 50);
        assert_eq!(past.hold_until, 100 + HOLD_SECONDS, "a past time still gets the undo window");
    }

    #[test]
    fn hold_counts_down_over_ten_seconds() {
        let hold = PendingHold::start(DraftId(1), "me@x.test".to_owned(), 100);
        assert_eq!(hold.hold_until, 110);
        let start = hold_progress(&hold, 100);
        assert_eq!(start.remaining, Duration::from_secs(10));
        assert_eq!(start.fraction, 1.0);
        assert!(!start.due);
        let middle = hold_progress(&hold, 105);
        assert_eq!(middle.remaining, Duration::from_secs(5));
        assert!((middle.fraction - 0.5).abs() < 1e-6, "{}", middle.fraction);
        let end = hold_progress(&hold, 110);
        assert!(end.due);
        assert_eq!(end.remaining, Duration::ZERO);
        assert_eq!(end.fraction, 0.0);
        assert_eq!(
            hold_remaining(&hold, 200),
            None,
            "an elapsed hold has no time left"
        );
        assert_eq!(hold_remaining(&hold, 108), Some(Duration::from_secs(2)));
    }

    #[test]
    fn undo_restores_the_complete_editable_draft() {
        let mut draft = draft();
        draft.attempted = false;
        draft.state = DraftState::Held;
        let hold = PendingHold::start(draft.id, draft.account.clone(), 100);

        let HoldTransition::Restored(restored) = draft.apply_hold(&hold, HoldAction::Undo, 103)
        else {
            panic!("undo restores");
        };
        let restored = *restored;
        assert_eq!(restored.state, DraftState::Editing);
        assert_eq!(restored.to, "sam@x.test");
        assert_eq!(restored.subject, "Re: Lunch");
        assert_eq!(restored.body, draft.body);
        assert_eq!(restored.message_id, draft.message_id);
        assert_eq!(restored.updated_at, 103);
    }

    #[test]
    fn flush_hands_the_draft_to_the_sender() {
        let draft = draft();
        let hold = PendingHold::start(draft.id, draft.account.clone(), 100);
        let transition = draft.apply_hold(&hold, HoldAction::Flush, 110);
        assert_eq!(transition, HoldTransition::Sending { draft: draft.id });
    }

    #[test]
    fn registry_tracks_due_holds() {
        let mut registry = HoldRegistry::new();
        registry.insert(PendingHold::start(DraftId(1), "a@x.test".to_owned(), 100));
        registry.insert(PendingHold::start(DraftId(2), "b@x.test".to_owned(), 105));
        assert_eq!(registry.len(), 2);
        assert!(registry.get(DraftId(2)).is_some());

        assert!(
            registry.due(108).is_empty(),
            "the second hold is still counting down"
        );
        let due = registry.due(110);
        assert_eq!(
            due.iter().map(|h| h.draft).collect::<Vec<_>>(),
            vec![DraftId(1)]
        );

        // Replacing a hold for the same draft keeps one entry.
        registry.insert(PendingHold::start(DraftId(1), "a@x.test".to_owned(), 108));
        assert_eq!(registry.len(), 2);

        assert_eq!(registry.remove(DraftId(1)).map(|h| h.created_at), Some(108));
        assert!(registry.remove(DraftId(1)).is_none());
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn a_lookup_that_finds_nothing_does_not_clear_uncertainty() {
        assert_eq!(
            reconcile_uncertain(SentLookup::NotFound),
            Reconciliation::Unconfirmed
        );
        assert_eq!(
            Reconciliation::Unconfirmed.draft_state(),
            DraftState::Uncertain
        );
        assert_eq!(
            reconcile_uncertain(SentLookup::Failed("timeout".to_owned())),
            Reconciliation::Unavailable {
                message: "timeout".to_owned()
            }
        );
        assert_eq!(
            Reconciliation::Unavailable {
                message: "timeout".to_owned()
            }
            .draft_state(),
            DraftState::Uncertain
        );
    }

    #[test]
    fn a_found_message_id_settles_the_send() {
        let r = reconcile_uncertain(SentLookup::Found("m1".to_owned()));
        assert!(r.is_confirmed());
        assert_eq!(
            r.draft_state(),
            DraftState::Sent {
                remote_id: "m1".to_owned()
            }
        );
    }

    #[test]
    fn in_flight_states_are_not_editable_and_are_recovered_as_uncertain() {
        assert!(DraftState::Sending.is_in_flight());
        assert!(DraftState::Uncertain.is_in_flight());
        assert!(!DraftState::Editing.is_in_flight());
        assert!(
            !DraftState::Sent {
                remote_id: "m".to_owned()
            }
            .is_in_flight()
        );
        assert!(DraftState::Conflict.is_editable());
    }
}
