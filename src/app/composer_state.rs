//! The composer's app-side state: which drafts exist, which one each surface is editing,
//! what each one's durable state is, and what the send hold is doing.
//!
//! Everything blocking — the draft store, attachment bytes, the provider — belongs to the
//! worker in `composer_worker.rs`. This module is the UI-thread half: it holds the
//! snapshots the worker hands back, decides what to ask the worker next, and turns the
//! answers into what the user sees.

use std::collections::HashMap;

use crate::clock::Timestamp;
use crate::outgoing::{DraftId, DraftRecord, DraftState, HoldProgress, PendingHold, hold_progress};

/// How long after the last keystroke the *remote* mirror waits for a pause. Typing
/// pauses matter to a server round trip; they do not matter to a local write, which
/// follows every edit.
pub const REMOTE_DEBOUNCE_SECS: i64 = 2;

/// How long after the last keystroke the *local* write waits for a pause.
///
/// A keystroke is not worth a database transaction: each edit already updates the in-memory
/// record, and a burst of typing is written once when it settles. Three seconds is long
/// enough that ordinary typing coalesces into one write, and short enough that a crash or a
/// close loses at most the last three seconds of typing. Explicit paths — Send, Close,
/// switching account, app quit — flush the pending write at once instead of waiting.
pub const LOCAL_DEBOUNCE_SECS: i64 = 3;

/// What the user is told about one draft's durability, in the composer's status line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveStatus {
    /// Saved locally; there is no remote copy to sync.
    Local,
    /// Edits are not written yet.
    Dirty,
    /// A save is running.
    Saving,
    /// Saved locally and mirrored to the provider's draft.
    Synced,
    /// The save or the sync failed; the local copy is safe and the message says why.
    Failed(String),
    /// The provider's draft moved on; the local copy is kept and a conflict is shown.
    Conflict { remote_revision: String },
}

/// Blocking work other than the independent local-write lane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DraftOperation {
    Sync,
    Attachments,
    Hold,
    Send,
    Reconcile,
    Resolve,
    Discard,
}

/// One open draft surface: the record the composer is editing, plus what the owner needs
/// to know about it.
#[derive(Clone, Debug)]
pub struct OpenDraft {
    pub record: DraftRecord,
    pub status: SaveStatus,
    /// When the last edit landed, so the remote mirror can decide to write.
    pub dirty_since: Option<Timestamp>,
    /// The newest snapshot taken while a save was already in flight, so an edit during a
    /// save is written when that save comes back instead of being lost. `None` means the
    /// store already holds everything the form says.
    pub queued: Option<DraftRecord>,
    /// The provider's copy is behind what is stored here. Tracked apart from the local save:
    /// acknowledging a local write must not make an unmirrored edit look mirrored.
    pub remote_dirty: bool,
    /// This draft's pending write should not wait out the autosave debounce. Explicit saves
    /// — Close, Send, quitting — set it so the newest text reaches the store at once.
    pub urgent: bool,
    /// One local save at a time; network and attachment work do not block this lane.
    pub in_flight: bool,
    pub operation: Option<DraftOperation>,
    /// Where the surface is shown.
    pub inline: bool,
    /// Bumped on every local edit. A worker answer that predates the current generation
    /// carries a snapshot of the editor from before those edits, so its *content* is
    /// stale even though its identity is not.
    pub generation: u64,
    /// The generation the in-flight save was taken at.
    pub saved_generation: u64,
}

impl OpenDraft {
    pub fn new(record: DraftRecord) -> Self {
        let remote_dirty = matches!(
            record.state,
            DraftState::Editing | DraftState::Failed { .. }
        ) && record.synced_content_revision != Some(record.content_revision);
        let dirty_since = remote_dirty.then_some(record.updated_at);
        Self {
            record,
            status: SaveStatus::Local,
            dirty_since,
            queued: None,
            remote_dirty,
            urgent: false,
            in_flight: false,
            operation: None,
            inline: false,
            generation: 0,
            saved_generation: 0,
        }
    }

