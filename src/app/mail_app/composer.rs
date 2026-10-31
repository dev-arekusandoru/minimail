//! Composer orchestration on the root view: opening a draft (inline under a thread, in a
//! tab, or as a new message), autosaving it, holding a send, undoing that hold, and
//! folding a confirmed send back into the mailbox.
//!
//! The rules this file exists to enforce:
//!
//! * a draft is durable *before* anything is sent, and closing a surface never deletes it —
//!   only [`MailApp::discard_draft`] does;
//! * a save answer merges the store's identity into what the user has typed and never
//!   overwrites it, and an edit that lands while a save is running is written next;
//! * a send waits out a ten-second hold with a countdown and an Undo, and Undo restores the
//!   whole draft, attachments included;
//! * only a provider confirmation puts a message in the mailbox, and a confirmed send never
//!   becomes a second copy when the next sync sees the same message from the server;
//! * an uncertain send is reconciled, never retried blind.
//!
//! ## What runs where
//!
//! The retained 50ms heartbeat drains both lanes, starts due work, flushes holds, and
//! applies queued window work. Rendering only constructs elements; it never starts IO.

use std::path::PathBuf;

use gpui_kit::component::notification::Notification;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::app::compose::{AccountChoice, ComposeEvent, Composer, Focus, Placement};
use crate::app::composer_state::{DraftOperation, SaveStatus};
use crate::app::composer_worker::{
    Accounts, Client as ComposerWorker, Command, ComposeAccount, Event, ImportFailure, ResolveMode,
    Stores,
};
use crate::judge::Judge as _;
use crate::model::{Message, MessageId, ProviderKind, TriageState};
use crate::outgoing::{AttachmentId, DraftId, DraftKind, DraftRecord, DraftState, SendIntent};
use crate::provider::RemoteId;
use crate::provider::compose::{RemoteDraft, SentMail, Threading};
use crate::tabs;

use super::modals::DialogSpec;
use super::*;
use crate::model::Attachment;

/// Identity of the "restored a draft" toast.
struct RecoveryToastId;

/// How many provider drafts one import round asks for.
const IMPORT_LIMIT: usize = 25;

/// What a draft still needs once the worker has given it a durable identity.
///
/// A draft the user just started does not have an id yet, so the things that belong to
/// *that* draft — the original's files for a forward, the original's RFC threading for a
/// reply — are remembered here and applied when the id arrives.
#[derive(Clone, Default)]
pub(crate) struct PendingOpen {
    pub placement: Placement,
    pub focus: Focus,
    /// Carry the original message's files into this forward.
    pub forward_from: Option<MessageId>,
    /// Ask for the original's RFC threading headers, so the reply is threaded.
    pub thread_from: Option<RemoteId>,
}

/// Serialized attachment mutations; local body saves use an independent lane.
pub(super) enum AttachmentOp {
    Attach(Vec<PathBuf>),
    Detach(AttachmentId),
    Forward { account: String, message: RemoteId },
}
/// Whether closing a draft tab (its x, or `cmd-w`) asks Send / Save & Close / Discard first.
/// One switch, so the behaviour is a single line to change.
pub(super) const CONFIRM_DRAFT_TAB_CLOSE: bool = true;
/// Work a decision has taken that can only be carried out where there is a window.
pub(super) enum PendingUi {
    /// Build the composer over a durable record and put it where it was asked for.
    Build(Box<DraftRecord>, PendingOpen),
    /// Move a tab draft back under its thread and put the caret in the body.
    Collapse(DraftId),
    /// Put the caret back in a draft, after an undo restored it.
    FocusBody(DraftId),
    RestoreDraft(DraftId),
    /// Ask before deleting a draft: the one action that cannot be undone.
    ConfirmDiscard(DraftId),
    /// Say what a startup recovery brought back.
    Recovered(DraftId),
    /// Offer the filing choice for a confirmed reply's original.
    PostSend(MessageId),
    /// Give the keyboard back to the app once the window work above has run. A draft that
    /// vanishes takes its inputs with it, and whatever focused them must not be left holding
    /// keys that no longer reach anything.
    FocusApp,
}

impl MailApp {
    // ------------------------------------------------------------------ lifecycle

