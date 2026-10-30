//! The composer's local persistence lane: the short SQLite work, on a thread of its own.
//!
//! Typing must never wait for the network, and the network must never wait for typing. So
//! the work that is *only* the draft table — creating a draft, writing the user's edits,
//! listing what is stored, recovering after a restart, taking a send hold on and off it —
//! lives here, and shares the [`DraftStore`](crate::outgoing::store::DraftStore) with the
//! composer's other worker. That store hands out one short transaction at a time, so the
//! two lanes interleave without either holding a lock across a provider call or a file
//! copy.
//!
//! Everything here is a single small database operation. Nothing in this file opens a
//! provider connection, reads a file or retries anything: a provider call that blocks for
//! thirty seconds cannot delay the draft write for a keystroke, and a draft write cannot
//! delay it.
//!
//! Both lanes answer on the same event channel, so a `Saved` that landed while a sync was
//! still out is already queued by the time the UI drains. The UI reads that queue on its own
//! beat: a GPUI task is woken on the executor it was spawned on, so no lane here wakes the
//! UI directly.

use std::sync::mpsc::{Receiver, Sender, channel};

use crate::clock::Timestamp;
use crate::outgoing::draft::{DraftId, DraftRecord, DraftState};
use crate::outgoing::hold::PendingHold;
use crate::outgoing::store::{DraftError, DraftStore};

use super::composer_worker::{Command, Event};

/// Which lane a command belongs to.
///
/// Kept as one function so the routing is decided once, in the place that dispatches,
/// rather than being re-derived by anything that sends a command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lane {
    /// Pure local persistence: a short SQLite transaction and nothing else.
    Local,
    /// Provider and filesystem work, serialised on the composer's existing worker.
    Work,
}

/// The lane `command` belongs to.
///
/// Everything that touches nothing but the draft table goes to the local lane: it is a
/// small transaction that must not queue behind a provider call or a file copy. Everything
/// that talks to a provider or moves bytes stays on the worker, which is serial on purpose:
/// two drafts must never mutate the same attachments at once.
pub fn lane_of(command: &Command) -> Lane {
    match command {
        Command::Create { .. }
        | Command::Save { .. }
        | Command::List
        | Command::Recover { .. }
        | Command::BeginHold { .. }
        | Command::CancelHold(_) => Lane::Local,
        Command::Discard(_)
        | Command::Attach { .. }
        | Command::ForwardAttachments { .. }
        | Command::Detach { .. }
        | Command::Sync { .. }
        | Command::Resolve { .. }
        | Command::Import { .. }
        | Command::ImportOne { .. }
        | Command::Send { .. }
        | Command::Reconcile { .. }
        | Command::Probe { .. }
        | Command::Threading { .. } => Lane::Work,
    }
}

/// Where a finished job's events go. One of these is shared by both lanes, so every answer
/// reaches the same queue the UI drains.
pub struct Signals {
    pub events: Sender<Event>,
}

impl Signals {
    /// The channel both lanes share.
    pub fn new() -> (Self, Receiver<Event>) {
        let (event_tx, event_rx) = channel::<Event>();
        (Self { events: event_tx }, event_rx)
    }

    /// Queue a job's events. An event queue a UI reads on its own beat is enough: the
    /// worker runs on its own threads, and a GPUI task may only be woken on the executor it
    /// was spawned on, so nothing here reaches back into the UI.
    pub fn publish(&self, events: Vec<Event>) {
        for event in events {
            // The UI is gone only when the app is shutting down.
            if self.events.send(event).is_err() {
                return;
            }
        }
    }
}

impl Clone for Signals {
    /// Both lanes queue onto the same events, so a clone is the same signal, not a second
    /// one.
    fn clone(&self) -> Self {
        Self {
            events: self.events.clone(),
        }
    }
}

/// The local persistence lane: a clone of the draft store, and the answers it gives.
pub struct LocalLane {
    drafts: DraftStore,
    signals: Signals,
}

impl LocalLane {
    pub fn new(drafts: DraftStore, signals: Signals) -> Self {
        Self { drafts, signals }
    }

    /// Answer local commands until the channel closes.
    pub fn run(mut self, commands: Receiver<Command>) {
        for command in commands {
            if matches!(lane_of(&command), Lane::Work) {
                // The dispatcher routes by `lane_of`; a work command reaching this thread
                // is a routing mistake, and saying so beats silently doing nothing.
                self.signals.publish(vec![Event::JobFailed(
                    "a provider command reached the local lane".into(),
                )]);
                continue;
            }
            let events = self.handle(command);
            self.signals.publish(events);
        }
    }

    /// One local command, answered exactly once.
    pub fn handle(&mut self, command: Command) -> Vec<Event> {
        match command {
            Command::Create { draft } => vec![self.create(*draft)],
            Command::Save { draft, expected } => vec![self.save(*draft, expected)],
            Command::List => vec![Event::Listed(self.list())],
            Command::Recover { now } => vec![self.recover(now)],
            Command::BeginHold { draft, now, at } => vec![self.begin_hold(draft, now, at)],
            Command::CancelHold(id) => vec![self.cancel_hold(id)],
            _ => vec![Event::JobFailed(
                "that command is not local persistence".into(),
            )],
        }
    }

    // ------------------------------------------------------------------ the table

    fn create(&mut self, mut draft: DraftRecord) -> Event {
        match self.drafts.create(&mut draft) {
            Ok(stored) => Event::Created(Box::new(stored)),
            Err(e) => self.store_failed(None, e),
        }
    }