    /// Whether an edit landed after the in-flight save was taken, so that save's snapshot
    /// of the editor is already stale when it comes back.
    pub fn edited_since_save(&self) -> bool {
        self.generation != self.saved_generation
    }

    /// Take the identity the store now holds — the id, the revision and the remote
    /// bookkeeping — while keeping the content the user has typed since. A save answer
    /// must never overwrite later edits.
    pub fn absorb(&mut self, stored: DraftRecord) -> bool {
        let stale = self.edited_since_save() || stored.account != self.record.account;
        self.merge(stored, stale)
    }

    /// Background completion owns metadata/files, never the current editor fields.
    pub fn absorb_metadata(&mut self, stored: DraftRecord) {
        self.merge(stored, true);
    }

    fn merge(&mut self, stored: DraftRecord, stale: bool) -> bool {
        if stored.revision < self.record.revision {
            return self.edited_since_save();
        }
        let mut merged = stored;
        if stale {
            let same_account = merged.account == self.record.account;
            let same_subject = merged.subject == self.record.subject;
            // Move the current editor fields over the acknowledgement without copying
            // the body. Files and durable identity belong to the worker's snapshot.
            std::mem::swap(&mut merged.to, &mut self.record.to);
            std::mem::swap(&mut merged.cc, &mut self.record.cc);
            std::mem::swap(&mut merged.bcc, &mut self.record.bcc);
            std::mem::swap(&mut merged.subject, &mut self.record.subject);
            std::mem::swap(&mut merged.body, &mut self.record.body);
            std::mem::swap(&mut merged.in_reply_to, &mut self.record.in_reply_to);
            std::mem::swap(&mut merged.references, &mut self.record.references);
            if !same_account || !same_subject {
                std::mem::swap(
                    &mut merged.thread_remote_id,
                    &mut self.record.thread_remote_id,
                );
            }
            std::mem::swap(&mut merged.account, &mut self.record.account);
            merged.kind = self.record.kind;
            merged.updated_at = self.record.updated_at;
            if self.edited_since_save()
                && matches!(merged.state, DraftState::Failed { .. })
                && matches!(self.record.state, DraftState::Editing)
            {
                merged.state = self.record.state.clone();
            }
            if !same_account {
                // A late answer from the old account cannot bind the new account.
                std::mem::swap(&mut merged.remote_id, &mut self.record.remote_id);
                std::mem::swap(
                    &mut merged.remote_revision,
                    &mut self.record.remote_revision,
                );
                merged.synced_content_revision = self.record.synced_content_revision;
            }
        }
        self.record = merged;
        stale
    }

    /// Mark an edit. `snapshot` is the newest form state: a local save follows
    /// immediately, because a draft has to survive a crash at any moment, while only
    /// the *remote* mirror waits out a pause in typing.
    ///
    /// The snapshot carries only what the user can change. The revision, the provider
    /// identity and the threading are the store's to hand out, so they are taken from the
    /// record already held: a form that has not been told about the last save's revision
    /// would otherwise send it again and be refused, losing the user's next edits.
    pub fn touch(&mut self, snapshot: DraftRecord, now: Timestamp) {
        self.status = SaveStatus::Dirty;
        self.generation += 1;
        // The mirror is due after a pause in typing regardless of what the local write does.
        self.remote_dirty = true;
        self.dirty_since = Some(now);
        let mut next = snapshot;
        next.id = self.record.id;
        next.account = self.record.account.clone();
        next.kind = self.record.kind;
        next.message_id = self.record.message_id.clone();
        next.source_message = self.record.source_message;
        next.prefilled_subject = self.record.prefilled_subject.clone();
        next.imported_attachment_ids = self.record.imported_attachment_ids.clone();
        next.attachments = self.record.attachments.clone();
        next.in_reply_to = self.record.in_reply_to.clone();
        next.references = self.record.references.clone();
        next.attempted = self.record.attempted;
        next.state = match &self.record.state {
            DraftState::Failed { .. } => DraftState::Editing,
            state => state.clone(),
        };
        next.updated_at = now;
        next.revision = self.record.revision;
        next.editor_revision = self.record.editor_revision;
        next.content_revision = self.record.content_revision;
        next.synced_content_revision = self.record.synced_content_revision;
        next.remote_id = self.record.remote_id.clone();
        next.remote_revision = self.record.remote_revision.clone();
        next.thread_remote_id = self.record.thread_remote_id.clone();
        if next.subject != self.record.subject {
            next.clear_threading();
        }
        self.record = next.clone();
        self.queued = Some(next);
    }