    /// Start the composer worker and recover whatever the last run left behind: interrupted
    /// holds become editable drafts, interrupted sends become uncertain ones, and nothing
    /// is sent on startup.
    pub fn attach_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // The drafts live beside the mail cache, under one directory the worker owns.
        let root = crate::outgoing::DraftStore::default_path()
            .parent()
            .map(|parent| parent.join("drafts"))
            .unwrap_or_else(crate::outgoing::AttachmentStore::default_root);
        match Stores::open(&root) {
            Ok(stores) => {
                self.composer_worker = Some(ComposerWorker::start(stores, self.provider_handles()));
                self.spawn_composer_beat(window, cx);
                self.watch_window_close(window, cx);
                self.composer_recover(cx);
            }
            Err(e) => self.show_toast(format!("Drafts are unavailable: {e}"), window, cx),
        }
    }

    /// Start the composer over stores that keep nothing, for a headless test: the same
    /// worker and the same code path, with no draft left on disk afterwards.
    pub fn attach_composer_in_memory(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match Stores::open_in_memory() {
            Ok(stores) => {
                self.composer_worker = Some(ComposerWorker::start(stores, self.provider_handles()));
                self.spawn_composer_beat(window, cx);
                self.watch_window_close(window, cx);
                self.composer_recover(cx);
            }
            Err(e) => self.show_toast(format!("Drafts are unavailable: {e}"), window, cx),
        }
    }

    /// The accounts the composer can compose through, keyed by account id.
    ///
    /// A mock account gets a real in-memory backend inside the worker rather than a
    /// provider handle it does not have, so the mock path exercises the same create, save,
    /// send and reconcile code a Gmail account does instead of pretending delivery. Every
    /// account is registered with the address it sends *from*, which is its email and never
    /// its local id.
    fn provider_handles(&self) -> Accounts {
        let mut accounts = Accounts::new();
        for account in self.mailbox.accounts() {
            let composed = match account.provider {
                crate::model::ProviderKind::Mock => {
                    ComposeAccount::mock(self.now(), account.email.clone())
                }
                _ => match self.providers.get(&account.id) {
                    Some(handle) => ComposeAccount::remote(handle.clone(), account.email.clone()),
                    // A signed-out account composes locally until it is linked again; the
                    // draft is never lost, and the send reports why it could not go.
                    None => continue,
                },
            };
            accounts.insert(account.id.clone(), composed);
        }
        accounts
    }

    /// Tell the worker which accounts it may compose through, now.
    ///
    /// One replacement rather than a register-then-forget loop: an account that was signed
    /// out is no longer in the map either, so a loop that only forgot the handles it still
    /// held would leave a stale backend able to answer a send. The worker also keeps an
    /// account's existing backend when it is unchanged, so a refresh does not restart an
    /// in-memory mock server and lose the drafts and sent history a reconciliation needs.
    pub(super) fn refresh_composer_accounts(&mut self, cx: &mut Context<Self>) {
        let accounts = self.provider_handles();
        if let Some(worker) = self.composer_worker.as_ref() {
            worker.update_accounts(accounts);
        }
        self.push_accounts_to_composers(cx);
    }

    /// Give every open composer the accounts that exist now, so its selector offers the same
    /// list the app holds. A draft opened before a sign-in would otherwise keep offering only
    /// the accounts it was built with.
    pub(super) fn push_accounts_to_composers(&mut self, cx: &mut Context<Self>) {
        let accounts = self.compose_accounts();
        for entity in self.composers.values().cloned().collect::<Vec<_>>() {
            entity.update(cx, |composer, cx| {
                composer.set_accounts(accounts.clone(), cx)
            });
        }
    }

    /// Save every draft that still has a pending write when the main window closes, so
    /// quitting does not drop the last few seconds of typing. The window is gone by the time
    /// this runs, so the writes go to the local lane's thread; it is the hook GPUI offers
    /// here. Only the main window counts: a Settings window closing must not touch drafts.
    fn watch_window_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let main = window.window_handle().window_id();
        let app = cx.weak_entity();
        let sub = cx.on_window_closed(move |cx, closed| {
            if closed != main {
                return;
            }
            app.update(cx, |this, cx| this.flush_dirty_drafts(cx))
                .ok();
        });
        self._composer_subs.push(sub);
    }

    /// Mark every draft with a queued snapshot urgent and start its write, skipping the
    /// autosave debounce. The window-close hook calls this; tests call it directly.
    pub fn flush_dirty_drafts(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<DraftId> = self
            .composer
            .drafts
            .iter()
            .filter(|d| d.has_pending_write())
            .map(|d| d.record.id)
            .collect();
        for id in ids {
            if let Some(open) = self.composer.get_mut(id) {
                open.mark_urgent();
            }
            self.write_draft(id);
            self.sync_composer_status(id, cx);
        }
    }

    /// Ask the worker to recover interrupted work. Nothing is sent; drafts come back
    /// editable and the user is told which ones were in flight.
    fn composer_recover(&mut self, cx: &mut Context<Self>) {
        if let Some(worker) = self.composer_worker.as_ref() {
            worker.send(Command::Recover { now: self.now() });
        }
        // The provider's own drafts come in at startup too: a draft written on another
        // device is here without the user having to ask for it.
        self.import_all_account_drafts(cx);
        cx.notify();
    }

    /// Pull each linked account's remote drafts. Called at startup, after a sign-in and on
    /// the sync cadence, so an edit made elsewhere shows up as a conflict rather than being
    /// missed.
    pub(super) fn import_all_account_drafts(&mut self, cx: &mut Context<Self>) {
        let accounts: Vec<String> = self
            .mailbox
            .accounts()
            .iter()
            .filter(|a| a.provider != ProviderKind::Mock)
            .map(|a| a.id.clone())
            .collect();
        for account in accounts {
            self.import_provider_drafts(&account, cx);
        }
    }

    /// One beat of composer work: take what the worker finished, start the local saves and
    /// sends that are due, then carry out the window work those decisions queued.
    ///
    /// This runs from the retained heartbeat timer, which holds the real window, so *every*
    /// part of the beat happens here — a render pass neither mutates state nor starts work.
    pub(super) fn composer_beat(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let mut did = false;
        if let Some(worker) = self.composer_worker.as_ref() {
            let events: Vec<Event> = worker.drain();
            did |= !events.is_empty();
            for event in events {
                self.apply_composer_event(event, Some(&mut *window), cx);
            }
            let now = self.now();
            // A hold that ran out sends; nothing else starts a send.
            for id in self.composer.due_holds(now) {
                self.flush_hold(id);
                did = true;
            }
            if self.composer.holds.is_empty() {
                self.hold_painted_at = None;
            } else if self.hold_painted_at != Some(now) {
                self.hold_painted_at = Some(now);
                // The pill counts down in whole seconds, so it repaints once per second.
                let counting: Vec<DraftId> = self
                    .composer
                    .holds
                    .values()
                    .filter(|hold| !hold.is_scheduled())
                    .map(|hold| hold.draft)
                    .collect();
                for id in counting {
                    if let Some(entity) = self.composers.get(&id) {
                        entity.update(cx, |_, cx| cx.notify());
                    }
                    did = true;
                }
            }
            // A local write follows a pause in typing, so a burst of keystrokes is one save
            // rather than one transaction per character.
            for id in self.composer.due_saves(now) {
                self.write_draft(id);
                self.sync_composer_status(id, cx);
                did = true;
            }
            // Only the remote mirror waits for a pause in typing.
            for id in self.composer.due_remote_syncs(now) {
                self.sync_draft(id);
                self.sync_composer_status(id, cx);
                did = true;
            }
            let attachments: Vec<_> = self
                .pending_attachment_ops
                .keys()
                .copied()
                .filter(|id| {
                    self.composer
                        .get(*id)
                        .is_some_and(|open| open.operation.is_none())
                })
                .collect();
            for id in attachments {
                self.flush_attachment_op(id);
                self.sync_composer_status(id, cx);
                did = true;
            }
            let resolving: Vec<_> = self.pending_resolutions.keys().copied().collect();
            for id in resolving {
                self.flush_resolution(id);
                self.sync_composer_status(id, cx);
            }
            // A send asked for mid-save holds as soon as that save is stored.
            let waiting: Vec<DraftId> = self.hold_after_save.iter().copied().collect();
            for id in waiting {
                self.hold_when_persisted(id, cx);
            }
        }
        let queued = !self.pending_ui.is_empty();
        self.settle_composer_ui(window, cx);
        // A beat that found nothing does not ask for a repaint, so an idle app costs a
        // channel drain and nothing else.
        let changed = did | queued;
        if changed {
            cx.notify();
        }
        changed
    }

    /// Carry out the window work a decision queued. Called from the heartbeat timer.
    pub(super) fn settle_composer_ui(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        while let Some(work) = self.pending_ui.pop_front() {
            match work {
                PendingUi::Build(draft, open) => {
                    let id = draft.id;
                    let forward_from = open.forward_from;
                    let thread_from = open.thread_from.clone();
                    self.build_composer(*draft, open, window, cx);
                    // What the draft still needs is read before it is handed over.
                    if let Some(message) = forward_from {
                        self.forward_attachments(id, message, cx);
                    }
                    if let Some(message) = thread_from {
                        self.request_threading(id, &message);
                    }
                    cx.notify();
                }
                PendingUi::Collapse(id) => self.collapse_composer(id, window, cx),
                PendingUi::FocusBody(id) => self.focus_draft_body(id, window, cx),
                PendingUi::RestoreDraft(id) => {
                    if self.inline_draft() != Some(id) {
                        self.reopen_draft(id, window, cx);
                    }
                    self.focus_draft_body(id, window, cx);
                }
                PendingUi::ConfirmDiscard(id) => self.confirm_discard(id, window, cx),
                PendingUi::Recovered(id) => self.show_recovered(id, window, cx),
                PendingUi::PostSend(original) => self.open_post_send_dialog(original, window, cx),
                // The filing dialog a reply leaves behind owns the keyboard; the handback
                // only applies when nothing took it.
                PendingUi::FocusApp => {
                    if !self.modal_open() {
                        window.focus(&self.focus_handle, cx);
                    }
                }
            }
        }
    }

    /// A message the user should see, raised where there is no window to push a
    /// notification to. The next frame shows it, which is how every other background
    /// message in the app reads.
    fn say(&mut self, text: String, _cx: &mut Context<Self>) {
        self.pending_toast = Some((text, None));
    }

    /// Fold one worker answer into the app's state. `window` is present when the answer
    /// arrived from an action that had one; a heartbeat has none and queues the window work.
    fn apply_composer_event(
        &mut self,
        event: Event,
        mut window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        let changed_draft = event.draft_id();
        match event {
            Event::Created(draft) => {
                let open = self.pending_composers.pop_front().unwrap_or_default();
                self.pending_ui.push_back(PendingUi::Build(draft, open));
            }
            Event::Saved(draft) => self.saved(&draft, SaveStatus::Local),
            Event::Synced(draft, remote) => self.synced(&draft, *remote),
            // A save lost a revision race with something else. The stored copy is shown as a
            // conflict and the user's own text is kept; nothing is overwritten either way.
            Event::SaveRejected(stored) => {
                let id = stored.id;
                let revision = stored.revision;
                if let Some(open) = self.composer.get_mut(id) {
                    open.absorb_metadata(*stored);
                    open.status = SaveStatus::Conflict {
                        remote_revision: revision.to_string(),
                    };
                    open.in_flight = false;
                    open.dirty_since = None;
                }
                self.cancel_send_intent(id, cx);
                self.say(
                    "This draft changed elsewhere — your copy is kept".into(),
                    cx,
                );
            }
            Event::Conflicted(draft, remote) => {
                let id = draft.id;
                let revision = remote.revision.clone();
                let Some(open) = self.composer.get_mut(id) else {
                    return;
                };
                open.absorb_metadata(*draft);
                open.status = SaveStatus::Conflict {
                    remote_revision: revision,
                };
                open.operation = None;
                open.dirty_since = None;
                self.cancel_send_intent(id, cx);
                self.set_composer_sending(id, false, cx);
            }
            Event::Discarded(id) => self.retire_draft(id, cx),
            // The provider no longer has this draft (sent or deleted elsewhere). The surface
            // goes rather than pretending a record that can never be sent is still a draft.
            Event::Missing(id) => {
                self.retire_draft(id, cx);
                self.say("That draft is gone from the server".into(), cx);
            }
            Event::Listed(drafts) => {
                for draft in drafts {
                    self.composer.put(draft);
                }
            }
            Event::Recovered(recovery, drafts) => {
                for draft in drafts {
                    self.composer.put(draft);
                }
                self.recovered = *recovery;
                // A send-later hold survived the restart: it counts again and goes out when due.
                for hold in self.recovered.scheduled.clone() {
                    self.composer.begin_hold(hold);
                }
                // The lists are taken rather than read so the recovery stays on the record
                // for the rest of the session.
                let restored = self.recovered.restored.clone();
                let uncertain = self.recovered.uncertain.clone();
                for id in restored {
                    self.pending_ui.push_back(PendingUi::Recovered(id));
                }
                if !uncertain.is_empty() {
                    let n = uncertain.len();
                    self.say(
                        format!("{n} send(s) were interrupted and are being checked, not retried"),
                        cx,
                    );
                    for id in uncertain {
                        self.composer.reconciling.push(id);
                        self.reconcile_draft(id);
                    }
                }
            }
            Event::Held(hold) => {
                let id = hold.draft;
                self.composer.begin_hold(hold.clone());
                self.hold_after_save.remove(&id);
                self.set_composer_sending(id, true, cx);
                if let Some(window) = window.as_deref_mut() {
                    window.focus(&self.focus_handle, cx);
                }
            }
            Event::HoldCancelled(draft) => {
                let id = draft.id;
                self.composer.holds.remove(&id);
                if let Some(open) = self.composer.get_mut(id) {
                    open.absorb_metadata(*draft);
                    open.operation = None;
                }
                self.set_composer_sending(id, false, cx);
                if let Some(entity) = self.composers.get(&id).cloned() {
                    entity.update(cx, |composer, cx| composer.show_restored_notice(cx));
                }
                self.pending_ui.push_back(PendingUi::RestoreDraft(id));
            }
            Event::Attached { draft, failed } => {
                self.attachment_landed(*draft);
                if !failed.is_empty() {
                    self.cancel_send_intent(changed_draft.expect("attachment draft"), cx);
                }
                self.report_failures(failed, "Could not attach", cx);
            }
            Event::Forwarded { draft, failed } => {
                let carried = draft.attachments.len();
                self.attachment_landed(*draft);
                if !failed.is_empty() {
                    self.cancel_send_intent(changed_draft.expect("forward draft"), cx);
                }
                if carried > 0 {
                    self.say(format!("Forwarded with {carried} attachment(s)"), cx);
                }
                self.report_failures(failed, "Could not carry an attachment", cx);
            }
            Event::Detached(draft) => self.attachment_landed(*draft),
            Event::Imported {
                account,
                fresh,
                already_local,
                failed,
            } => {
                let account = account.clone();
                let imported = fresh.len();
                for draft in fresh {
                    let id = draft.id;
                    if let Some(open) = self.composer.get_mut(id) {
                        open.absorb_metadata(draft);
                    } else {
                        self.composer.put(draft);
                    }
                    self.sync_composer_record(id, cx);
                }
                if imported > 0 {
                    let n = imported;
                    self.say(
                        format!("Imported {n} Gmail draft{}", if n == 1 { "" } else { "s" }),
                        cx,
                    );
                }
                // A provider draft we already hold is that same draft, refreshed into the
                // open one rather than duplicated: the revision decides.
                for remote in already_local {
                    self.refresh_imported(account.as_str(), remote, cx);
                }
                for ImportFailure {
                    draft,
                    remote,
                    detail,
                } in failed
                {
                    // A file that could not be imported is named, never adopted: a draft
                    // that silently lost a file would be sent without it.
                    let what = draft
                        .and_then(|id| self.composer.record(id))
                        .map(|r| tabs::title(&r.subject, 40))
                        .unwrap_or_else(|| remote.clone());
                    self.say(format!("“{what}” was not imported: {detail}"), cx);
                }
            }
            Event::Sent {
                draft,
                sent,
                message,
            } => {
                let id = draft.id;
                self.composer.holds.remove(&id);
                // Delivery settles the draft's uncertainty, whether this send was watched
                // through or confirmed later by a reconciliation.
                self.composer.reconciling.retain(|pending| *pending != id);
                self.say("Sent".into(), cx);
                // The outgoing message needs the draft's own account, body and attachments, so
                // it is recorded *before* the draft is forgotten. Forgetting first is what
                // made a confirmed send vanish without ever reaching the mailbox.
                self.record_sent_message(*message, id, *sent, cx);
                self.retire_draft(id, cx);
            }
            Event::Uncertain(draft, detail) => {
                let id = draft.id;
                if let Some(open) = self.composer.get_mut(id) {
                    open.absorb_metadata(*draft);
                    open.operation = None;
                }
                self.composer.reconciling.push(id);
                self.say(
                    format!("The server may or may not have sent it — checking before sending again ({detail})"),
                    cx,
                );
                self.reconcile_draft(id);
            }
            Event::Offline(draft) => {
                let id = draft.id;
                if let Some(open) = self.composer.get_mut(id) {
                    open.absorb_metadata(*draft);
                    open.operation = None;
                }
                self.composer.holds.remove(&id);
                self.set_composer_sending(id, false, cx);
                self.say(
                    "You appear to be offline — the draft was kept and nothing was sent".into(),
                    cx,
                );
            }
            Event::SendFailed(draft, error) => {
                let id = draft.id;
                if let Some(open) = self.composer.get_mut(id) {
                    open.absorb_metadata(*draft);
                    open.operation = None;
                }
                // The worker has already taken the hold out, so the countdown comes down now
                // instead of running to the end for a send that is not going to happen.
                self.composer.holds.remove(&id);
                self.set_composer_sending(id, false, cx);
                self.say(format!("Not sent: {error}"), cx);
            }
            Event::AutosaveFailed(id, detail) => {
                if let Some(open) = self.composer.get_mut(id) {
                    open.status = SaveStatus::Failed(detail.clone());
                    open.operation = None;
                    open.dirty_since = None;
                }
                self.cancel_send_intent(id, cx);
                self.set_composer_sending(id, false, cx);
                self.say(
                    format!("The draft may not have reached the server: {detail}"),
                    cx,
                );
            }
            Event::StoreFailed(id, error) => {
                if let Some(id) = id
                    && let Some(open) = self.composer.get_mut(id)
                {
                    open.status = SaveStatus::Failed(error.clone());
                    open.in_flight = false;
                    open.operation = None;
                }
                if let Some(id) = id {
                    self.cancel_send_intent(id, cx);
                    self.pending_resolutions.remove(&id);
                    if self
                        .composer
                        .record(id)
                        .is_some_and(|d| !d.state.is_in_flight())
                    {
                        self.set_composer_sending(id, false, cx);
                    }
                }
                self.say(format!("Drafts: {error}"), cx);
            }
            Event::JobFailed(error) => {
                if error.is_empty() {
                    return;
                }
                // A metadata fetch that failed leaves nothing pending: the draft stays
                // editable and the next Send asks again, rather than being blocked forever
                // by a request that will never be answered.
                self.threading_requests.clear();
                self.say(error, cx);
            }
            Event::Reconciled { draft, detail } => {
                // The provider could not confirm. A miss is *not* proof of non-delivery, so
                // the draft stays uncertain and is never sent again: a confirmed attempt
                // arrives as an ordinary `Sent` and goes down the delivery path once.
                let id = draft.id;
                self.composer.reconciling.retain(|pending| *pending != id);
                if let Some(open) = self.composer.get_mut(id) {
                    open.absorb_metadata(*draft);
                    open.operation = None;
                }
                let said = if detail.is_empty() {
                    "Still unconfirmed — nothing was sent again; this draft is kept until you decide".to_owned()
                } else {
                    format!("Could not check: {detail}")
                };
                self.say(said, cx);
            }
            Event::Unsupported { account } => {
                let shown = self.account_label(&account);
                self.say(
                    format!("{shown} cannot send yet — your draft is kept on this Mac"),
                    cx,
                );
            }
            Event::Resolved { draft, mode } => {
                self.resolution_applied(*draft, mode, cx);
                let said = match mode {
                    ResolveMode::KeepLocal => "Your copy is kept",
                    ResolveMode::TakeRemote => "The server's version is now this draft",
                    ResolveMode::KeepBoth => "Both kept",
                };
                self.say(said.into(), cx);
            }
            Event::ResolvedBoth { bound, copy } => {
                self.resolution_split(*bound, *copy, cx);
                self.say("Both kept: yours as a separate draft".into(), cx);
            }
            // The choice could not be carried out: nothing was replaced, and the draft is
            // still here to try again or keep as it was.
            Event::ResolveFailed(id, detail) => {
                self.say(format!("Could not settle that draft: {detail}"), cx);
                if let Some(open) = self.composer.get_mut(id) {
                    open.operation = None;
                }
                self.set_composer_sending(id, false, cx);
            }
            Event::Composable { .. } => {}
            Event::Threaded {
                account,
                message,
                threading,
            } => {
                let key = (account.to_string(), message.to_string());
                let waiting = self.threading_requests.remove(&key).unwrap_or_default();
                for id in waiting {
                    // A draft that moved to another account in the meantime must not take
                    // headers that belonged to the account it left.
                    let same = self
                        .composer
                        .record(id)
                        .is_some_and(|r| r.account == account.as_str());
                    if same {
                        self.apply_threading(id, threading.clone(), cx);
                    }
                }
            }
        }
        if let Some(id) = changed_draft {
            self.sync_composer_record(id, cx);
        }
        let _ = window;
        cx.notify();
    }

    /// A save or a sync answered. The answer carries the identity the store now holds;
    /// anything typed after the request started stays and is written again straight away.
    fn saved(&mut self, draft: &DraftRecord, status: SaveStatus) {
        let id = draft.id;
        let Some(open) = self.composer.get_mut(id) else {
            return;
        };
        let stale = open.absorb(draft.clone());
        open.status = status;
        open.in_flight = false;
        if stale {
            // The debounce restarts from the newest edit, so a burst of typing mirrors once
            // after it settles rather than after every keystroke.
            let now = self.now();
            let Some(open) = self.composer.get_mut(id) else {
                return;
            };
            open.dirty_since = Some(now);
            self.requeue(id);
        }
        // `dirty_since` is deliberately *not* cleared here: the provider's copy is still
        // behind, and clearing it is what stopped ordinary edits from ever being mirrored.
    }

    /// The remote mirror is now in step, so its own debounce can rest.
    fn synced(&mut self, draft: &DraftRecord, remote: RemoteDraft) {
        let id = draft.id;
        let Some(open) = self.composer.get_mut(id) else {
            return;
        };
        open.absorb_metadata(draft.clone());
        open.operation = None;
        open.remote_dirty = open.in_flight
            || open.has_pending_write()
            || open.record.synced_content_revision != Some(open.record.content_revision);
        open.status = if open.remote_dirty {
            SaveStatus::Local
        } else {
            SaveStatus::Synced
        };
        if !open.remote_dirty {
            open.dirty_since = None;
        }
        let _ = remote;
    }

    /// An attachment landed or left: take the store's copy and keep whatever was typed.
    fn attachment_landed(&mut self, draft: DraftRecord) {
        let id = draft.id;
        let now = self.now();
        let Some(open) = self.composer.get_mut(id) else {
            return;
        };
        open.absorb_metadata(draft);
        open.operation = None;
        open.remote_dirty = true;
        open.dirty_since = Some(now);
        open.status = SaveStatus::Local;
    }

    fn report_failures(&mut self, failed: Vec<String>, what: &str, cx: &mut Context<Self>) {
        for failure in failed {
            self.say(format!("{what}: {failure}"), cx);
        }
    }

    /// Push a durable record back into the composer entity that is editing it.
    fn sync_composer_record(&mut self, id: DraftId, cx: &mut Context<Self>) {
        let Some(entity) = self.composers.get(&id).cloned() else {
            return;
        };
        let Some(record) = self.composer.record(id).cloned() else {
            return;
        };
        entity.update(cx, |composer, cx| composer.apply_record(record, cx));
        self.sync_composer_status(id, cx);
    }

    fn sync_composer_status(&mut self, id: DraftId, cx: &mut Context<Self>) {
        let Some(entity) = self.composers.get(&id).cloned() else {
            return;
        };
        let Some(open) = self.composer.get(id) else {
            return;
        };
        let queued_attachment = self.pending_attachment_ops.contains_key(&id);
        let attachments_busy =
            queued_attachment || open.operation == Some(DraftOperation::Attachments);
        let status = if self.hold_after_save.contains(&id) {
            "Finishing the draft before holding…".to_owned()
        } else if queued_attachment && open.operation != Some(DraftOperation::Attachments) {
            "Attachments waiting for the current operation…".to_owned()
        } else {
            open.status_line()
        };
        entity.update(cx, |composer, cx| {
            composer.set_save_status(status, attachments_busy, cx)
        });
    }

    /// Freeze or unfreeze one draft's controls, so a held, sending or unconfirmed message
    /// cannot be edited or sent again behind the user's back.
    fn set_composer_sending(&mut self, id: DraftId, sending: bool, cx: &mut Context<Self>) {
        let Some(entity) = self.composers.get(&id).cloned() else {
            return;
        };
        let hold = self.composer.holds.get(&id).cloned();
        entity.update(cx, |composer, cx| {
            composer.set_hold(hold, cx);
            composer.set_sending(sending, cx)
        });
    }

    fn account_label(&self, account: &str) -> String {
        self.mailbox
            .account(account)
            .map(|a| crate::account_style::nickname_or(a, &a.email).to_owned())
            .unwrap_or_else(|| account.to_owned())
    }

    // ------------------------------------------------------------------- opening

    /// Reply to everyone on the cursor's message: the original's To and Cc, minus us.
    pub(super) fn reply_all_to(
        &mut self,
        id: MessageId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reply_to(id, DraftKind::ReplyAll, window, cx);
    }

    /// Forward the cursor's message, carrying its files along.
    pub(super) fn forward(&mut self, id: MessageId, window: &mut Window, cx: &mut Context<Self>) {
        self.reply_to(id, DraftKind::Forward, window, cx);
    }

    /// The keyboard/palette path: reply to the cursor's message.
    pub(super) fn reply_to_cursor(
        &mut self,
        kind: DraftKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.modal_open() {
            return;
        }
        let Some(id) = self.cursor_id() else { return };
        self.reply_to(id, kind, window, cx);
    }

    /// Whether the composer has nothing left in flight: no queued UI work, no draft still
    /// waiting for its durable identity, and no draft whose save, sync, hold or attachment
    /// work has not been answered. The worker runs on its own threads, so a caller that has
    /// to wait for it (a test) waits on this rather than guessing at a beat.
    pub fn composer_idle(&self) -> bool {
        self.pending_ui.is_empty()
            && self.pending_composers.is_empty()
            && !self
                .composer
                .drafts
                .iter()
                .any(|draft| draft.in_flight || draft.has_pending_write() || draft.operation.is_some())
    }

    /// The draft whose send hold is counting down, if one is. `u` recalls it.
    pub fn hold_in_flight(&self) -> Option<DraftId> {
        // Ordered by when the send was pressed, then by draft so "the oldest" is stable:
        // two sends can be pressed inside the same millisecond, and the map keeps no
        // insertion order of its own.
        let mut holds: Vec<(Timestamp, DraftId)> = self
            .composer
            .holds
            .values()
            .filter(|h| !h.is_scheduled())
            .map(|h| (h.created_at, h.draft))
            .collect();
        holds.sort();
        holds.first().map(|(_, id)| *id)
    }

    /// Undo a specific hold: the draft comes back whole. A toast names its own draft, and
    /// `u` on the keyboard resolves through [`MailApp::undo_oldest_hold`].
    pub(super) fn undo_send_hold(&mut self, id: DraftId, cx: &mut Context<Self>) {
        self.undo_hold(id, cx);
    }

    /// The account a new draft is composed from: the one the cursor's mail belongs to, or
    /// the first account that can send.
    pub(super) fn compose_account(&self) -> Option<String> {
        let cursor = self.cursor_id().and_then(|id| self.mailbox.get(id));
        if let Some(account) = cursor
            && self.mailbox.account(&account.account).is_some()
        {
            return Some(account.account.clone());
        }
        self.mailbox.accounts().first().map(|a| a.id.clone())
    }

    /// Open a blank message in a draft tab.
    pub(super) fn new_mail(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal_open() {
            return;
        }
        let Some(account) = self.compose_account() else {
            self.show_toast("Add an account first".into(), window, cx);
            return;
        };
        let record = DraftRecord::new_email(account, self.now());
        self.open_composer(
            record,
            PendingOpen {
                focus: Focus::Recipients,
                ..Default::default()
            },
            cx,
        );
    }

    /// Reply to `id` (or the cursor's message), inline under the conversation.
    pub(super) fn reply_to(
        &mut self,
        id: MessageId,
        kind: DraftKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.modal_open() {
            return;
        }
        let Some(message) = self.mailbox.get(id).cloned() else {
            return;
        };
        // The conversation already has an inline draft of this intent: reply goes back into
        // it instead of stacking a second one under the same thread.
        if let Some(existing) = self.inline_reply_in_thread(message.thread_id, kind) {
            self.pin_message(id);
            self.reveal_inline_composer(message.thread_id);
            self.focus_draft_body(existing, window, cx);
            cx.notify();
            return;
        }
        let own = self
            .mailbox
            .account(&message.account)
            .map(|a| a.email.clone())
            .unwrap_or_default();
        let record = DraftRecord::prefilled(kind, &message, &own, self.now());
        self.pin_message(id);
        // A reply's recipients are already right, so the body takes the caret; a forward
        // has none, so its recipient field does.
        let open = PendingOpen {
            placement: Placement::Inline,
            focus: match kind {
                DraftKind::Forward => Focus::Recipients,
                _ => Focus::Body,
            },
            forward_from: (kind == DraftKind::Forward).then_some(id),
            // The provider's id for the original, which is what its threading lookup takes.
            // It lives in the cache, never on the message.
            thread_from: self.remote_id_of(id),
        };
        self.open_composer(record, open, cx);
    }

    /// Whether a reply may be held yet.
    ///
    /// A reply to a message on a real provider needs that message's RFC headers: the adapter
    /// only asks the server to thread a message when it has an `In-Reply-To` to send. Holding
    /// before they arrived would let a later, successful send go out as a brand new
    /// conversation. A mock account keeps its local threading, and a subject the user changed
    /// is deliberately its own conversation and needs none of this.
    fn threading_ready(
        &mut self,
        id: DraftId,
        record: &DraftRecord,
        cx: &mut Context<Self>,
    ) -> bool {
        if !matches!(record.kind, DraftKind::Reply | DraftKind::ReplyAll) {
            return true;
        }
        let subject_unchanged = record
            .prefilled_subject
            .as_deref()
            .is_some_and(|original| original.trim() == record.subject.trim());
        if !subject_unchanged || record.in_reply_to.is_some() {
            return true;
        }
        let is_mock = self
            .mailbox
            .account(&record.account)
            .is_some_and(|a| a.provider == ProviderKind::Mock);
        if is_mock {
            return true;
        }
        let Some(message) = self.threading_message_for(record) else {
            // Nothing to ask about: this reply has no remote original to thread onto.
            return true;
        };
        self.request_threading(id, &message);
        self.say(
            "Fetching the message's threading before it can be held".into(),
            cx,
        );
        false
    }

    /// The server-side id of the original this draft answers, when it has one.
    fn threading_message_for(&self, record: &DraftRecord) -> Option<String> {
        let source = record.source_message?;
        self.remote_id_of(source)
    }

    /// Ask the worker to give the record its durable identity, and remember where it
    /// should be shown. The composer entity is built when that answer arrives, so what the
    /// user sees is always a record the store already holds.
    fn open_composer(&mut self, record: DraftRecord, open: PendingOpen, cx: &mut Context<Self>) {
        // A draft reopened from the store already has its identity; one the user just
        // started does not, and the worker assigns it.
        let id = record.id;
        if id.0 > 0 {
            self.pending_ui
                .push_back(PendingUi::Build(Box::new(record), open));
            cx.notify();
            return;
        }
        self.pending_composers.push_back(open);
        if let Some(worker) = self.composer_worker.as_ref() {
            worker.send(Command::Create {
                draft: Box::new(record),
            });
        }
        cx.notify();
    }

    /// Build the composer entity over a persisted record and put it where it belongs.
    fn build_composer(
        &mut self,
        record: DraftRecord,
        open: PendingOpen,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = record.id;
        let placement = open.placement;
        // A reply from a message higher up the thread would otherwise open below the fold,
        // with its Send and Close buttons out of sight.
        let reveal_thread = record
            .source_message
            .and_then(|source| self.mailbox.get(source))
            .map(|message| message.thread_id);
        let entity = cx.new(|cx| {
            Composer::new(
                record.clone(),
                placement,
                open.focus,
                self.compose_accounts(),
                window,
                cx,
            )
        });
        self.give_recipient_context(&record, &entity, cx);
        let sub = cx.subscribe_in(
            &entity,
            window,
            move |this, _, event: &ComposeEvent, window, cx| {
                this.on_composer_event(id, event.clone(), window, cx);
            },
        );
        self._composer_subs.push(sub);
        let (clock, zone) = (self.clock.clone(), self.zone.clone());
        entity.update(cx, |composer, _| composer.set_clock(clock, zone));
        self.composers.insert(id, entity.clone());
        self.composer.put(record);
        if let Some(state) = self.composer.get_mut(id) {
            state.inline = placement == Placement::Inline;
        }
        if placement == Placement::Inline {
            self.close_other_inline(id, cx);
            if let Some(thread) = reveal_thread {
                self.reveal_inline_composer(thread);
            }
        } else {
            self.tabs.open_draft_pinned(id);
        }
        // A draft reopened while its send-later hold is waiting shows the frozen state.
        if self.composer.holds.contains_key(&id) {
            self.set_composer_sending(id, true, cx);
        }
        self.sync_composer_status(id, cx);
        cx.notify();
    }

    /// The accounts the composer's account selector offers, each with the rail's icon and tint.
    fn compose_accounts(&self) -> Vec<AccountChoice> {
        self.mailbox
            .accounts()
            .iter()
            .map(|a| AccountChoice {
                id: a.id.clone(),
                label: crate::account_style::nickname_or(a, &a.email).to_owned(),
                icon: crate::account_style::icon_key(a),
                color: a.color.clone(),
            })
            .collect()
    }

    /// A thread shows one inline draft. Another one appearing there moves the older draft to
    /// a tab rather than disappearing; other threads keep theirs.
    fn close_other_inline(&mut self, id: DraftId, cx: &mut Context<Self>) {
        let Some(thread) = self.thread_of_draft(id) else {
            return;
        };
        let displaced: Vec<DraftId> = self
            .composer
            .drafts
            .iter()
            .filter(|open| open.inline && open.record.id != id)
            .filter(|open| {
                open.record
                    .source_message
                    .and_then(|source| self.thread_of(source))
                    == Some(thread)
            })
            .map(|open| open.record.id)
            .collect();
        for previous in displaced {
            if let Some(entity) = self.composers.get(&previous).cloned() {
                entity.update(cx, |composer, cx| {
                    composer.set_placement(Placement::Tab, cx)
                });
            }
            if let Some(open) = self.composer.get_mut(previous) {
                open.inline = false;
            }
            // The user is looking at the thread the new reply belongs to: the displaced
            // draft keeps its tab without taking that away.
            self.tabs.add_draft_pinned(previous);
        }
    }

    /// The conversation `draft` answers, by its source message.
    fn thread_of_draft(&self, draft: DraftId) -> Option<u32> {
        let source = self.composer.record(draft)?.source_message?;
        self.thread_of(source)
    }

    /// The inline draft attached to `thread`, if any.
    fn inline_draft_in(&self, thread: u32) -> Option<DraftId> {
        self.composer
            .drafts
            .iter()
            .find(|open| {
                open.inline
                    && open.record.source_message.and_then(|source| self.thread_of(source))
                        == Some(thread)
            })
            .map(|open| open.record.id)
    }

    /// An inline draft of the same intent answering `thread`: a reply looks for a reply, a
    /// forward for a forward, so `f` after `r` still forwards.
    fn inline_reply_in_thread(&self, thread: u32, kind: DraftKind) -> Option<DraftId> {
        let forward = kind == DraftKind::Forward;
        self.composer
            .drafts
            .iter()
            .find(|open| {
                open.inline
                    && (open.record.kind == DraftKind::Forward) == forward
                    && open.record.source_message.and_then(|source| self.thread_of(source))
                        == Some(thread)
            })
            .map(|open| open.record.id)
    }

    /// Scroll `thread` down to its inline composer on the next render.
    fn reveal_inline_composer(&mut self, thread: u32) {
        self.reader_panes
            .borrow_mut()
            .entry(thread)
            .or_default()
            .reveal_composer = true;
    }

    fn tabs_index_of_draft(&self, id: DraftId) -> Option<usize> {
        self.tabs.index_of_draft(id)
    }

    // ------------------------------------------------------------------ the events

    fn on_composer_event(
        &mut self,
        id: DraftId,
        event: ComposeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let edited = matches!(event, ComposeEvent::Edited);
        match event {
            ComposeEvent::Edited => {
                let Some(entity) = self.composers.get(&id).cloned() else {
                    return;
                };
                if self.hold_after_save.contains(&id) {
                    return;
                }
                let now = self.now();
                // Copy the form's text fields straight into the in-memory record. No whole
                // record (or attachment list) is cloned here; the beat snapshots once when
                // the debounce fires.
                let (to, cc, bcc, subject, body) = entity.read(cx).draft_fields();
                let Some(open) = self.composer.get_mut(id) else {
                    return;
                };
                if matches!(
                    open.record.state,
                    DraftState::Held
                        | DraftState::Sending
                        | DraftState::Uncertain
                        | DraftState::Sent { .. }
                ) {
                    return;
                }
                open.edit_form(to, cc, bcc, subject, body, now);
                entity.update(cx, |composer, cx| {
                    open.record.set_intent(composer.intent());
                    composer.dismiss_restored_notice(cx);
                });
                // The write itself is not started here: the beat writes the queued snapshot
                // once typing pauses, so a keystroke never waits on a database transaction.
                self.sync_composer_status(id, cx);
            }
            ComposeEvent::Send => self.request_send(id, window, cx),
            ComposeEvent::Attach => self.attach_files(id, cx),
            ComposeEvent::RemoveAttachment(attachment) => {
                self.detach(id, &attachment, cx);
            }
            ComposeEvent::Close => {
                self.close_composer(id);
                // The surface that owned the keyboard is gone, so the app takes focus back:
                // list keys work straight after closing the composer instead of being
                // swallowed by an input that no longer exists.
                window.focus(&self.focus_handle, cx);
            }
            ComposeEvent::Discard => self.pending_ui.push_back(PendingUi::ConfirmDiscard(id)),
            ComposeEvent::Expand => self.expand_composer(id, cx),
            ComposeEvent::Collapse => self.pending_ui.push_back(PendingUi::Collapse(id)),
            ComposeEvent::ResolveConflict => self.confirm_resolution(id, window, cx),
            ComposeEvent::CheckDelivery => {
                self.reconcile_draft(id);
                self.sync_composer_status(id, cx);
            }
            ComposeEvent::SetAccount(account) => self.switch_account(id, account, cx),
        }
        // An edit already scheduled its frame through the input's own notify, and the
        // composer repaints its own status; notifying MailApp again would only mark it dirty
        // a second time for the same frame. Every other event changes what MailApp draws.
        if !edited {
            cx.notify();
        }
    }

    /// Send this draft from a different account.
    ///
    /// The content, attachments and reply threading are all kept. What cannot cross an
    /// account is a *provider* identity: the remote draft id, its revision and the thread it
    /// named belong to the old account's backend, so they are dropped and the draft becomes
    /// a local one the new account will create its own copy of. The old provider draft is
    /// left alone — it is still that account's, and deleting it is not this action's job.
    ///
    /// Only an untouched-enough draft may move: one already held, attempted or unconfirmed
    /// has a send in flight, and changing who it goes from under that is not a choice.
    fn switch_account(&mut self, id: DraftId, account: String, cx: &mut Context<Self>) {
        let Some(mut record) = self.composer.record(id).cloned() else {
            return;
        };
        if self.hold_after_save.contains(&id) {
            return;
        }
        if record.account == account {
            return;
        }
        if !matches!(record.state, DraftState::Editing) || record.attempted {
            self.say(
                "This draft has already been sent or is on its way; its account is fixed".into(),
                cx,
            );
            return;
        }
        let had_remote = record.remote_id.is_some();
        record.account = account;
        record.remote_id = None;
        record.remote_revision = None;
        record.thread_remote_id = None;
        record.synced_content_revision = None;
        if let Some(open) = self.composer.get_mut(id) {
            open.record.account = record.account.clone();
            open.record.remote_id = None;
            open.record.remote_revision = None;
            open.record.thread_remote_id = None;
            open.record.synced_content_revision = None;
        }
        self.queue_edit(id, record);
        if let Some(open) = self.composer.get_mut(id) {
            open.mark_urgent();
        }
        self.sync_composer_record(id, cx);
        self.write_draft(id);
        if had_remote {
            // The copy on the old account's server is untouched and still listed there.
            self.say(
                "Sending from a different account; the old draft stays in its Drafts folder".into(),
                cx,
            );
        }
        cx.notify();
    }

    /// Record an edit: mark the draft newer than the store and queue the snapshot for an
    /// immediate local write.
    fn queue_edit(&mut self, id: DraftId, record: DraftRecord) {
        let now = self.now();
        if self.hold_after_save.contains(&id) {
            return;
        }
        let Some(open) = self.composer.get_mut(id) else {
            return;
        };
        if matches!(
            open.record.state,
            DraftState::Held
                | DraftState::Sending
                | DraftState::Uncertain
                | DraftState::Sent { .. }
        ) {
            return;
        }
        open.touch(record, now);
    }

    /// Put a draft's newest form state in the queue, because the answer that just landed
    /// carried content from before the last edit.
    fn requeue(&mut self, id: DraftId) {
        let now = self.now();
        let Some(open) = self.composer.get_mut(id) else {
            return;
        };
        let snapshot = open.record.clone();
        open.requeue(snapshot, now);
    }

    /// Write the queued snapshot to the local store. One save at a time per draft: a second
    /// would be refused by the store's revision check anyway, and this makes that explicit.
    ///
    /// Called both from the beat (when the autosave debounce is up) and directly by the
    /// explicit save paths — Close, Send, switching account — which is why it writes the
    /// queue rather than consulting the debounce itself.
    fn write_draft(&mut self, id: DraftId) {
        let Some(worker) = self.composer_worker.as_ref() else {
            return;
        };
        let Some(open) = self.composer.get_mut(id) else {
            return;
        };
        if open.in_flight {
            // The pending snapshot stays queued, and an explicit save keeps its urgency so
            // the write that follows the in-flight answer does not wait out the debounce.
            return;
        }
        // One snapshot per debounce, taken here rather than at every keystroke.
        open.snapshot_pending();
        let Some(record) = open.take_queued() else {
            return;
        };
        open.urgent = false;
        let expected = record.editor_revision;
        open.status = SaveStatus::Saving;
        open.in_flight = true;
        open.saved_generation = open.generation;
        worker.send(Command::Save {
            draft: Box::new(record),
            expected,
        });
    }

    /// Queue the draft the composer is editing right now, straight from the form, and write
    /// it without waiting out the autosave debounce: Send, switching account and Close all
    /// come through here because the newest text has to be durable before they proceed.
    fn save_from_editor(&mut self, id: DraftId, cx: &mut Context<Self>) {
        if let Some(entity) = self.composers.get(&id).cloned() {
            let record = entity.read(cx).record();
            self.queue_edit(id, record);
        }
        if let Some(open) = self.composer.get_mut(id) {
            open.mark_urgent();
        }
        self.write_draft(id);
    }

    /// Mirror a settled draft to the provider's own draft store. Only this waits for a
    /// pause in typing; the local copy was written at the keystroke.
    fn sync_draft(&mut self, id: DraftId) {
        let Some(worker) = self.composer_worker.as_ref() else {
            return;
        };
        if self.pending_attachment_ops.contains_key(&id) || self.hold_after_save.contains(&id) {
            return;
        }
        let Some(open) = self.composer.get_mut(id) else {
            return;
        };
        if open.in_flight
            || open.has_pending_write()
            || open.operation.is_some()
            || !matches!(
                open.record.state,
                DraftState::Editing | DraftState::Failed { .. }
            )
        {
            return;
        }
        open.dirty_since = None;
        open.operation = Some(DraftOperation::Sync);
        worker.send(Command::Sync { draft: id });
    }

    // ------------------------------------------------------------------- sending

    fn request_send(&mut self, id: DraftId, window: &mut Window, cx: &mut Context<Self>) {
        // A second submit while the message is held does not send: the countdown is the
        // point, and Undo on the countdown toast is what brings it back.
        if self.composer.holds.contains_key(&id) || self.hold_after_save.contains(&id) {
            return;
        }
        let Some(record) = self.composer.record(id).cloned() else {
            return;
        };
        if !matches!(
            record.state,
            DraftState::Editing | DraftState::Failed { .. }
        ) {
            return;
        }
        if let Err(problem) = record.validate() {
            self.show_toast(problem, window, cx);
            return;
        }
        if !self.threading_ready(id, &record, cx) {
            return;
        }
        // The draft is durable before it can be sent, so the hold always has something to
        // restore, and a hold has a complete draft to give back. If a save is already running
        // the newest snapshot cannot go out yet, so the hold waits for it rather than
        // counting down a body that is one edit behind.
        self.save_from_editor(id, cx);
        self.hold_after_save.insert(id);
        self.set_composer_sending(id, true, cx);
        self.hold_when_persisted(id, cx);
        self.sync_composer_status(id, cx);
    }

    /// Put a draft into its hold. Only reached once everything typed is stored.
    fn begin_hold(&mut self, id: DraftId, cx: &mut Context<Self>) {
        if self.composer.holds.contains_key(&id) {
            return;
        }
        let now = self.now();
        if let Some(worker) = self.composer_worker.as_ref() {
            let Some(open) = self.composer.get_mut(id) else {
                return;
            };
            if open.operation.is_some() {
                return;
            }
            open.operation = Some(DraftOperation::Hold);
            // Send later holds until its time; anything else waits out the undo window.
            let at = open.record.intent().send_at();
            worker.send(Command::BeginHold { draft: id, now, at });
        }
        cx.notify();
    }

    /// A draft whose send was asked for while a save was still running. The hold starts as
    /// soon as that save lands, so the countdown counts the message as it now stands.
    fn hold_when_persisted(&mut self, id: DraftId, cx: &mut Context<Self>) {
        if !self.hold_after_save.contains(&id) {
            return;
        }
        let Some(open) = self.composer.get(id) else {
            return;
        };
        if matches!(
            open.status,
            SaveStatus::Failed(_) | SaveStatus::Conflict { .. }
        ) || !matches!(
            open.record.state,
            DraftState::Editing | DraftState::Failed { .. }
        ) {
            self.cancel_send_intent(id, cx);
            return;
        }
        if open.in_flight
            || open.has_pending_write()
            || open.operation.is_some()
            || self.pending_attachment_ops.contains_key(&id)
        {
            return;
        }
        if let Err(problem) = open.record.validate() {
            self.say(problem, cx);
            self.cancel_send_intent(id, cx);
            return;
        }
        self.begin_hold(id, cx);
    }

    fn cancel_send_intent(&mut self, id: DraftId, cx: &mut Context<Self>) {
        if self.hold_after_save.remove(&id) {
            self.set_composer_sending(id, false, cx);
        }
    }

    /// Send the held draft right away: the countdown ends, nothing else changes.
    fn flush_hold(&mut self, id: DraftId) {
        if self.composer.holds.remove(&id).is_none() {
            return;
        }
        let Some(record) = self.composer.record(id).cloned() else {
            return;
        };
        let account = record.account.clone();
        if let Some(open) = self.composer.get_mut(id) {
            open.operation = Some(DraftOperation::Send);
            open.record.state = DraftState::Sending;
        }
        if let Some(worker) = self.composer_worker.as_ref() {
            worker.send(Command::Send {
                draft: id,
                account,
                now: self.now(),
            });
        }
    }

    /// Undo a held send: the draft comes back whole, attachments included.
    fn undo_hold(&mut self, id: DraftId, cx: &mut Context<Self>) {
        if self.composer.holds.remove(&id).is_none() {
            return;
        }
        if let Some(open) = self.composer.get_mut(id) {
            open.operation = Some(DraftOperation::Hold);
        }
        if let Some(worker) = self.composer_worker.as_ref() {
            worker.send(Command::CancelHold(id));
        }
        cx.notify();
    }

    /// Ask the provider whether an uncertain send arrived. Nothing is sent again here.
    fn reconcile_draft(&mut self, id: DraftId) {
        let Some(record) = self.composer.record(id).cloned() else {
            return;
        };
        let Some(worker) = self.composer_worker.as_ref() else {
            return;
        };
        if let Some(open) = self.composer.get_mut(id) {
            if open.operation.is_some() {
                return;
            }
            open.operation = Some(DraftOperation::Reconcile);
        }
        worker.send(Command::Reconcile {
            draft: id,
            account: record.account,
        });
    }

    /// Put a confirmed outgoing message into the mailbox, once.
    ///
    /// The server's id for it is remembered so the next sync adopts the same message
    /// instead of adding a second copy, and the thread it was sent in decides where the
    /// local copy lands: a reply keeps its original's thread, a new message gets the one
    /// the provider put it in.
    fn record_sent_message(
        &mut self,
        message: crate::outgoing::OutgoingMessage,
        draft: DraftId,
        sent: SentMail,
        cx: &mut Context<Self>,
    ) {
        let Some(record) = self.composer.record(draft).cloned() else {
            return;
        };
        let account = record.account.clone();
        // A message the sync loop has already seen is adopted, not duplicated.
        if let Some(local) = self.message_by_remote(&account, &sent.id) {
            self.last_sent_remote = Some(sent.id.clone());
            let original = self.triage_original(&record, &sent);
            self.after_send(original, original.unwrap_or(local), record.intent(), cx);
            return;
        }
        // The provider says which thread the message landed in, and that is where the copy
        // goes: a reply the user re-headed, or one the server could not thread, arrives in a
        // thread of its own and must not be filed back under the conversation it no longer
        // belongs to.
        let mapped = self
            .cache
            .as_ref()
            .map(|cache| cache.thread_id(&account, &sent.thread, || self.next_thread_id()))
            .transpose()
            .ok()
            .flatten();
        // Two different originals, deliberately: the server's thread decides where the copy
        // lands, and the triage original decides what the reply inherits. They only agree
        // when the delivered message really is in the original's thread; where the provider
        // put it elsewhere, no reply semantics are inherited from a message it is not part of.
        let triage_original = self
            .triage_original(&record, &sent)
            .filter(|id| mapped.is_none_or(|thread| self.thread_of(*id) == Some(thread)));
        let thread_id = match mapped {
            Some(thread) => thread,
            // With no mapping to ask, the draft's own thread hint is the only account of where
            // the provider put the message, and it counts only when it still names that thread.
            None => record
                .thread_remote_id
                .as_deref()
                .filter(|thread| *thread == sent.thread)
                .and(triage_original)
                .and_then(|id| self.mailbox.get(id).map(|message| message.thread_id))
                .unwrap_or_else(|| self.next_thread_id()),
        };
        let account_email = self
            .mailbox
            .account(&account)
            .map(|a| a.email.clone())
            .unwrap_or_else(|| account.clone());
        let from_name = self
            .mailbox
            .account(&account)
            .map(|a| a.name.clone())
            .unwrap_or_else(|| account_email.clone());
        let now = self.now();
        let attachments = self.composer.attachments(draft);
        let outgoing = Message {
            id: 0,
            thread_id,
            from_name,
            from_email: account_email,
            to: message.to.clone(),
            cc: message.cc.clone(),
            bcc: message.bcc.clone(),
            subject: message.subject.clone(),
            body: message.text.clone(),
            html: Some(message.html.clone()),
            received: crate::outgoing::mime::rfc2822_date(now),
            state: TriageState::Inbox,
            account,
            outgoing: true,
            snooze: None,
            read: true,
            partial: false,
            bulk: false,
            attachments: crate::app::composer_worker::attachment_names(&attachments)
                .into_iter()
                .zip(attachments.iter().map(|a| a.size))
                .map(|(name, size)| Attachment { name, size })
                .collect(),
        };
        // A reply that asks for an answer leaves its original awaiting one, but only while
        // automatic classification is on. The reply question is answered locally, on the
        // stub, so sending never waits on the network.
        let expects = self.judge.is_some()
            && self.auto_classify
            && crate::judge::StubJudge
                .judge(
                    &serde_json::json!({"sender": "", "subject": "", "body": message.text.trim()}),
                    &[crate::judge::reply_question()],
                )
                .is_ok_and(|answers| {
                    answers.iter().any(|(key, answer)| {
                        *key == crate::judge::QuestionKey::ExpectsReply
                            && answer.value == crate::judge::AnswerValue::Bool(true)
                    })
                });
        let local = self
            .mailbox
            .record_sent(outgoing, triage_original, expects, now);
        self.cache_outgoing(local, &sent);
        self.last_sent_remote = Some(sent.id.clone());
        self.after_send(triage_original, triage_original.unwrap_or(local), record.intent(), cx);
        self.pending_ui.push_back(PendingUi::FocusBody(draft));
    }

    /// The message a confirmed send should inherit its triage from, which is the draft's own
    /// original — read off the draft, not off a global left over from another send.
    ///
    /// It counts only when the draft really is a reply to that message: same account, and a
    /// subject the user has not changed. A subject the user edited starts its own
    /// conversation — the worker drops the reply headers for exactly that reason — so it must
    /// not be filed as though it answered the original.
    fn triage_original(&self, record: &DraftRecord, sent: &SentMail) -> Option<MessageId> {
        // The provider says which thread the message actually landed in, so that decides.
        // A local `source_message` alone is not enough: after an account is removed and added
        // back the local ids may be reassigned, and an unrelated message can share a subject.
        let local_in_sent_thread = |id: MessageId| {
            let message = self.mailbox.get(id)?;
            (message.account == record.account)
                .then_some(())
                .map(|_| id)
        };
        let mapped = self
            .mailbox
            .messages()
            .iter()
            .filter(|m| m.account == record.account)
            .find(|m| self.remote_thread_of(m.id).as_deref() == Some(sent.thread.as_str()))
            .map(|m| m.id);
        if let Some(id) = mapped {
            return local_in_sent_thread(id);
        }
        if !matches!(record.kind, DraftKind::Reply | DraftKind::ReplyAll) {
            return None;
        }
        let source = record.source_message?;
        let unchanged = record
            .prefilled_subject
            .as_deref()
            .is_some_and(|original| original.trim() == record.subject.trim());
        if !unchanged {
            return None;
        }
        // A mock fixture backend has no server-side thread to compare against, so its drafts
        // fall back to the same account plus the unchanged subject.
        let is_mock = self
            .mailbox
            .account(&record.account)
            .is_some_and(|a| a.provider == ProviderKind::Mock);
        if !is_mock {
            return None;
        }
        local_in_sent_thread(source)
    }

    /// The provider's thread id for a local message, resolved through the cache's mapping.
    fn remote_thread_of(&self, id: MessageId) -> Option<String> {
        self.cache.as_ref()?.remote_thread_of(id).ok().flatten()
    }

    /// The server-side id of a local message. Remote ids live in the cache, never on the
    /// message itself, so this is the one place the app asks for them.
    fn remote_id_of(&self, id: MessageId) -> Option<String> {
        self.cache
            .as_ref()?
            .remote_state(id)
            .ok()
            .flatten()
            .map(|(_, remote, _)| remote)
    }

    /// The local message this account already holds for a server-side id.
    fn message_by_remote(&self, account: &str, remote_id: &str) -> Option<MessageId> {
        let id = self
            .cache
            .as_ref()?
            .message_by_remote(account, remote_id)
            .ok()
            .flatten()?;
        self.mailbox.get(id).map(|_| id)
    }

    /// The local message this draft answers, when it was a reply: the record names the
    /// original directly, so two drafts replying to different messages do not confuse each
    /// other the way one global "last reply" would.
    fn draft_original(&self, draft: DraftId) -> Option<MessageId> {
        self.composer.record(draft)?.source_message
    }

    /// The local thread id for a message that starts its own thread.
    fn next_thread_id(&self) -> u32 {
        self.mailbox
            .messages()
            .iter()
            .map(|m| m.thread_id)
            .max()
            .map_or(1, |max| max + 1)
    }

    /// Remember a confirmed send in the cache under the server's own id, so the next sync
    /// recognises it rather than delivering it twice.
    fn cache_outgoing(&mut self, local: MessageId, sent: &SentMail) {
        let Some(cache) = self.cache.as_ref() else {
            return;
        };
        let Some(message) = self.mailbox.get(local).cloned() else {
            return;
        };
        // A confirmed send is remembered under the server's own id, so the next sync adopts
        // this very message instead of adding a second copy of it.
        let _ = cache.upsert_message(
            &message,
            &sent.id,
            &crate::provider::RemoteState::Inbox,
            true,
        );
    }

    /// What a confirmed send leaves behind: the thread's fate as the sentence said (`target`
    /// is the original it answered, or the sent message for a new thread), else the
    /// post-send filing choice, offered only for a reply whose original this very draft names.
    fn after_send(
        &mut self,
        original: Option<MessageId>,
        target: MessageId,
        intent: SendIntent,
        cx: &mut Context<Self>,
    ) {
        if original.is_some() && original == self.session_current() {
            self.session_advance(super::Outcome::Replied);
        }
        let now = self.now();
        self.mailbox.apply_send_intent(target, intent, now);
        let said = match intent {
            SendIntent::Archive => Some("Sent · thread archived".to_owned()),
            SendIntent::RemindIfNoReply { days } => {
                Some(format!("Sent · reminding you in {days}d if no reply"))
            }
            SendIntent::JustSend | SendIntent::SendLater { .. } => None,
        };
        // The sentence already answered what to do with the thread: no filing dialog on top.
        let asked = said.is_some();
        if let Some(said) = said {
            self.say(said, cx);
        }
        if let Some(original) = original
            && !asked
            && !self.modal_open()
        {
            self.pending_ui.push_back(PendingUi::PostSend(original));
        }
        self.scroll_to_cursor();
        cx.notify();
    }

    // ------------------------------------------------------------ attachments

    /// Open the file picker and hand the picked files to the worker.
    fn attach_files(&mut self, id: DraftId, cx: &mut Context<Self>) {
        let app = cx.weak_entity();
        let picker = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach".into()),
        });
        cx.spawn(async move |_, cx| {
            // A cancelled prompt is not an error: the user simply attached nothing.
            let Ok(Ok(Some(picked))) = picker.await else {
                return;
            };
            let files: Vec<PathBuf> = picked.into_iter().collect();
            if files.is_empty() {
                return;
            }
            let _ = app.update(cx, |this, cx| this.attach_files_at(id, files, cx));
        })
        .detach();
    }

    /// Hand `files` to the worker as this draft's attachments.
    ///
    /// Split out from the picker so the path *after* the prompt is one callable step: the
    /// prompt is a native dialog no headless test can drive, but what it feeds — marking the
    /// draft busy and sending the command — is the part worth exercising.
    pub fn attach_files_at(&mut self, id: DraftId, files: Vec<PathBuf>, cx: &mut Context<Self>) {
        if files.is_empty() {
            return;
        }
        self.enqueue_attachment_op(id, AttachmentOp::Attach(files), cx);
    }

    fn detach(&mut self, id: DraftId, attachment: &AttachmentId, cx: &mut Context<Self>) {
        self.enqueue_attachment_op(id, AttachmentOp::Detach(attachment.clone()), cx);
    }

    /// Carry the original message's files into a forward. The bytes are copied into the
    /// draft's own store, so the draft stays complete even if the original moves.
    fn forward_attachments(&mut self, draft: DraftId, message: MessageId, cx: &mut Context<Self>) {
        let account = self
            .mailbox
            .get(message)
            .map(|m| m.account.clone())
            .unwrap_or_default();
        // The provider addresses the original's *message*, which the cache holds the id for.
        let Some(remote) = self.remote_id_of(message) else {
            return;
        };
        self.enqueue_attachment_op(
            draft,
            AttachmentOp::Forward {
                account,
                message: remote,
            },
            cx,
        );
    }

    fn enqueue_attachment_op(&mut self, id: DraftId, op: AttachmentOp, cx: &mut Context<Self>) {
        let Some(open) = self.composer.get(id) else {
            return;
        };
        if self.hold_after_save.contains(&id)
            || !matches!(
                open.record.state,
                DraftState::Editing | DraftState::Failed { .. } | DraftState::Conflict
            )
        {
            return;
        }
        self.pending_attachment_ops
            .entry(id)
            .or_default()
            .push_back(op);
        self.flush_attachment_op(id);
        self.sync_composer_status(id, cx);
        cx.notify();
    }

    fn flush_attachment_op(&mut self, id: DraftId) {
        let Some(worker) = self.composer_worker.as_ref() else {
            return;
        };
        let Some(open) = self.composer.get_mut(id) else {
            return;
        };
        if open.operation.is_some() {
            return;
        }
        let Some(queue) = self.pending_attachment_ops.get_mut(&id) else {
            return;
        };
        let Some(op) = queue.pop_front() else { return };
        if queue.is_empty() {
            self.pending_attachment_ops.remove(&id);
        }
        open.operation = Some(DraftOperation::Attachments);
        worker.send(match op {
            AttachmentOp::Attach(files) => Command::Attach { draft: id, files },
            AttachmentOp::Detach(attachment) => Command::Detach {
                draft: id,
                attachment,
            },
            AttachmentOp::Forward { account, message } => Command::ForwardAttachments {
                draft: id,
                account,
                message,
            },
        });
    }

    /// Ask for the RFC threading of a message being answered, so the reply is threaded. The
    /// answer names the message, so the draft waiting for it is remembered here.
    fn request_threading(&mut self, draft: DraftId, message: &RemoteId) {
        let account = self
            .composer
            .record(draft)
            .map(|r| r.account.clone())
            .unwrap_or_default();
        self.threading_requests
            .entry((account.clone(), message.clone()))
            .or_default()
            .push(draft);
        if let Some(worker) = self.composer_worker.as_ref() {
            worker.send(Command::Threading {
                message: message.clone(),
                account,
            });
        }
    }

    fn apply_threading(&mut self, draft: DraftId, threading: Threading, cx: &mut Context<Self>) {
        let context = crate::outgoing::ReplyContext {
            message_id: threading.message_id,
            references: threading.references,
            subject: threading.subject,
            thread: None,
        };
        let now = self.now();
        let Some(open) = self.composer.get_mut(draft) else {
            return;
        };
        let before = open.record.in_reply_to.clone();
        open.record.set_threading(&context);
        // A threading answer that arrives after the user already typed must not undo it.
        if open.generation == 0 {
            open.queued = None;
        }
        if open.record.in_reply_to != before {
            let snapshot = open.record.clone();
            open.record.touch(now);
            open.queued = Some(snapshot);
        }
        self.sync_composer_record(draft, cx);
        self.write_draft(draft);
        cx.notify();
    }

    // --------------------------------------------------------------- surfaces

    /// Emit a composer event to the draft the user is working in, for a command that
    /// arrives while focus is outside the composer (the palette hands focus back to the
    /// app before running one).
    pub(super) fn emit_to_composer(&self, event: ComposeEvent, cx: &mut Context<Self>) {
        let Some(id) = self.tabs.active_draft().or_else(|| self.inline_draft()) else {
            return;
        };
        if let Some(entity) = self.composers.get(&id).cloned() {
            entity.update(cx, |_, cx| cx.emit(event));
        }
    }

    fn expand_composer(&mut self, id: DraftId, cx: &mut Context<Self>) {
        // The same draft moves; nothing is re-created, so nothing typed is lost.
        if let Some(entity) = self.composers.get(&id).cloned() {
            entity.update(cx, |composer, cx| {
                composer.set_placement(Placement::Tab, cx)
            });
        }
        if let Some(open) = self.composer.get_mut(id) {
            open.inline = false;
        }
        self.tabs.open_draft_pinned(id);
        cx.notify();
    }

    /// The inline draft of the conversation on screen, if it has one.
    ///
    /// A reply belongs under its own original: shown under another thread it would read as
    /// an answer to that one. Drafts stay attached to their thread, so another tab simply
    /// shows none and the draft reappears with its own conversation.
    pub(super) fn inline_composer_for_thread(&self) -> Option<DraftId> {
        let active = if self.triage_open() {
            self.session_current().and_then(|id| self.thread_of(id))
        } else {
            self.tabs
                .active()
                .and_then(|tab| tab.thread_id())
                .or_else(|| {
                    self.triage()
                        .cursor(&self.mailbox, &self.local_now())
                        .and_then(|id| self.thread_of(id))
                })
        }?;
        self.inline_draft_in(active)
    }

    fn collapse_composer(&mut self, id: DraftId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = self.draft_original(id) else {
            return;
        };
        if self.mailbox.get(source).is_none() {
            self.say(
                "The original conversation is unavailable; keep this draft in its tab.".into(),
                cx,
            );
            return;
        }
        self.show_inline(id, source, window, cx);
    }

    /// Put `draft` back inline under `source`'s conversation: close its tab if it has one,
    /// open and pin the thread, scroll the composer into view and put the caret in the body.
    /// Used by Collapse and by reopening a reply from the Drafts pane.
    fn show_inline(
        &mut self,
        id: DraftId,
        source: MessageId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(entity) = self.composers.get(&id).cloned() {
            entity.update(cx, |composer, cx| {
                composer.set_placement(Placement::Inline, cx)
            });
        }
        if let Some(open) = self.composer.get_mut(id) {
            open.inline = true;
        }
        if let Some(ix) = self.tabs_index_of_draft(id) {
            self.tabs.close(ix);
        }
        self.pin_message(source);
        self.follow_active_tab();
        self.close_other_inline(id, cx);
        // The composer is the thread's last child, so the reader has to be at the bottom for
        // it to be on screen. The reveal runs on the next render, after the reply is in the
        // tree, and is what an inline reply opened from a message higher up already does.
        if let Some(thread) = self.mailbox.get(source).map(|message| message.thread_id) {
            self.reveal_inline_composer(thread);
        }
        self.focus_draft_body(id, window, cx);
        cx.notify();
    }

    fn focus_draft_body(&mut self, id: DraftId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entity) = self.composers.get(&id).cloned() else {
            return;
        };
        let body = entity.read(cx).body_focus(cx);
        window.focus(&body, cx);
    }

    /// Close a surface and keep the draft: the tab goes, the record stays, and the draft
    /// reopens from the drafts list. An untouched draft is kept too — closing is never a
    /// discard, and only the discard path asks first and then deletes.
    pub(super) fn close_composer(&mut self, id: DraftId) {
        // Closing is an explicit save: the pending write goes out now instead of waiting out
        // the autosave debounce, even if a save is already in flight (it then follows that
        // answer without another wait).
        if let Some(open) = self.composer.get_mut(id) {
            open.mark_urgent();
            open.inline = false;
        }
        self.write_draft(id);
        if let Some(ix) = self.tabs_index_of_draft(id) {
            self.tabs.close(ix);
        }
    }

    /// Ask what to do with a draft tab that is being closed by its x (or `cmd-w`): Send it,
    /// save and close it, or discard it. The dialog is the confirmation, so any choice acts
    /// at once.
    pub(super) fn confirm_draft_tab_close(
        &mut self,
        id: DraftId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let subject = self
            .composer
            .record(id)
            .map(|r| r.subject.clone())
            .unwrap_or_default();
        let title = if subject.trim().is_empty() {
            "Close this draft?".to_owned()
        } else {
            format!("Close “{}”?", tabs::title(&subject, 40))
        };
        let options = vec![
            DialogOption::new(
                "1",
                "Send",
                "Send it now; held ten seconds so it can be recalled",
            ),
            DialogOption::new(
                "2",
                "Save & Close",
                "Save the draft now and close the composer",
            ),
            DialogOption::new(
                "3",
                "Discard",
                "Delete the draft, attachments included",
            ),
        ];
        self.open_dialog(
            DialogSpec::new(
                title,
                "The draft is open in a tab.".to_owned(),
                options,
                1,
            ),
            move |this, ix, window, cx| match ix {
                0 => this.request_send(id, window, cx),
                1 => this.close_composer(id),
                _ => this.discard_draft(id, cx),
            },
            window,
            cx,
        );
    }

    /// Ask first, so this runs where there is a window to ask in. The Drafts pane's delete
    /// path and the composer's own Discard control both come through here.
    pub(super) fn confirm_discard(&mut self, id: DraftId, window: &mut Window, cx: &mut Context<Self>) {
        let subject = self
            .composer
            .record(id)
            .map(|r| r.subject.clone())
            .unwrap_or_default();
        let title = if subject.trim().is_empty() {
            "Discard this draft?".to_owned()
        } else {
            format!("Discard “{}”?", tabs::title(&subject, 40))
        };
        let options = vec![
            DialogOption::new("1", "Discard", "Delete it, attachments included"),
            DialogOption::new("2", "Keep", "Close the composer and keep the draft"),
        ];
        self.open_dialog(
            DialogSpec::new(title, "This cannot be undone.".to_owned(), options, 1),
            move |this, ix, _window, cx| {
                if ix == 0 {
                    this.discard_draft(id, cx);
                } else {
                    this.close_composer(id);
                }
            },
            window,
            cx,
        );
    }

    /// The only path that deletes a draft.
    pub(super) fn discard_draft(&mut self, id: DraftId, cx: &mut Context<Self>) {
        let Some(open) = self.composer.get_mut(id) else {
            return;
        };
        if self.hold_after_save.contains(&id)
            || !matches!(
                open.record.state,
                DraftState::Editing | DraftState::Failed { .. } | DraftState::Conflict
            )
        {
            return;
        }
        open.operation = Some(DraftOperation::Discard);
        self.pending_attachment_ops.remove(&id);
        self.pending_resolutions.remove(&id);
        self.set_composer_sending(id, true, cx);
        self.sync_composer_status(id, cx);
        if let Some(worker) = self.composer_worker.as_ref() {
            worker.send(Command::Discard(id));
        }
        self.composer.holds.remove(&id);
        cx.notify();
    }

    /// Stop showing a draft that is gone, handing the keyboard back if the surface that
    /// held it has just vanished: list keys work straight after a send instead of being
    /// swallowed by an input that no longer exists. An open modal keeps the focus it owns.
    fn retire_draft(&mut self, id: DraftId, cx: &mut Context<Self>) {
        let shown = self
            .composer
            .get(id)
            .is_some_and(|open| open.inline)
            || self.tabs.active_draft() == Some(id);
        self.forget_draft(id);
        if shown && !self.modal_open() {
            // Queued, not immediate: taking the countdown toast down is window work too, and
            // it runs after this and would otherwise leave the vanished draft holding keys.
            self.pending_ui.push_back(PendingUi::FocusApp);
            cx.notify();
        }
    }

    /// Stop showing a draft that was sent, discarded or confirmed by a reconciliation.
    fn forget_draft(&mut self, id: DraftId) {
        self.pending_attachment_ops.remove(&id);
        self.hold_after_save.remove(&id);
        self.pending_resolutions.remove(&id);
        self.composer.remove(id);
        self.composers.remove(&id);
        if let Some(ix) = self.tabs_index_of_draft(id) {
            self.tabs.close(ix);
        }
    }

    // ------------------------------------------------------------------- the drafts list

    /// Reopen a stored draft, from the record the store already holds. This is the Drafts
    /// pane's Enter/click path.
    ///
    /// A reply whose original is still in the mailbox reopens where it was written: under
    /// that conversation, inline, as if `r` had been pressed there. Everything else (a
    /// forward, a new message, a reply whose original is gone) opens in its own tab.
    pub(super) fn reopen_draft(&mut self, id: DraftId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(record) = self.composer.record(id).cloned() else {
            return;
        };
        // Reopening a draft puts the caret in it, so the reader is the pane the user is now
        // in: the tab keys (`cmd-w`, `ctrl-tab`) follow them there, as with any opened mail.
        self.focus_pane(Pane::Reader);
        let original = matches!(record.kind, DraftKind::Reply | DraftKind::ReplyAll)
            .then_some(record.source_message)
            .flatten()
            .filter(|source| self.mailbox.get(*source).is_some());
        if let Some(source) = original {
            if self.composers.contains_key(&id) {
                self.sync_composer_record(id, cx);
            } else {
                self.build_composer(
                    record,
                    PendingOpen {
                        placement: Placement::Inline,
                        focus: Focus::Body,
                        ..Default::default()
                    },
                    window,
                    cx,
                );
            }
            self.show_inline(id, source, window, cx);
            return;
        }
        self.tabs.open_draft_pinned(id);
        if self.composers.contains_key(&id) {
            if let Some(open) = self.composer.get_mut(id) {
                open.inline = false;
            }
            if let Some(entity) = self.composers.get(&id).cloned() {
                entity.update(cx, |composer, cx| {
                    composer.set_placement(Placement::Tab, cx)
                });
            }
            // Already open somewhere: just bring the same entity forward, so nothing typed
            // is re-created.
            self.sync_composer_record(id, cx);
            if let Some(entity) = self.composers.get(&id).cloned() {
                let body = entity.read(cx).body_focus(cx);
                window.focus(&body, cx);
            }
            cx.notify();
            return;
        }
        self.build_composer(
            record,
            PendingOpen {
                placement: Placement::Tab,
                focus: Focus::Body,
                ..Default::default()
            },
            window,
            cx,
        );
    }

    // ---------------------------------------------------------- conflict resolution

    /// A draft whose provider copy moved on is a choice, not a dead end: keep what is typed
    /// here, take the server's version, or keep both as two drafts. Nothing is overwritten
    /// before the user picks, and the worker does the actual settling — the UI only asks.
    fn confirm_resolution(&mut self, id: DraftId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(record) = self.composer.record(id).cloned() else {
            return;
        };
        if record.remote_id.is_none() {
            self.show_toast("This draft has no copy on the server".into(), window, cx);
            return;
        }
        let subject = tabs::title(&record.subject, 40);
        let title = format!("“{subject}” changed on the server");
        let options = vec![
            DialogOption::new("1", "Keep mine", "Send what is on this Mac"),
            DialogOption::new(
                "2",
                "Use theirs",
                "Take the server's version, rich formatting included",
            ),
            DialogOption::new("3", "Keep both", "Keep mine as a separate draft"),
            DialogOption::new("4", "Cancel", "Leave both alone for now"),
        ];
        self.open_dialog(
            DialogSpec::new(
                title,
                "Nothing is replaced until you choose.".to_owned(),
                options,
                3,
            ),
            move |this, ix, _, cx| {
                let mode = match ix {
                    0 => ResolveMode::KeepLocal,
                    1 => ResolveMode::TakeRemote,
                    2 => ResolveMode::KeepBoth,
                    _ => return,
                };
                this.resolve_conflict(id, mode, cx);
            },
            window,
            cx,
        );
    }

    /// Ask the worker to settle one conflict. The worker re-reads the remote first, so the
    /// revision it acts on is the current one rather than the one that produced the conflict.
    fn resolve_conflict(&mut self, id: DraftId, mode: ResolveMode, cx: &mut Context<Self>) {
        if self.pending_resolutions.contains_key(&id) {
            return;
        }
        self.save_from_editor(id, cx);
        self.pending_resolutions.insert(id, mode);
        self.set_composer_sending(id, true, cx);
        self.flush_resolution(id);
        self.sync_composer_status(id, cx);
        cx.notify();
    }

    fn flush_resolution(&mut self, id: DraftId) {
        let Some(open) = self.composer.get_mut(id) else {
            return;
        };
        if open.in_flight
            || open.has_pending_write()
            || open.operation.is_some()
            || self.pending_attachment_ops.contains_key(&id)
        {
            return;
        }
        let Some(worker) = self.composer_worker.as_ref() else {
            return;
        };
        let Some(mode) = self.pending_resolutions.remove(&id) else {
            return;
        };
        open.operation = Some(DraftOperation::Resolve);
        worker.send(Command::Resolve {
            draft: id,
            account: open.record.account.clone(),
            mode,
        });
    }

    /// The worker settled it: adopt the record it settled on.
    ///
    /// "Use theirs" is the one mode that replaces the local body on purpose, so it takes
    /// the whole record. "Keep mine" settled the content that was already here, so only
    /// the store's identity is taken and anything typed since is kept — and whatever the
    /// provider ended up holding decides whether that content still needs mirroring: a
    /// keystroke that landed while the resolution was out was never on the server, and
    /// calling it mirrored would leave it there for good.
    fn resolution_applied(
        &mut self,
        draft: DraftRecord,
        mode: ResolveMode,
        cx: &mut Context<Self>,
    ) {
        let id = draft.id;
        if matches!(mode, ResolveMode::TakeRemote) {
            self.composer.put(draft);
        } else if let Some(open) = self.composer.get_mut(id) {
            open.absorb_metadata(draft);
        }
        let now = self.now();
        if let Some(open) = self.composer.get_mut(id) {
            open.operation = None;
            open.remote_dirty =
                open.record.synced_content_revision != Some(open.record.content_revision);
            open.status = if open.remote_dirty {
                SaveStatus::Local
            } else {
                SaveStatus::Synced
            };
            open.dirty_since = open.remote_dirty.then_some(now);
        }
        self.set_composer_sending(id, false, cx);
        self.sync_composer_record(id, cx);
        cx.notify();
    }

    /// Keep both produced two records: the bound draft now holds the server's version and
    /// the local copy is a new draft of its own.
    fn resolution_split(&mut self, bound: DraftRecord, copy: DraftRecord, cx: &mut Context<Self>) {
        self.set_composer_sending(bound.id, false, cx);
        let id = bound.id;
        self.composer.put(bound);
        let now = self.now();
        if let Some(open) = self.composer.get_mut(id) {
            open.operation = None;
            // What the provider acknowledged is the server's own copy, so this only holds
            // when nothing local is left waiting to be mirrored.
            open.remote_dirty =
                open.record.synced_content_revision != Some(open.record.content_revision);
            open.status = if open.remote_dirty {
                SaveStatus::Local
            } else {
                SaveStatus::Synced
            };
            open.dirty_since = open.remote_dirty.then_some(now);
        }
        self.sync_composer_record(id, cx);
        self.pending_ui.push_back(PendingUi::Build(
            Box::new(copy),
            PendingOpen {
                placement: Placement::Tab,
                focus: Focus::Body,
                ..Default::default()
            },
        ));
        cx.notify();
    }

    // ------------------------------------------------------------------- imports

    /// Pull the provider's drafts in, so a draft written elsewhere is here to finish.
    pub(super) fn import_provider_drafts(&mut self, account: &str, cx: &mut Context<Self>) {
        if let Some(worker) = self.composer_worker.as_ref() {
            worker.send(Command::Import {
                account: account.to_owned(),
                limit: IMPORT_LIMIT,
            });
        }
        cx.notify();
    }

    /// A provider draft we already hold: the local copy is kept untouched and the change is
    /// shown as a conflict rather than overwritten.
    ///
    /// The account is part of the match: remote ids are only unique within the account that
    /// issued them, so the same id from two accounts names two different drafts.
    fn refresh_imported(&mut self, account: &str, remote: RemoteDraft, cx: &mut Context<Self>) {
        let found = self.composer.drafts.iter().find_map(|d| {
            (d.record.account == account
                && d.record.remote_id.as_deref() == Some(remote.id.as_str()))
            .then_some(d.record.id)
        });
        let Some(id) = found else { return };
        if self
            .composer
            .record(id)
            .and_then(|r| r.remote_revision.clone())
            == Some(remote.revision.clone())
        {
            return;
        }
        if let Some(open) = self.composer.get_mut(id) {
            open.status = SaveStatus::Conflict {
                remote_revision: remote.revision.clone(),
            };
            open.record.state = DraftState::Conflict;
        }
        self.sync_composer_record(id, cx);
        self.say(
            format!(
                "“{}” changed on the server — your copy was kept",
                tabs::title(&remote.subject, 40)
            ),
            cx,
        );
    }

    // --------------------------------------------------------------------- toasts

    /// The toast shown after a startup recovery, naming what came back.
    fn show_recovered(&mut self, draft: DraftId, window: &mut Window, cx: &mut Context<Self>) {
        let subject = self
            .composer
            .record(draft)
            .map(|r| tabs::title(&r.subject, 40))
            .unwrap_or_else(|| "(no subject)".to_owned());
        self.toast = Some(SharedString::from("Draft restored"));
        window.push_notification(
            Notification::info(format!("Restored “{subject}”"))
                .id::<RecoveryToastId>()
                .autohide(true),
            cx,
        );
    }
}