    /// Write the user's edits to the row as it stands.
    ///
    /// `expected` is the *editor* revision the surface built the snapshot from, not the
    /// total row revision: a file that finished copying or a draft that was just mirrored
    /// moved the row on without touching anything the user typed, and refusing their next
    /// keystroke over that would lose their work over something they never saw. A
    /// genuinely stale token — another surface's newer content — is still refused, with the
    /// stored copy handed back so the user keeps what they wrote.
    fn save(&mut self, draft: DraftRecord, expected: u64) -> Event {
        let id = draft.id;
        match self.drafts.save_local(draft, expected) {
            Ok(stored) => Event::Saved(Box::new(stored)),
            Err(DraftError::StaleRevision { .. }) => match self.drafts.get(id) {
                Ok(Some(stored)) => Event::SaveRejected(Box::new(stored)),
                Ok(None) => Event::Missing(id),
                Err(e) => self.store_failed(Some(id), e),
            },
            // A draft that is held, sending, in doubt or already sent is not editable, and
            // its own error says which — nothing is lost, the user has to wait or undo
            // first. Every other failure is a write that could not be kept.
            Err(e) => self.store_failed(Some(id), e),
        }
    }

    fn list(&mut self) -> Vec<DraftRecord> {
        self.drafts.list().unwrap_or_default()
    }

    fn recover(&mut self, now: Timestamp) -> Event {
        match self.drafts.recover(now) {
            Ok(recovery) => {
                // Every draft comes back, whatever state it was left in: nothing is sent
                // and nothing is hidden from the drafts list.
                Event::Recovered(Box::new(recovery), self.drafts.list().unwrap_or_default())
            }
            Err(e) => self.store_failed(None, e),
        }
    }

    /// Put a draft into its send hold, and take it back out again.
    ///
    /// The hold row and the state go in one breath, so a crash between them cannot leave a
    /// draft that both waits out a hold and looks editable. Both writes are applied to the
    /// row as it is *now*: a file copy that finished while the user pressed send adds its
    /// attachment without making the hold fail.
    fn begin_hold(&mut self, id: DraftId, now: Timestamp, at: Option<Timestamp>) -> Event {
        let draft = match self.drafts.get(id) {
            Ok(Some(draft)) => draft,
            Ok(None) => return Event::Missing(id),
            Err(e) => return self.store_failed(Some(id), e),
        };
        // A draft whose send has already been decided does not wait out a countdown, and
        // it is not put back into an editable-looking state either: what it needs is the
        // state it is in, kept.
        if let Some(reason) = hold_refusal(&draft) {
            return Event::StoreFailed(Some(id), reason);
        }
        let account = draft.account;
        let hold = match at {
            Some(at) => PendingHold::scheduled(id, account, now, at),
            None => PendingHold::start(id, account, now),
        };
        if let Err(e) = self.hold_state(id, DraftState::Held, now) {
            return self.store_failed(Some(id), e);
        }
        match self.drafts.set_pending(&hold) {
            Ok(()) => Event::Held(hold),
            Err(e) => {
                // The hold could not be persisted, so there is nothing to count down to.
                // Put the draft back rather than leave it looking frozen forever.
                if let Err(restore) = self.hold_state(id, DraftState::Editing, now) {
                    return Event::StoreFailed(
                        Some(id),
                        format!("{e}; restoring the editable draft also failed: {restore}"),
                    );
                }
                self.store_failed(Some(id), e)
            }
        }
    }

    fn cancel_hold(&mut self, id: DraftId) -> Event {
        match self.drafts.get(id) {
            Ok(Some(draft)) => {
                // The hold goes in the same breath as the state, so a crash between them
                // cannot leave a draft that both waits out a hold and looks editable.
                let _ = self.drafts.clear_pending(id);
                // Only a draft that really is held becomes editable again. One whose send is
                // in flight, in doubt or already delivered keeps that state: making it
                // editable would drop the attempt a restart reconciles and let the same
                // message be sent a second time.
                if !matches!(draft.state, DraftState::Held) {
                    return Event::HoldCancelled(Box::new(draft));
                }
                match self.hold_state(id, DraftState::Editing, draft.updated_at) {
                    Ok(stored) => Event::HoldCancelled(Box::new(stored)),
                    Err(e) => self.store_failed(Some(id), e),
                }
            }
            Ok(None) => Event::Missing(id),
            Err(e) => self.store_failed(Some(id), e),
        }
    }

    /// Move a draft in or out of its hold on the current row. A no-op state change writes
    /// nothing, so re-holding an already held draft is not a new revision.
    fn hold_state(
        &self,
        id: DraftId,
        state: DraftState,
        now: Timestamp,
    ) -> Result<DraftRecord, DraftError> {
        self.drafts.update_current(id, |row| {
            row.state = state;
            row.touch(now);
            Ok(())
        })
    }

    fn store_failed(&self, draft: Option<DraftId>, error: DraftError) -> Event {
        Event::StoreFailed(draft, error.to_string())
    }
}

/// Why a draft must not be put on a send hold, when its send has already been decided.
///
/// A hold counts towards a send request that has not been made. For a draft already in
/// flight, in doubt or delivered there is nothing left to count towards, and making it look
/// held again would hide the very state that stops it being sent twice.
fn hold_refusal(draft: &DraftRecord) -> Option<String> {
    let reason = match draft.state {
        DraftState::Sending => "this draft is already being sent",
        DraftState::Uncertain => {
            "this draft may already have been delivered, so it is being checked rather than sent again"
        }
        DraftState::Sent { .. } => "this draft has already been sent",
        _ => return None,
    };
    Some(reason.to_owned())
}