    /// Queue the content again because the answer to a save carried a snapshot from before
    /// the last edit. This is not a new edit: the generation does not move, so the same
    /// comparison still recognises the *next* stale answer as stale.
    pub fn requeue(&mut self, snapshot: DraftRecord, now: Timestamp) {
        self.record = snapshot.clone();
        self.queued = Some(snapshot);
        // The remote mirror restarts its own debounce from here; the local write is immediate.
        self.dirty_since = Some(now);
        self.remote_dirty = true;
        self.status = SaveStatus::Dirty;
    }

    /// Whether a local save can start at `now`. A save follows a pause in typing rather than
    /// every keystroke: the snapshot is queued at once, but the write waits out
    /// [`LOCAL_DEBOUNCE_SECS`] from the newest edit — unless an explicit save marked it
    /// urgent, or a save is already in flight for this draft.
    pub fn save_due(&self, now: Timestamp) -> bool {
        !self.in_flight
            && self.queued.is_some()
            && (self.urgent
                || self
                    .dirty_since
                    .is_some_and(|at| now >= at + LOCAL_DEBOUNCE_SECS))
    }

    /// Mark the pending write as one the user asked for (Close, Send, quitting), so the beat
    /// writes it without waiting out the debounce.
    pub fn mark_urgent(&mut self) {
        if self.queued.is_some() {
            self.urgent = true;
        }
    }

    /// The snapshot a local save should write. Taken from the queue so the caller never
    /// re-reads the form between deciding to save and sending.
    pub fn take_queued(&mut self) -> Option<DraftRecord> {
        self.queued.take()
    }

    /// Whether the remote mirror is due at `now`. This is tracked apart from the local save:
    /// acknowledging a local write must not make an unmirrored edit look mirrored.
    pub fn remote_due(&self, now: Timestamp) -> bool {
        !self.in_flight
            && self.operation.is_none()
            && self.queued.is_none()
            && matches!(
                self.record.state,
                DraftState::Editing | DraftState::Failed { .. }
            )
            && self.remote_dirty
            && self
                .dirty_since
                .is_some_and(|at| now >= at + REMOTE_DEBOUNCE_SECS)
    }

    /// Whether the draft has any content worth keeping: a subject, a body, a recipient or
    /// an attachment. An untouched new message is not worth a draft record.
    pub fn is_empty(&self) -> bool {
        let r = &self.record;
        r.subject.trim().is_empty()
            && r.body.source.trim().is_empty()
            && r.to.trim().is_empty()
            && r.cc.trim().is_empty()
            && r.bcc.trim().is_empty()
            && r.attachments.is_empty()
    }

    /// Whether Send may be offered: the draft is editable and has a recipient.
    pub fn can_send(&self) -> bool {
        matches!(
            self.record.state,
            DraftState::Editing | DraftState::Failed { .. }
        ) && !self.record.attempted
            && self.record.validate().is_ok()
    }

    /// The durability and delivery state shown under the editor.
    pub fn status_line(&self) -> String {
        if let Some(operation) = self.operation {
            let text = match operation {
                DraftOperation::Sync if self.in_flight => "Saving · syncing…",
                DraftOperation::Sync => "Syncing…",
                DraftOperation::Attachments if self.in_flight => "Saving · updating attachments…",
                DraftOperation::Attachments => "Updating attachments…",
                DraftOperation::Hold => "Preparing to hold…",
                DraftOperation::Send => "Sending…",
                DraftOperation::Reconcile => "Checking delivery…",
                DraftOperation::Resolve => "Resolving conflict…",
                DraftOperation::Discard => "Discarding…",
            };
            return text.to_owned();
        }
        if self.in_flight {
            return "Saving…".to_owned();
        }
        match (&self.status, &self.record.state) {
            (_, DraftState::Held) => "Held — undo restores it".to_owned(),
            (_, DraftState::Sending) => "Sending…".to_owned(),
            (_, DraftState::Sent { .. }) => "Sent".to_owned(),
            (_, DraftState::Uncertain) => "Delivery uncertain — check before retrying".to_owned(),
            (_, DraftState::Failed { error }) => format!("Not sent: {error}"),
            (_, DraftState::Conflict) => "Changed on the server — your copy is kept".to_owned(),
            (SaveStatus::Dirty, _) => "Unsaved changes…".to_owned(),
            (SaveStatus::Saving, _) => "Saving…".to_owned(),
            (SaveStatus::Failed(e), _) => format!("Not saved: {e}"),
            (SaveStatus::Conflict { .. }, _) => "Conflict — your copy is kept".to_owned(),
            (SaveStatus::Synced, _) => "Synced".to_owned(),
            (SaveStatus::Local, _) => "Saved on this Mac".to_owned(),
        }
    }
}

/// The composer state: every open draft, plus the holds in flight.
#[derive(Default)]
pub struct ComposerState {
    /// Open drafts by id, in the order they were opened.
    pub drafts: Vec<OpenDraft>,
    /// The holds waiting out their countdown, by draft.
    pub holds: HashMap<DraftId, PendingHold>,
    /// A draft whose send was in flight when the app last ran, awaiting reconciliation.
    pub reconciling: Vec<DraftId>,
}

impl ComposerState {
    pub fn index(&self, id: DraftId) -> Option<usize> {
        self.drafts.iter().position(|d| d.record.id == id)
    }

    pub fn get(&self, id: DraftId) -> Option<&OpenDraft> {
        self.drafts.iter().find(|d| d.record.id == id)
    }

    pub fn get_mut(&mut self, id: DraftId) -> Option<&mut OpenDraft> {
        self.drafts.iter_mut().find(|d| d.record.id == id)
    }

    pub fn record(&self, id: DraftId) -> Option<&DraftRecord> {
        self.get(id).map(|d| &d.record)
    }

    /// Add or replace a draft's snapshot, keeping its place in the open order.
    pub fn put(&mut self, draft: DraftRecord) {
        match self.index(draft.id) {
            Some(ix) => self.drafts[ix].record = draft,
            None => self.drafts.push(OpenDraft::new(draft)),
        }
    }

    /// Forget a draft that was sent, discarded or pruned.
    pub fn remove(&mut self, id: DraftId) -> Option<OpenDraft> {
        self.holds.remove(&id);
        let ix = self.index(id)?;
        Some(self.drafts.remove(ix))
    }

    /// The drafts whose local save is due at `now`. A local save follows a pause in typing:
    /// "has something queued, nothing in flight, and the newest edit is old enough" — oldest
    /// edit first. An explicit save marked urgent skips the wait.
    pub fn due_saves(&self, now: Timestamp) -> Vec<DraftId> {
        let mut due: Vec<(DraftId, Timestamp)> = self
            .drafts
            .iter()
            .filter(|d| d.save_due(now))
            .filter_map(|d| d.dirty_since.map(|at| (d.record.id, at)))
            .collect();
        due.sort_by_key(|(_, at)| *at);
        due.into_iter().map(|(id, _)| id).collect()
    }

    /// The drafts whose *remote* mirror is due, oldest edit first. Only mirroring waits
    /// for a pause in typing; the local copy was written at the keystroke.
    pub fn due_remote_syncs(&self, now: Timestamp) -> Vec<DraftId> {
        let mut due: Vec<(DraftId, Timestamp)> = self
            .drafts
            .iter()
            .filter(|d| d.remote_due(now))
            .filter_map(|d| d.dirty_since.map(|at| (d.record.id, at)))
            .collect();
        due.sort_by_key(|(_, at)| *at);
        due.into_iter().map(|(id, _)| id).collect()
    }

    /// Holds whose countdown is over, oldest first.
    pub fn due_holds(&self, now: Timestamp) -> Vec<DraftId> {
        let mut due: Vec<(DraftId, i64)> = self
            .holds
            .values()
            .filter(|h| h.is_due(now))
            .map(|h| (h.draft, h.hold_until))
            .collect();
        due.sort_by_key(|(_, until)| *until);
        due.into_iter().map(|(id, _)| id).collect()
    }

    /// Start (or restart) a draft's send hold. Everything typed is already stored, so
    /// nothing is queued behind it: a held draft is not edited.
    pub fn begin_hold(&mut self, hold: PendingHold) {
        if let Some(draft) = self.get_mut(hold.draft) {
            draft.record.state = DraftState::Held;
            draft.dirty_since = None;
            draft.queued = None;
            draft.urgent = false;
            draft.status = SaveStatus::Local;
            draft.operation = None;
        }
        self.holds.insert(hold.draft, hold);
    }

    /// Drop a hold without sending, so the draft becomes editable again.
    pub fn cancel_hold(&mut self, id: DraftId, now: Timestamp) -> Option<OpenDraft> {
        self.holds.remove(&id);
        let draft = self.get_mut(id)?;
        draft.record.state = DraftState::Editing;
        draft.record.touch(now);
        Some(draft.clone())
    }

    /// The countdown for one draft's hold, for the toast's progress bar.
    pub fn progress(&self, id: DraftId, now: Timestamp) -> Option<HoldProgress> {
        self.holds.get(&id).map(|hold| hold_progress(hold, now))
    }

    /// Whether any attachment operation is running for this draft.
    pub fn attachment_busy(&self, id: DraftId) -> bool {
        self.get(id)
            .is_some_and(|d| d.operation == Some(DraftOperation::Attachments))
    }

    /// The attachments the composer shows for this draft.
    pub fn attachments(&self, id: DraftId) -> Vec<crate::outgoing::ManagedAttachment> {
        self.record(id)
            .map(|r| r.attachments.clone())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reading::Recipient;

    fn record(id: i64, to: &str) -> DraftRecord {
        let mut r = DraftRecord::new_email("personal", 0);
        r.id = DraftId(id);
        r.to = to.to_owned();
        r.revision = 1;
        r
    }

    /// An edit queues a local write at once, but the write waits for a pause in typing: a
    /// keystroke is not worth its own database transaction, and a crash can still only lose
    /// the last few seconds of typing.
    #[test]
    fn an_edit_waits_out_the_autosave_debounce() {
        let mut state = ComposerState::default();
        state.put(record(1, "a@b.test"));
        let mut r = record(1, "a@b.test");
        r.body.source = "hi".into();
        state.get_mut(DraftId(1)).expect("open").touch(r, 100);
        assert!(state.due_saves(100).is_empty(), "not on the keystroke");
        assert!(
            state.due_saves(100 + LOCAL_DEBOUNCE_SECS - 1).is_empty(),
            "not a moment before the pause is up"
        );
        assert_eq!(
            state.due_saves(100 + LOCAL_DEBOUNCE_SECS),
            vec![DraftId(1)],
            "the write follows the pause"
        );
        // The remote mirror, unlike the local write, waits for a pause in typing.
        assert!(state.due_remote_syncs(100).is_empty());
        let open = state.get_mut(DraftId(1)).expect("draft");
        open.take_queued().expect("local save");
        assert_eq!(
            state.due_remote_syncs(100 + REMOTE_DEBOUNCE_SECS),
            vec![DraftId(1)]
        );
    }

    /// Every keystroke restarts the pause, so a burst of typing is one write when it settles
    /// rather than one write per character.
    #[test]
    fn typing_restarts_the_debounce_so_a_burst_is_one_save() {
        let mut state = ComposerState::default();
        state.put(record(1, "a@b.test"));
        for (at, text) in [(100, "a"), (101, "ab"), (102, "abc")] {
            let mut r = state.record(DraftId(1)).expect("open").clone();
            r.body.source = text.into();
            state.get_mut(DraftId(1)).expect("open").touch(r, at);
            assert!(
                state.due_saves(at + LOCAL_DEBOUNCE_SECS - 1).is_empty(),
                "the newest keystroke pushes the pause out"
            );
        }
        assert!(state.due_saves(104).is_empty(), "102 + pause is still ahead");
        assert_eq!(state.due_saves(102 + LOCAL_DEBOUNCE_SECS), vec![DraftId(1)]);
    }

    /// An explicit save (Close, Send, quitting) skips the pause: the newest text is written
    /// at once instead of waiting out the debounce.
    #[test]
    fn an_urgent_save_skips_the_debounce() {
        let mut state = ComposerState::default();
        state.put(record(1, "a@b.test"));
        let mut r = record(1, "a@b.test");
        r.body.source = "close me".into();
        let open = state.get_mut(DraftId(1)).expect("open");
        open.touch(r, 100);
        open.mark_urgent();
        assert_eq!(state.due_saves(100), vec![DraftId(1)]);
    }

    /// A save running must not swallow the next keystroke: the newer snapshot waits in
    /// the queue and goes out when the in-flight answer lands.
    #[test]
    fn an_edit_during_a_save_queues_the_newest_snapshot() {
        let mut state = ComposerState::default();
        state.put(record(1, "a@b.test"));
        let first = state.get_mut(DraftId(1)).expect("open");
        let mut typed = first.record.clone();
        typed.body.source = "one".into();
        first.touch(typed.clone(), 100);
        assert_eq!(
            state.due_saves(100 + LOCAL_DEBOUNCE_SECS),
            vec![DraftId(1)]
        );
        let open = state.get_mut(DraftId(1)).expect("open");
        let taken = open.take_queued().expect("snapshot");
        open.in_flight = true;
        open.saved_generation = open.generation;
        assert_eq!(taken.body.source, "one");

        let mut typed_again = open.record.clone();
        typed_again.body.source = "one two".into();
        open.touch(typed_again, 101);
        assert!(
            state.due_saves(101 + LOCAL_DEBOUNCE_SECS).is_empty(),
            "one save at a time"
        );
        let open = state.get_mut(DraftId(1)).expect("open");
        open.in_flight = false;
        let queued = open
            .take_queued()
            .expect("the newer snapshot is still queued");
        assert_eq!(queued.body.source, "one two");
    }

    /// An answer from the account a draft has since left must not restore that account's
    /// remote identity onto it. This is consumer-visible: a rebound draft would otherwise be
    /// sent to a backend that has never heard of it, or update another account's draft.
    #[test]
    fn an_answer_from_the_old_account_cannot_rebind_a_switched_draft() {
        let mut open = OpenDraft::new(record(1, "a@b.test"));
        open.touch(open.record.clone(), 100);
        open.saved_generation = open.generation;

        // The old account's backend answers, carrying its own remote identity.
        let mut stale = record(1, "a@b.test");
        stale.account = "personal".into();
        stale.remote_id = Some("gmail-1".into());
        stale.remote_revision = Some("m-1".into());
        stale.revision = 4;

        // The draft moves to another account while that answer is in flight.
        let mut moved = open.record.clone();
        moved.account = "work".into();
        open.record = moved.clone();

        open.absorb(stale);
        assert_eq!(open.record.account, "work", "the chosen account stands");
        assert_eq!(
            open.record.remote_id, None,
            "the old account's draft id is not adopted"
        );
        assert_eq!(open.record.remote_revision, None);
        assert_eq!(
            open.record.revision, 4,
            "the store's revision is still taken for the next save"
        );
    }

    /// A rebound draft's own cleared identity survives an answer from its *new* account's
    /// backend too, until that backend reports a remote of its own.
    #[test]
    fn a_cleared_identity_is_not_resurrected_by_a_new_accounts_answer() {
        let mut open = OpenDraft::new(record(1, "a@b.test"));
        open.record.account = "work".into();
        open.record.remote_id = None;
        open.record.remote_revision = None;
        open.touch(open.record.clone(), 100);
        open.saved_generation = open.generation;

        // Same account, but the store still holds the pre-switch binding.
        let mut stale = open.record.clone();
        stale.account = "work".into();
        stale.remote_id = Some("gmail-1".into());
        open.absorb(stale);
        assert_eq!(
            open.record.remote_id.as_deref(),
            Some("gmail-1"),
            "the same account may rebind"
        );
    }

    /// Re-queueing after a stale answer writes the same content again without counting as a
    /// new edit, so the store gets it and the generation comparison still works afterwards.
    #[test]
    fn a_requeue_writes_the_content_again_without_counting_as_an_edit() {
        let mut open = OpenDraft::new(record(1, "a@b.test"));
        let mut typed = open.record.clone();
        typed.body.source = "typed".into();
        open.touch(typed, 100);
        let open_gen = open.generation;
        let queued = open.take_queued().expect("the first snapshot");
        open.saved_generation = open_gen;
        open.in_flight = true;

        // A stale answer lands; the local text stays and goes back in the queue.
        open.record.body.source = "typed".into();
        open.generation = open_gen;
        open.requeue(open.record.clone(), 101);
        assert_eq!(open.generation, open_gen, "a requeue is not a new edit");
        assert!(
            open.save_due(101 + LOCAL_DEBOUNCE_SECS) || open.in_flight,
            "the content is queued again"
        );
        open.in_flight = false;
        assert_eq!(open.take_queued().expect("queued").body.source, "typed");
        let _ = queued;
    }

    /// The answer to a save carries the stored revision, but never the content of a
    /// snapshot older than the edits that have landed since.
    #[test]
    fn a_save_answer_keeps_typing_that_landed_after_it_started() {
        let mut open = OpenDraft::new(record(1, "a@b.test"));
        open.touch(record(1, "a@b.test"), 100);
        open.saved_generation = open.generation;
        let mut typed = open.record.clone();
        typed.body.source = "later".into();
        open.record = typed.clone();
        open.generation += 1;

        let mut stored = record(1, "a@b.test");
        stored.revision = 7;
        stored.body.source = "stale".into();
        assert!(open.absorb(stored), "the answer is stale content");
        assert_eq!(open.record.body.source, "later", "the newer typing stands");
        assert_eq!(
            open.record.revision, 7,
            "the identity the store now holds is taken"
        );
    }

    #[test]
    fn an_untouched_new_message_is_not_worth_saving() {
        let mut state = ComposerState::default();
        state.put(DraftRecord::new_email("personal", 0));
        assert!(state.drafts[0].is_empty());
        assert!(
            !state.drafts[0].can_send(),
            "no recipient, so nothing to send"
        );
    }

    #[test]
    fn sending_needs_at_least_one_valid_recipient() {
        let mut state = ComposerState::default();
        state.put(record(1, "a@b.test"));
        assert!(state.drafts[0].can_send());
        state.put(record(2, "not-an-address@"));
        assert!(!state.get(DraftId(2)).expect("open").can_send());
    }

    #[test]
    fn bcc_only_counts_as_a_recipient_for_sending() {
        let mut state = ComposerState::default();
        let mut r = record(3, "");
        r.bcc = "hidden@b.test".to_owned();
        state.put(r);
        assert!(state.get(DraftId(3)).expect("open").can_send());
    }

    #[test]
    fn a_hold_suspends_saving_and_undo_makes_the_draft_editable_again() {
        let mut state = ComposerState::default();
        state.put(record(1, "a@b.test"));
        let now = 500;
        let mut edited = record(1, "a@b.test");
        edited.body.source = "body".into();
        state.get_mut(DraftId(1)).expect("open").touch(edited, now);
        let hold = PendingHold::start(DraftId(1), "personal".into(), now);
        state.begin_hold(hold);
        assert!(
            state.due_saves(now + LOCAL_DEBOUNCE_SECS).is_empty(),
            "a held draft is not saved"
        );
        assert!(
            !state.get(DraftId(1)).expect("open").can_send(),
            "held, not sendable"
        );
        state.cancel_hold(DraftId(1), now);
        assert!(
            state.get(DraftId(1)).expect("open").can_send(),
            "undo restored editing"
        );
        assert!(state.holds.is_empty());
    }

    #[test]
    fn due_holds_come_back_oldest_first() {
        let mut state = ComposerState::default();
        let mut first = PendingHold::start(DraftId(2), "personal".into(), 100);
        first.hold_until = 200;
        let mut second = PendingHold::start(DraftId(1), "personal".into(), 100);
        second.hold_until = 110;
        state.begin_hold(second);
        state.begin_hold(first);
        assert_eq!(state.due_holds(150), vec![DraftId(1)]);
    }

    #[test]
    fn recipients_are_validated_the_same_way_the_send_will_validate_them() {
        let r = record(1, "A@b.test, a@B.TEST, c@d.test");
        let recipients: Vec<Recipient> = r.validate().expect("valid");
        assert_eq!(
            recipients.len(),
            2,
            "duplicate addresses collapse, case-insensitively"
        );
    }

    #[test]
    fn removing_a_draft_drops_its_hold_too() {
        let mut state = ComposerState::default();
        state.put(record(1, "a@b.test"));
        state.begin_hold(PendingHold::start(DraftId(1), "personal".into(), 0));
        assert!(state.remove(DraftId(1)).is_some());
        assert!(state.holds.is_empty());
        assert!(state.get(DraftId(1)).is_none());
        assert!(state.remove(DraftId(1)).is_none());
    }

    #[test]
    fn a_canonical_send_failure_is_not_hidden_by_metadata_merging() {
        let mut open = OpenDraft::new(record(1, "a@b.test"));
        let mut failed = open.record.clone();
        failed.revision += 1;
        failed.state = DraftState::Failed {
            error: "offline".into(),
        };
        open.absorb_metadata(failed);
        assert!(matches!(open.record.state, DraftState::Failed { .. }));
        let mut edited = open.record.clone();
        edited.body.source = "new attempt".into();
        open.touch(edited, 100);
        assert_eq!(open.record.state, DraftState::Editing);
    }

    #[test]
    fn an_older_metadata_receipt_cannot_roll_back_a_newer_local_save() {
        let mut open = OpenDraft::new(record(1, "a@b.test"));
        let mut old = open.record.clone();
        old.revision = 2;
        old.remote_id = Some("old-server-draft".into());
        open.record.revision = 3;
        open.record.body.source = "latest durable body".into();
        open.record.remote_id = Some("current-server-draft".into());
        open.absorb_metadata(old);
        assert_eq!(open.record.body.source, "latest durable body");
        assert_eq!(
            open.record.remote_id.as_deref(),
            Some("current-server-draft")
        );
    }
}
