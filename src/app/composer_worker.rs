//! The composer's blocking half: one background worker that owns the durable draft store,
//! the attachment bytes and the provider's compose handle.
//!
//! The UI thread never touches any of them. It sends [`Command`]s and receives [`Event`]s;
//! every command is answered exactly once, and every answer names the draft it is about, so
//! a late answer for a draft that has since been closed is dropped rather than
//! resurrecting it.
//!
//! Five rules from the send path matter enough to state here:
//!
//! * the provider's copy of the draft is put on the server *before* a send is attempted, so
//!   the send endpoint is only ever asked to deliver bytes the provider already holds;
//! * the revision this client persisted is the token every update is checked against, never
//!   the one it just fetched — a draft that moved on is refused, not overwritten;
//! * a draft the provider still holds, imported rich and never edited, is sent **as the
//!   provider holds it** — its own bytes, its own `Message-ID`. Rebuilding MIME from our
//!   Markdown would throw away inline images and styling the conversion cannot express,
//!   and reconciling on a `Message-ID` that is not in the bytes we sent reconciles nothing;
//! * a send request is persisted as attempted *before* it leaves, so a crash in flight is
//!   recovered as uncertain (reconcilable by `Message-ID`) instead of as a plain retryable
//!   draft that could be sent twice;
//! * nothing on the server is ever deleted to settle a disagreement. A conflict is resolved
//!   by choosing, explicitly, what the local draft becomes.
//! * a local store that cannot write does not un-send anything. The provider's answer
//!   is the truth about delivery, so a confirmed send is still reported as one and an
//!   attempt that may have gone out is still reported as uncertain; the write that
//!   failed is reported beside that, never in place of it, and the row left in the
//!   store keeps the attempt a restart reconciles instead of resending.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use crate::clock::Timestamp;
use crate::outgoing::draft::new_message_id;
use crate::outgoing::mime::{OutgoingMessage, build_mime};
use crate::outgoing::store::{DraftError, DraftStore, Recovery};
use crate::outgoing::{
    AttachmentId, AttachmentSource, AttachmentStore, DraftBody, DraftId, DraftRecord, DraftState,
    ManagedAttachment, PendingHold, keep_hashes,
};
use crate::provider::compose::{
    ComposeError, ComposeProvider, OutgoingMail, RemoteDraft, SendStatus, SentMail, Threading,
};
use crate::provider::mime::{ParsedMessage, bare_message_id, header, parse_message};
use crate::provider::{MailProvider, ProviderError, RemoteId};

use super::composer_local::{Lane, LocalLane, Signals, lane_of};

/// The blocking state the worker owns.
pub struct Stores {
    pub drafts: DraftStore,
    pub attachments: AttachmentStore,
}

impl Stores {
    /// Both stores with nothing durable behind them: an in-memory draft table and a
    /// throwaway attachment directory. Headless tests drive the real worker through this,
    /// so they exercise the same code without writing to the user's disk.
    pub fn open_in_memory() -> Result<Self, String> {
        let drafts = DraftStore::open_in_memory().map_err(|e| e.to_string())?;
        let root = Self::scratch_root("memory");
        let _ = std::fs::remove_dir_all(&root);
        let attachments =
            AttachmentStore::open(&root).map_err(|e| format!("attachment store: {e}"))?;
        Ok(Self {
            drafts,
            attachments,
        })
    }

    /// A unique throwaway directory, so two tests never share one.
    fn scratch_root(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("mail-drafts-{tag}-{}-{unique}", std::process::id()))
    }

    /// Open both stores under `root`, creating them if they are not there yet.
    pub fn open(root: &std::path::Path) -> Result<Self, String> {
        let drafts = DraftStore::open(&root.join("drafts.sqlite")).map_err(|e| e.to_string())?;
        let attachments = AttachmentStore::open(&root.join("attachments"))
            .map_err(|e| format!("attachment store: {e}"))?;
        Ok(Self {
            drafts,
            attachments,
        })
    }
}

/// One account's provider, shared with the sync loop.
pub type ComposeHandle = Arc<Mutex<Box<dyn MailProvider>>>;

/// A provider plus the address it sends from. The account *id* is not an address
/// (`personal`, `gmail:…`), so the `From:` header needs the configured one.
#[derive(Clone)]
pub struct ComposeAccount {
    pub backend: ComposeBackend,
    /// The address this account sends from, as configured.
    pub email: String,
}

/// Where composing actually happens: a full mailbox backend that also composes, or a
/// compose-only backend (the deterministic mock) that has no mailbox side at all.
#[derive(Clone)]
pub enum ComposeBackend {
    /// A `MailProvider` that also implements `ComposeProvider`.
    Provider(ComposeHandle),
    /// A compose-only backend, held in the worker.
    ComposeOnly(Arc<Mutex<Box<dyn ComposeProvider>>>),
}

impl ComposeAccount {
    /// Run `f` against this account's compose handle, or `None` when the backend cannot
    /// compose. A `None` is reported honestly, so the draft stays local instead of the
    /// send pretending to have happened.
    pub fn with_compose<T>(&self, f: impl FnOnce(&mut dyn ComposeProvider) -> T) -> Option<T> {
        match &self.backend {
            ComposeBackend::Provider(handle) => {
                let mut guard = handle.lock().ok()?;
                let compose = guard.as_mut().compose()?;
                Some(f(compose))
            }
            ComposeBackend::ComposeOnly(handle) => {
                let mut guard = handle.lock().ok()?;
                Some(f(guard.as_mut()))
            }
        }
    }

    /// A mailbox backend that also composes.
    pub fn remote(handle: ComposeHandle, email: String) -> Self {
        Self {
            backend: ComposeBackend::Provider(handle),
            email,
        }
    }

    /// A deterministic in-memory backend for an account that has no mailbox of its own.
    /// It really creates, updates and sends drafts — it is not a stand-in for success.
    pub fn mock(now: Timestamp, email: String) -> Self {
        let compose: Box<dyn ComposeProvider> =
            Box::new(crate::provider::mock::MockCompose::new(now));
        Self {
            backend: ComposeBackend::ComposeOnly(Arc::new(Mutex::new(compose))),
            email,
        }
    }
}

/// Account id → provider and sender address.
pub type Accounts = HashMap<String, ComposeAccount>;

/// How the user chose to settle a draft whose provider copy moved on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolveMode {
    /// Keep what is on this Mac. The revision just read is adopted into the record first,
    /// so the write that follows is the explicit one the user asked for and not a fight
    /// with a token of our own making. Body, attachments and reply threading all stay.
    KeepLocal,
    /// Take the server's version: the remote is read whole — rich original HTML and every
    /// attachment — and replaces the *same* local draft, so the open surface stays put.
    TakeRemote,
    /// Keep both: the local record becomes a new, unambiguously local draft with a fresh
    /// id and a fresh `Message-ID`, and the bound draft takes the server's content.
    KeepBoth,
}

/// An imported provider draft that could not become a local one. The remote draft is
/// untouched, and nothing it carries is silently dropped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportFailure {
    /// Set once the draft was stored but something about it still went wrong.
    pub draft: Option<DraftId>,
    pub remote: RemoteId,
    pub detail: String,
}

/// A job for the worker.
#[derive(Debug)]
pub enum Command {
    /// Give a new draft its durable identity, so what the user edits is already stored.
    Create { draft: Box<DraftRecord> },
    /// Save a draft; `expected` is the revision the UI last saw, so a stale save is
    /// refused instead of overwriting a newer copy.
    Save {
        draft: Box<DraftRecord>,
        expected: u64,
    },
    /// Delete a draft, its hold and any bytes no other draft needs.
    Discard(DraftId),
    /// Every stored draft, for startup and the drafts list.
    List,
    /// Recover interrupted sends and interrupted holds after a restart.
    Recover { now: Timestamp },
    /// Put a draft into its send hold.
    BeginHold {
        draft: DraftId,
        now: Timestamp,
        /// Send-later: hold until then instead of the ten-second undo window.
        at: Option<Timestamp>,
    },
    /// Take a draft back out of its hold.
    CancelHold(DraftId),
    /// Copy files into the draft's managed attachments.
    Attach { draft: DraftId, files: Vec<PathBuf> },
    /// Fetch an original message's files and carry them into a forward.
    ForwardAttachments {
        draft: DraftId,
        message: RemoteId,
        account: String,
    },
    /// Drop one attachment from the draft.
    Detach {
        draft: DraftId,
        attachment: AttachmentId,
    },
    /// Mirror the draft to the provider's draft store.
    Sync { draft: DraftId },
    /// Settle a conflict the way the user chose. Never deletes anything on the server.
    Resolve {
        draft: DraftId,
        account: String,
        mode: ResolveMode,
    },
    /// Import the provider's drafts that are not already local.
    Import { account: String, limit: usize },
    /// Adopt one provider draft as a local one, its rich original preserved.
    ImportOne { account: String, remote: RemoteId },
    /// Send a draft, now or when its hold runs out.
    Send {
        draft: DraftId,
        account: String,
        now: Timestamp,
    },
    /// Ask the provider whether an uncertain send was in fact delivered.
    Reconcile { draft: DraftId, account: String },
    /// Can this account's provider compose at all?
    Probe { account: String },
    /// The RFC threading headers of a message, for a reply.
    Threading { message: RemoteId, account: String },
}

/// What the worker reports back.
#[derive(Debug)]
pub enum Event {
    /// A new draft now has its identity and is safe to edit.
    Created(Box<DraftRecord>),
    Saved(Box<DraftRecord>),
    /// A save lost a local revision race. The stored copy is handed back so the UI can
    /// keep what the user typed and show the difference; nothing is overwritten here.
    SaveRejected(Box<DraftRecord>),
    /// A draft was mirrored to the provider; `remote` is the provider's copy.
    Synced(Box<DraftRecord>, Box<RemoteDraft>),
    /// The provider's draft moved on. The local copy is kept untouched.
    Conflicted(Box<DraftRecord>, Box<RemoteDraft>),
    /// A conflict the user settled. `draft` is what the local draft is now; the server's
    /// copy is untouched either way.
    Resolved {
        draft: Box<DraftRecord>,
        mode: ResolveMode,
    },
    /// [`ResolveMode::KeepBoth`]: `bound` keeps the id the user was looking at and now
    /// carries the server's content, `copy` is the new local-only draft to open beside it.
    ResolvedBoth {
        bound: Box<DraftRecord>,
        copy: Box<DraftRecord>,
    },
    /// A resolution could not be carried out. The local draft is exactly as it was.
    ResolveFailed(DraftId, String),
    Discarded(DraftId),
    Listed(Vec<DraftRecord>),
    /// Recovery after a restart: what was restored, what is uncertain, and every draft.
    Recovered(Box<Recovery>, Vec<DraftRecord>),
    Held(PendingHold),
    HoldCancelled(Box<DraftRecord>),
    Attached {
        draft: Box<DraftRecord>,
        failed: Vec<String>,
    },
    Forwarded {
        draft: Box<DraftRecord>,
        failed: Vec<String>,
    },
    Detached(Box<DraftRecord>),
    /// Imported drafts, the provider's drafts already held locally, and the ones that
    /// could not be imported in full. `account` names whose drafts these are: a remote
    /// id is scoped to one account, so two accounts may hold the very same id and every
    /// consumer has to tell them apart.
    Imported {
        account: String,
        fresh: Vec<DraftRecord>,
        already_local: Vec<RemoteDraft>,
        failed: Vec<ImportFailure>,
    },
    /// The provider confirmed delivery.
    Sent {
        draft: Box<DraftRecord>,
        sent: Box<SentMail>,
        message: Box<OutgoingMessage>,
    },
    /// The request may or may not have been delivered: reconcile before retrying.
    Uncertain(Box<DraftRecord>, String),
    /// The request provably never left the machine, so nothing was applied: the draft is
    /// exactly as it was, editable and unscheduled.
    Offline(Box<DraftRecord>),
    /// The operation was refused with nothing applied; the draft is editable with the
    /// reason attached.
    SendFailed(Box<DraftRecord>, String),
    /// A save, a listing or a read failed.
    ///
    /// Never the *only* answer to a send: whatever the provider settled on is reported
    /// first, and this follows it, so a local write that failed never stands in for a
    /// delivery, a delivery in doubt, or a proven non-send.
    StoreFailed(Option<DraftId>, String),
    /// A background job that names no particular draft failed.
    JobFailed(String),
    /// The draft asked for is not stored here any more.
    Missing(DraftId),
    /// The provider could not confirm an uncertain attempt, so the draft stays uncertain
    /// and this says why. A *confirmed* send is not reported here: it is an ordinary
    /// [`Event::Sent`], with the provider's own record of the message, so the outbox and
    /// the mailbox are built from the same thing a directly observed send is.
    Reconciled {
        draft: Box<DraftRecord>,
        detail: String,
    },
    /// A draft write to the provider did not complete. No mail was sent, so this is never
    /// a delivery in doubt: the draft stays locally editable and the next import shows
    /// what the server actually holds.
    AutosaveFailed(DraftId, String),
    /// This account's backend cannot compose; the draft stays local and usable.
    Unsupported {
        account: String,
    },
    /// This account can compose.
    Composable {
        account: String,
    },
    /// The RFC threading of a message, for a reply's headers. `account` is part of the
    /// identity: a message id is scoped to one account, so a request for a reply is
    /// answered per account, never merged into a lookup keyed by the id alone.
    Threaded {
        account: String,
        message: RemoteId,
        threading: Threading,
    },
}

impl Event {
    /// The draft this event is about, when it is about one.
    pub fn draft_id(&self) -> Option<DraftId> {
        match self {
            Self::Created(d)
            | Self::Saved(d)
            | Self::SaveRejected(d)
            | Self::Synced(d, _)
            | Self::Conflicted(d, _)
            | Self::Detached(d)
            | Self::HoldCancelled(d)
            | Self::Attached { draft: d, .. }
            | Self::Forwarded { draft: d, .. }
            | Self::Sent { draft: d, .. }
            | Self::Uncertain(d, _)
            | Self::Offline(d)
            | Self::SendFailed(d, _) => Some(d.id),
            Self::Resolved { draft, .. } => Some(draft.id),
            Self::ResolvedBoth { bound, .. } => Some(bound.id),
            Self::Reconciled { draft, .. } => Some(draft.id),
            Self::Held(hold) => Some(hold.draft),
            Self::Discarded(id)
            | Self::Missing(id)
            | Self::ResolveFailed(id, _)
            | Self::AutosaveFailed(id, _) => Some(*id),
            Self::StoreFailed(draft, _) => *draft,
            Self::Listed(_)
            | Self::Recovered(..)
            | Self::Imported { .. }
            | Self::Unsupported { .. }
            | Self::Composable { .. }
            | Self::Threaded { .. }
            | Self::JobFailed(_) => None,
        }
    }
}

/// The UI-thread end of the worker: commands go out, events come back.
///
/// Two lanes answer here. Commands that are only the draft table go to the local
/// persistence lane, so a keystroke's write is never queued behind a provider call or a
/// file copy; everything that talks to a provider or moves bytes stays on the serial
/// worker. Both lanes answer on this one event queue and wake the UI the same way.
pub struct Client {
    /// Provider and filesystem work, serial on purpose.
    tx: Sender<Command>,
    /// Pure local persistence: short SQLite transactions, never blocked by the network.
    local_tx: Sender<Command>,
    rx: Receiver<Event>,
    accounts: Arc<Mutex<Accounts>>,
}

impl Client {
    /// Start both lanes over `stores` and the given provider handles.
    ///
    /// The two lanes share one draft store and one event queue, so they interleave freely:
    /// a `Saved` that lands while a sync is still out is already queued by the time the UI
    /// drains.
    pub fn start(stores: Stores, accounts: Accounts) -> Self {
        let (tx, command_rx) = channel::<Command>();
        let (local_tx, local_rx) = channel::<Command>();
        let (signals, event_rx) = Signals::new();
        let accounts = Arc::new(Mutex::new(accounts));
        let worker_accounts = Arc::clone(&accounts);
        let worker_signals = signals.clone();
        // The draft store is one connection behind an internal lock, and every method
        // takes it for one short transaction — so a clone is the same store, not a second
        // one, and the local lane writes while the worker is blocked on a provider.
        let local = LocalLane::new(stores.drafts.clone(), signals);
        std::thread::Builder::new()
            .name("composer-drafts".to_owned())
            .spawn(move || local.run(local_rx))
            .expect("spawn the composer draft lane");
        std::thread::Builder::new()
            .name("composer".to_owned())
            .spawn(move || {
                Worker {
                    stores,
                    accounts: worker_accounts,
                    signals: worker_signals,
                }
                .run(command_rx)
            })
            .expect("spawn the composer worker");
        Self {
            tx,
            local_tx,
            rx: event_rx,
            accounts,
        }
    }

    /// Ask for something to be done, on the lane that owns it.
    pub fn send(&self, command: Command) {
        let tx = match lane_of(&command) {
            Lane::Local => &self.local_tx,
            Lane::Work => &self.tx,
        };
        // A closed worker means the app is shutting down; there is nothing left to save to.
        let _ = tx.send(command);
    }

    /// Take the next finished job's events, if any. The UI calls this on its beat, so it
    /// has to be cheap.
    pub fn drain(&self) -> Vec<Event> {
        self.rx.try_iter().collect()
    }

    /// Replace the whole registry in one step. A register/forget pair can miss an account
    /// whose provider went away between two reads, and a stale handle that stays behind
    /// answers commands for an account the user has signed out of; replacing the map means
    /// anything not in `accounts` is simply gone.
    ///
    /// A compose-only backend keeps the one already registered when the account is still
    /// compose-only and still sends from the same address. It owns state nothing else can
    /// rebuild — a deterministic mock's drafts and its sent mail, which an uncertain send
    /// is still reconciled against — so handing out a fresh one on every refresh would
    /// quietly lose both. Real provider handles are replaced with the new ones.
    pub fn update_accounts(&self, accounts: Accounts) {
        let Ok(mut current) = self.accounts.lock() else {
            return;
        };
        let mut next = Accounts::with_capacity(accounts.len());
        for (id, account) in accounts {
            let keep_compose_only = matches!(
                (current.get(&id), &account.backend),
                (
                    Some(old),
                    ComposeBackend::ComposeOnly(_)
                ) if old.email == account.email
                    && matches!(&old.backend, ComposeBackend::ComposeOnly(_))
            );
            let backend = if keep_compose_only {
                current
                    .get(&id)
                    .map(|old| old.backend.clone())
                    .unwrap_or(account.backend.clone())
            } else {
                account.backend.clone()
            };
            next.insert(
                id,
                ComposeAccount {
                    backend,
                    email: account.email,
                },
            );
        }
        *current = next;
    }

    /// Register a provider after a sign-in, together with the address it sends from.
    pub fn register(&self, account: &str, provider: ComposeAccount) {
        if let Ok(mut accounts) = self.accounts.lock() {
            accounts.insert(account.to_owned(), provider);
        }
    }

    pub fn forget(&self, account: &str) {
        if let Ok(mut accounts) = self.accounts.lock() {
            accounts.remove(account);
        }
    }

    /// Whether a provider for `account` is registered at all.
    pub fn has_account(&self, account: &str) -> bool {
        self.accounts
            .lock()
            .is_ok_and(|accounts| accounts.contains_key(account))
    }
}

/// The worker thread: provider calls and filesystem work, one job at a time.
struct Worker {
    stores: Stores,
    accounts: Arc<Mutex<Accounts>>,
    signals: Signals,
}

/// What one provider call produced. Every provider call in the worker goes through
/// [`Worker::compose`], so "this account cannot compose" and "the provider refused" are
/// never confused with each other.
enum Attempt<T> {
    Ok(T),
    /// The account has no compose backend at all.
    NoBackend,
    Failed(ComposeError),
}

/// What a provider call produced, before any draft state is written. Kept separate from
/// [`Event`] so the mapping is written once and the state write happens once.
enum Outcome {
    Sent(SentMail),
    /// A send may or may not have reached recipients: reconcile by `Message-ID` before
    /// anything is retried.
    Uncertain(String),
    /// An interrupted *draft* save. No mail was sent, so this is never reconciled; the
    /// draft stays locally editable and the next import decides what landed.
    AutosaveUncertain(String),
    /// Provably never left the machine: safe to try again later.
    Offline,
    /// Refused, with nothing applied.
    Refused(String),
}

impl Outcome {
    /// Map a provider error onto what the send path must do about it.
    ///
    /// `needs_send_reconciliation` is deliberately narrower than `is_uncertain`: only a
    /// send that may have gone out needs `find_sent`, so a failed autosave can never be
    /// shown to the user as "this may have been delivered".
    fn from_error(error: ComposeError) -> Self {
        match &error {
            ComposeError::Offline => return Self::Offline,
            ComposeError::AutosaveUncertain { detail } => {
                return Self::AutosaveUncertain(detail.clone());
            }
            _ if error.needs_send_reconciliation() => {
                return Self::Uncertain(error.to_string());
            }
            _ => {}
        }
        Self::Refused(error.to_string())
    }
}

/// What the provider settled on, in the words that must survive a failed local write.
/// These lead the paired [`Event::StoreFailed`], so the last thing a one-line surface says
/// is never a bare storage error standing in for a delivery, a delivery in doubt, or a
/// proven non-send.
const SENT_TRUTH: &str = "the provider confirmed this was sent";
const UNCERTAIN_TRUTH: &str =
    "this may or may not have been delivered, so it will be checked rather than sent again";
const NOTHING_SENT_TRUTH: &str = "nothing was sent";

/// Why a draft could not be put on the server.
enum PublishError {
    /// The provider's copy moved on; the local copy is kept as it is.
    Moved(Box<RemoteDraft>),
    /// A draft this one is bound to is no longer on the server. It may have been sent
    /// from the web, so nothing may be created or sent on the strength of that alone.
    Vanished(RemoteId),
    NoBackend(String),
    Failed(ComposeError),
}

/// One step of a publish, so a refusal is data rather than a control-flow escape.
enum Step {
    Created(RemoteDraft),
    Updated(RemoteDraft),
    /// The draft on the server is not the one this client last saw.
    Moved(RemoteDraft),
    /// A bound draft is no longer on the server: not written to, and not replaced.
    Vanished(RemoteId),
}

/// What a reconciliation found. The provider's own [`SentMail`] is kept whole: a
/// confirmed send has to carry the id, thread, time and RFC `Message-ID` the server
/// recorded, because that is what the mailbox and the outbox are built from.
enum Lookup {
    Found(SentMail),
    /// Nothing with that `Message-ID` — which is not proof it was never sent.
    NotFound,
    /// The check itself failed, and must be retried rather than read as an answer.
    Failed(String),
}

/// What binding a provider draft onto a local row achieved.
enum Bind {
    /// The row as it stands, now carrying that binding.
    Stored(Box<DraftRecord>),
    /// The row belongs to a different account now, so nothing was written. The provider
    /// copy still exists where it was put; it is simply not this draft's any more.
    MovedAccount,
}

/// A file of a remote message, with its bytes already in hand.
struct RemoteFile {
    name: String,
    content_type: String,
    part: String,
    bytes: Vec<u8>,
    /// `Content-ID` of an inline part, without angle brackets: the id the body refers to
    /// with `cid:`. It is carried onto the managed copy so sending it keeps that
    /// reference working instead of showing a broken image.
    content_id: Option<String>,
}

/// Everything an import reads out of one remote draft.
struct Gathered {
    /// Whether the stored bytes were actually read. A provider draft may be perfectly
    /// valid and *empty*; what may not be invented is content nobody could read.
    decoded: bool,
    html: Option<String>,
    text: String,
    files: Vec<RemoteFile>,
    message_id: String,
    references: Vec<String>,
    in_reply_to: Option<String>,
}

impl Worker {
    /// Answer work-lane commands until the channel closes. Each job's events are queued
    /// before the UI is woken, so a woken UI always finds them already there.
    fn run(mut self, commands: Receiver<Command>) {
        for command in commands {
            if matches!(lane_of(&command), Lane::Local) {
                // Routing is decided once, by `lane_of`; a local command here is a bug,
                // and saying so beats silently leaving a keystroke's write unanswered.
                self.signals.publish(vec![Event::JobFailed(
                    "a local draft command reached the provider worker".into(),
                )]);
                continue;
            }
            let events = self.handle(command);
            self.signals.publish(events);
        }
    }

    fn handle(&mut self, command: Command) -> Vec<Event> {
        match command {
            Command::Discard(id) => vec![self.discard(id)],
            Command::Attach { draft, files } => vec![self.attach(draft, &files)],
            Command::ForwardAttachments {
                draft,
                message,
                account,
            } => {
                vec![self.forward(draft, message, &account)]
            }
            Command::Detach { draft, attachment } => vec![self.detach(draft, attachment)],
            Command::Sync { draft } => vec![self.sync(draft)],
            Command::Resolve {
                draft,
                account,
                mode,
            } => vec![self.resolve(draft, &account, mode)],
            Command::Import { account, limit } => vec![self.import(&account, limit)],
            Command::ImportOne { account, remote } => vec![self.import_one(&account, remote)],
            Command::Send {
                draft,
                account,
                now,
            } => self.send(draft, &account, now),
            Command::Reconcile { draft, account } => self.reconcile(draft, &account),
            Command::Probe { account } => vec![self.probe(&account)],
            Command::Threading { message, account } => vec![self.threading(message, &account)],
            _ => vec![Event::JobFailed(
                "that command is local persistence, not provider work".into(),
            )],
        }
    }

    /// Run one provider call, distinguishing "cannot compose" from "refused".
    fn compose<T>(
        &self,
        account: &str,
        f: impl FnOnce(&mut dyn ComposeProvider) -> Result<T, ComposeError>,
    ) -> Attempt<T> {
        let Some(handle) = self.account_handle(account) else {
            return Attempt::NoBackend;
        };
        match handle.with_compose(f) {
            Some(Ok(value)) => Attempt::Ok(value),
            Some(Err(error)) => Attempt::Failed(error),
            None => Attempt::NoBackend,
        }
    }

    // --------------------------------------------------------------- the drafts
    //
    // Creating a draft, writing the user's edits, listing, recovering and the send hold
    // are on the local persistence lane: they are short SQLite transactions and nothing
    // else, so a keystroke never waits on a provider. What is left here is the draft row
    // as the *worker* reads it, and the completions that have to land on the row as it
    // stands rather than as it stood when a file copy or a provider call began.

    fn load(&mut self, id: DraftId) -> Result<DraftRecord, Event> {
        match self.stores.drafts.get(id) {
            Ok(Some(draft)) => Ok(draft),
            Ok(None) => Err(Event::Missing(id)),
            Err(e) => Err(self.store_failed(Some(id), e)),
        }
    }

    fn discard(&mut self, id: DraftId) -> Event {
        let draft = match self.load(id) {
            Ok(draft) => draft,
            Err(event) => return event,
        };
        if matches!(
            draft.state,
            DraftState::Held
                | DraftState::Sending
                | DraftState::Uncertain
                | DraftState::Sent { .. }
        ) {
            return Event::StoreFailed(
                Some(id),
                "a held or unconfirmed send cannot be discarded".into(),
            );
        }
        if let Some(remote_id) = draft.remote_id.as_ref() {
            match self.compose(&draft.account, |provider| provider.get_draft(remote_id)) {
                Attempt::Ok(remote) => {
                    if draft.remote_revision.as_deref() != Some(remote.revision.as_str()) {
                        return self.conflicted(&draft, remote);
                    }
                    match self.compose(&draft.account, |provider| provider.delete_draft(remote_id))
                    {
                        Attempt::Ok(()) => {}
                        Attempt::Failed(error) => {
                            return Event::StoreFailed(Some(id), error.to_string());
                        }
                        Attempt::NoBackend => {
                            return Event::StoreFailed(
                                Some(id),
                                "the draft's account cannot reach its provider".into(),
                            );
                        }
                    }
                }
                Attempt::Failed(ComposeError::NotFound(_)) => {}
                Attempt::Failed(error) => return Event::StoreFailed(Some(id), error.to_string()),
                Attempt::NoBackend => {
                    return Event::StoreFailed(
                        Some(id),
                        "the draft's account cannot reach its provider".into(),
                    );
                }
            }
        }
        match self.stores.drafts.delete(id) {
            Ok(_) => {
                self.prune_orphans();
                Event::Discarded(id)
            }
            Err(e) => self.store_failed(Some(id), e),
        }
    }

    /// Delete the bytes of attachments no stored draft references any more. An attachment
    /// may be shared between drafts, and the same file can be forwarded twice.
    fn prune_orphans(&mut self) {
        let Ok(drafts) = self.stores.drafts.list() else {
            return;
        };
        let keep = keep_hashes(drafts.iter().flat_map(|d| d.attachments.iter()));
        let _ = self.stores.attachments.prune(&keep);
    }

    // ------------------------------------------------------------ attachments

    /// Copy the picked files into the draft's managed attachments.
    ///
    /// The copying happens first and holds nothing: reading a large file must not keep the
    /// draft table locked, and the user must be able to keep typing while it happens. The
    /// managed copies are then added to the row *as it is then*, so an edit made during
    /// the copy is still here afterwards.
    fn attach(&mut self, id: DraftId, files: &[PathBuf]) -> Event {
        let mut managed = Vec::new();
        let mut failed = Vec::new();
        for file in files {
            // Copy, never move: moving the picked file would break the user's own file
            // and leave a draft whose attachment no longer exists.
            match self.stores.attachments.import_file(file) {
                Ok(stored) => managed.push(stored),
                Err(e) => failed.push(format!("{}: {e}", file.display())),
            }
        }
        match self.add_attachments(id, managed) {
            Ok(stored) => Event::Attached {
                draft: Box::new(stored),
                failed,
            },
            Err(event) => event,
        }
    }

    fn detach(&mut self, id: DraftId, attachment: AttachmentId) -> Event {
        // A detachment is a fact about the file list, so it is applied to the row as it
        // stands: nothing the user typed while this command waited can be lost to it.
        let mut removed = false;
        let stored = match self.stores.drafts.update_current(id, |row| {
            removed = row.detach(&attachment);
            Ok(())
        }) {
            Ok(stored) => stored,
            Err(e) => return self.store_failed(Some(id), e),
        };
        if !removed {
            return Event::Detached(Box::new(stored));
        }
        self.prune_orphans();
        Event::Detached(Box::new(stored))
    }

    /// Bring an original message's files into the draft, so forwarding carries them and
    /// removing one is local afterwards. A file that could not be fetched is reported,
    /// never dropped in silence: the user decides whether to send without it.
    fn forward(&mut self, id: DraftId, message: RemoteId, account: &str) -> Event {
        let (files, mut failed) = match self.compose(account, |compose| {
            let listed = compose.list_attachments(&message)?;
            let mut fetched = Vec::with_capacity(listed.len());
            let mut failed = Vec::new();
            for file in listed {
                match compose.attachment_bytes(&message, &file) {
                    Ok(bytes) => fetched.push(RemoteFile {
                        name: file.name.clone(),
                        part: file.id.clone().unwrap_or_default(),
                        bytes: bytes.bytes,
                        // The bytes the provider served are the authority on what this file
                        // is; a listing that named no type falls back to what the fetch says.
                        content_type: if file.content_type.trim().is_empty() {
                            bytes.content_type
                        } else {
                            file.content_type.clone()
                        },
                        content_id: file.content_id.clone(),
                    }),
                    Err(e) => failed.push(format!("{}: {e}", file.name)),
                }
            }
            Ok::<_, ComposeError>((fetched, failed))
        }) {
            Attempt::Ok(files) => files,
            Attempt::NoBackend => {
                let draft = match self.load(id) {
                    Ok(draft) => draft,
                    Err(event) => return event,
                };
                return Event::Forwarded {
                    draft: Box::new(draft),
                    failed: vec![format!("{account} cannot fetch the original's attachments")],
                };
            }
            Attempt::Failed(error) => {
                let draft = match self.load(id) {
                    Ok(draft) => draft,
                    Err(event) => return event,
                };
                return Event::Forwarded {
                    draft: Box::new(draft),
                    failed: vec![error.to_string()],
                };
            }
        };
        let mut managed = Vec::with_capacity(files.len());
        for file in &files {
            let source = AttachmentSource::Forwarded {
                message: message.clone(),
                part: file.part.clone(),
            };
            match self.managed_copy(
                &file.name,
                &file.bytes,
                &file.content_type,
                source,
                file.content_id.clone(),
            ) {
                Ok(stored) => managed.push(stored),
                Err(detail) => failed.push(detail),
            }
        }
        // Same rule as a picked file: the fetch took as long as it took, and what the user
        // wrote meanwhile is the draft this forward now belongs to.
        match self.add_attachments(id, managed) {
            Ok(stored) => Event::Forwarded {
                draft: Box::new(stored),
                failed,
            },
            Err(event) => event,
        }
    }

    /// Add managed copies to the row as it stands now.
    ///
    /// A copy the draft already carries is not a failure — the same file twice must not be
    /// carried twice — so it is simply left out and the row is not written for it.
    fn add_attachments(
        &mut self,
        id: DraftId,
        managed: Vec<ManagedAttachment>,
    ) -> Result<DraftRecord, Event> {
        self.stores
            .drafts
            .update_current(id, |row| {
                for file in managed {
                    row.attach(file);
                }
                Ok(())
            })
            .map_err(|e| match e {
                DraftError::NotFound(id) => Event::Missing(id),
                other => self.store_failed(Some(id), other),
            })
    }

    /// Store one remote file as a managed copy. An inline part keeps the `Content-ID` its
    /// body refers to; an id that would not survive a header is refused rather than
    /// rewritten, because a body pointing at a `cid:` nothing carries shows a hole.
    ///
    /// The provider's media type is authoritative and is carried verbatim: the bytes came
    /// from that part, so replacing it with a guess from the file name would mislabel
    /// exactly the files whose names lie. Only a provider that named none at all falls back
    /// to the name, which is what guessing is for.
    fn managed_copy(
        &self,
        name: &str,
        bytes: &[u8],
        content_type: &str,
        source: AttachmentSource,
        content_id: Option<String>,
    ) -> Result<ManagedAttachment, String> {
        let named = (!content_type.trim().is_empty()).then_some(content_type);
        let stored = self
            .stores
            .attachments
            .import_bytes(name, bytes, source, named)
            .map_err(|e| format!("{name}: {e}"))?;
        match content_id {
            Some(cid) => stored
                .with_content_id(Some(cid))
                .map_err(|e| format!("{name}: {e}")),
            None => Ok(stored),
        }
    }

    // ---------------------------------------------------------------- providers

    /// The account's provider and sender address, or `None` when it has none.
    fn account_handle(&self, account: &str) -> Option<ComposeAccount> {
        self.accounts
            .lock()
            .ok()
            .and_then(|accounts| accounts.get(account).cloned())
    }

    fn probe(&mut self, account: &str) -> Event {
        match self.compose(account, |_| Ok(())) {
            Attempt::Ok(()) => Event::Composable {
                account: account.to_owned(),
            },
            Attempt::NoBackend | Attempt::Failed(_) => Event::Unsupported {
                account: account.to_owned(),
            },
        }
    }

    fn threading(&mut self, message: RemoteId, account: &str) -> Event {
        match self.compose(account, |compose| compose.threading(&message)) {
            Attempt::Ok(threading) => Event::Threaded {
                account: account.to_owned(),
                message,
                threading,
            },
            Attempt::NoBackend => Event::Unsupported {
                account: account.to_owned(),
            },
            Attempt::Failed(e) => Event::JobFailed(e.to_string()),
        }
    }

    /// Mirror the draft to the provider, creating or replacing its remote draft. A
    /// replacement the provider refuses as stale keeps the local copy and reports the
    /// conflict instead of overwriting whatever moved on.
    fn sync(&mut self, id: DraftId) -> Event {
        let draft = match self.load(id) {
            Ok(draft) => draft,
            Err(event) => return event,
        };
        // A queued mirror follows the durable sender, never a stale UI account snapshot.
        let account = draft.account.as_str();
        let Some(handle) = self.account_handle(account) else {
            // Draft-aware, so whatever the UI has in flight for this draft is settled
            // rather than left waiting on an answer that is not coming.
            return Event::AutosaveFailed(
                id,
                format!(
                    "this account cannot reach a provider any more, so {account} was not saved"
                ),
            );
        };
        let from = handle.email.clone();
        // A draft the provider still holds exactly as it was imported is *not* written
        // over: mirroring it would replace the rich original nobody has approved
        // converting, and nothing local can rebuild what that would throw away.
        if let Some(current) = self.current_remote(&draft, account)
            && self.still_theirs(&draft, &current)
            && self.unchanged_since(&draft, &current)
        {
            // Nothing was uploaded — the server already holds exactly this content — so
            // the binding is recorded against the row as it stands, and the marker says
            // the content that is there now is what the server holds.
            let current_content = draft.content_revision;
            return match self.bind_published(id, account, &current, current_content, false) {
                Ok(Bind::Stored(stored)) => Event::Synced(stored, Box::new(current)),
                Ok(Bind::MovedAccount) => self.moved_account(id),
                Err(e) => self.store_failed(Some(id), e),
            };
        }
        let now = draft.updated_at;
        // No recipient required: an unaddressed draft is a real draft on the server too.
        let mail = match self.mail_for(&draft, &from, now) {
            Ok(mail) => mail,
            Err(reason) => return self.refuse(&draft, &reason),
        };
        let remote = match self.publish(&draft, account, &mail) {
            Ok(remote) => remote,
            Err(PublishError::Moved(remote)) => return self.conflicted(&draft, *remote),
            // The draft this one is bound to is gone from the server. Recreating it here
            // would bury the fact that it disappeared — and if it was sent from the web,
            // bury the duplicate that recreating it invites. The binding stays exactly as
            // it is, so the send path can still reconcile the Message-ID.
            Err(PublishError::Vanished(_)) => {
                return Event::AutosaveFailed(
                    draft.id,
                    "the server no longer holds this draft; it may have been sent from \
                     another device, so it was not recreated"
                        .to_owned(),
                );
            }
            Err(PublishError::NoBackend(account)) => {
                return Event::AutosaveFailed(
                    draft.id,
                    format!(
                        "this account cannot reach a provider any more, so {account} was not saved"
                    ),
                );
            }
            Err(PublishError::Failed(e)) => return self.compose_failed(&draft, e),
        };
        // The snapshot that was uploaded is the one the binding describes. Typing that
        // landed while the provider call was out is *not* on the server, so the marker
        // names the uploaded revision and the row keeps the newer text.
        let uploaded = draft.content_revision;
        match self.bind_published(id, account, &remote, uploaded, false) {
            Ok(Bind::Stored(stored)) => Event::Synced(stored, Box::new(remote)),
            Ok(Bind::MovedAccount) => self.moved_account(id),
            Err(e) => self.store_failed(Some(id), e),
        }
    }

    /// The draft moved to another account while a provider call was out, so its binding was
    /// refused. Nothing is reported as mirrored: the copy on the previous provider is still
    /// there, but it is not this draft's, and the UI must not say the draft was saved.
    fn moved_account(&self, id: DraftId) -> Event {
        Event::AutosaveFailed(
            id,
            "this draft was switched to another account while it was being saved, so the \
             copy on its previous account was not kept as this draft's"
                .to_owned(),
        )
    }

    /// Settle a conflict the way the user chose. The server's draft is never deleted: a
    /// disagreement is settled by choosing what the local draft becomes, and the only
    /// writes are the ones the choice asks for.
    fn resolve(&mut self, id: DraftId, account: &str, mode: ResolveMode) -> Event {
        let draft = match self.load(id) {
            Ok(draft) => draft,
            Err(event) => return event,
        };
        // The conflict is settled against the account the *durable record* names. A request
        // that queued behind a provider call, or behind the user picking a different
        // account, must not read or write another mailbox's draft.
        if draft.account != account {
            return moved_resolution(id);
        }
        let Some(handle) = self.account_handle(account) else {
            return Event::ResolveFailed(
                id,
                format!(
                    "this account cannot reach a provider any more, so {account} could not be read"
                ),
            );
        };
        let Some(remote_id) = draft.remote_id.clone() else {
            return Event::ResolveFailed(id, "this draft is not bound to a provider draft".into());
        };
        // What the provider holds *now*: every mode below decides against this read, not
        // against whatever was on screen when the conflict was noticed.
        let current = match handle.with_compose(|compose| compose.get_draft(&remote_id)) {
            Some(Ok(remote)) => remote,
            Some(Err(e)) => return Event::ResolveFailed(id, e.to_string()),
            None => {
                return Event::ResolveFailed(id, "this account cannot reach its provider".into());
            }
        };
        match mode {
            ResolveMode::KeepLocal => {
                // The revision just read becomes ours, so the write that follows is the
                // user's explicit choice and not a fight with a token of our own making.
                let bound =
                    match self.bind_published(id, account, &current, draft.content_revision, false)
                    {
                        Ok(Bind::Stored(stored)) => *stored,
                        Ok(Bind::MovedAccount) => {
                            return Event::ResolveFailed(
                                id,
                                "this draft was switched to another account, so its conflict \
                                 was not settled against its previous account's provider"
                                    .to_owned(),
                            );
                        }
                        Err(e) => return self.store_failed(Some(id), e),
                    };
                let now = bound.updated_at;
                let mail = match self.mail_for(&bound, &handle.email, now) {
                    Ok(mail) => mail,
                    Err(reason) => return self.refuse(&bound, &reason),
                };
                let uploaded = bound.content_revision;
                // The read above and this write are separate calls, so a draft that moves
                // again in between is still refused rather than overwritten.
                match self.publish(&bound, account, &mail) {
                    Ok(remote) => match self.bind_published(id, account, &remote, uploaded, true) {
                        Ok(Bind::Stored(stored)) => Event::Resolved {
                            draft: stored,
                            mode,
                        },
                        Ok(Bind::MovedAccount) => Event::ResolveFailed(
                            id,
                            "this draft was switched to another account while it was being \
                                 written, so nothing was bound to its previous account's copy"
                                .to_owned(),
                        ),
                        Err(e) => self.store_failed(Some(id), e),
                    },
                    Err(PublishError::Moved(remote)) => self.conflicted(&bound, *remote),
                    Err(PublishError::Vanished(remote)) => Event::ResolveFailed(
                        id,
                        format!("the server draft {remote} disappeared; nothing was overwritten"),
                    ),
                    Err(PublishError::NoBackend(account)) => Event::ResolveFailed(
                        id,
                        format!(
                            "this account cannot reach a provider any more, so {account} could not be written"
                        ),
                    ),
                    Err(PublishError::Failed(e)) => Event::ResolveFailed(id, e.to_string()),
                }
            }
            ResolveMode::TakeRemote => {
                // Read the server's draft whole before touching anything: a body or file
                // that cannot be read leaves the local draft exactly as it was.
                let mut filled = draft.clone();
                if let Err(detail) = self.fill_from_remote(&mut filled, account, &current) {
                    return Event::ResolveFailed(id, detail);
                }
                // Taking the server's copy is the one case where the local body is
                // replaced on purpose, so it is applied to the row as it stands — the UI
                // freezes this composer while the choice is being carried out, and any
                // other write would be a race it must not lose.
                match self.take_remote(id, &filled, &current) {
                    Ok(Bind::Stored(stored)) => {
                        self.prune_orphans();
                        Event::Resolved {
                            draft: stored,
                            mode,
                        }
                    }
                    Ok(Bind::MovedAccount) => moved_resolution(id),
                    Err(e) => self.store_failed(Some(id), e),
                }
            }
            ResolveMode::KeepBoth => {
                let now = draft.updated_at;
                // The remote content is read first and applied to a copy of the record: if
                // it cannot be read whole, neither draft is created nor changed.
                let mut bound = draft.clone();
                if let Err(detail) = self.fill_from_remote(&mut bound, account, &current) {
                    return Event::ResolveFailed(id, detail);
                }
                let mut copy = draft;
                // A second draft with the same `Message-ID` would make two sends ambiguous
                // to the recipient and to our own reconciliation.
                copy.id = DraftId(0);
                copy.message_id = new_message_id(&copy.account, now);
                copy.revision = 0;
                copy.remote_id = None;
                copy.remote_revision = None;
                copy.thread_remote_id = None;
                copy.synced_content_revision = None;
                copy.state = DraftState::Editing;
                let copy = match self.stores.drafts.create(&mut copy) {
                    Ok(stored) => stored,
                    Err(e) => return self.store_failed(Some(id), e),
                };
                match self.take_remote(id, &bound, &current) {
                    Ok(Bind::Stored(bound)) => Event::ResolvedBoth {
                        bound,
                        copy: Box::new(copy),
                    },
                    // Nothing was taken, so the new draft this choice would have paired it
                    // with is not left behind: a resolution that could not be carried out
                    // changes nothing, and the user can make the choice again.
                    Ok(Bind::MovedAccount) => {
                        let _ = self.stores.drafts.delete(copy.id);
                        moved_resolution(id)
                    }
                    Err(e) => {
                        // The copy is on its own and harmless; the bound draft is untouched.
                        self.store_failed(Some(id), e)
                    }
                }
            }
        }
    }

    /// Record which provider draft this local draft is now bound to, on the row as it
    /// stands, and mark exactly which content the server holds.
    ///
    /// `uploaded` is the content revision of the snapshot that was actually put on the
    /// server. It is not simply "the newest content": a keystroke that landed while the
    /// call was out is not on the server, and claiming it is would tell the UI that
    /// something was mirrored that never left this Mac. So the marker names the uploaded
    /// revision, and a row whose content has moved past it stays honestly unmirrored.
    ///
    /// A row that moved to another account while the call was out is left untouched and
    /// answered [`Bind::MovedAccount`]. Those remote ids belong to the *other* account's
    /// provider, so binding them here would make the next send update — or deliver from —
    /// a draft in the wrong mailbox. Callers must act on that answer: reporting a mirror
    /// that was never recorded, or going on to send, is exactly what it prevents.
    fn bind_published(
        &self,
        id: DraftId,
        account: &str,
        remote: &RemoteDraft,
        uploaded: u64,
        resolved: bool,
    ) -> Result<Bind, DraftError> {
        let account = account.to_owned();
        let remote = remote.clone();
        let wanted = account.clone();
        let stored = self.stores.drafts.update_current(id, move |row| {
            if row.account != account {
                return Ok(());
            }

            row.remote_id = Some(remote.id.clone());
            row.remote_revision = Some(remote.revision.clone());
            if row.thread_remote_id.is_none() {
                row.thread_remote_id = Some(remote.thread.clone());
            }
            row.synced_content_revision = Some(uploaded);
            if resolved {
                row.state = DraftState::Editing;
            }
            Ok(())
        })?;
        // The record handed back is the row as it stands, so the account it now names says
        // whether the binding was applied at all.
        Ok(if stored.account == wanted {
            Bind::Stored(Box::new(stored))
        } else {
            Bind::MovedAccount
        })
    }

    /// Replace the local draft with the provider's copy, on the row as it stands.
    ///
    /// The one place where the local body is replaced on purpose: the user chose the
    /// server's version, and the UI freezes this composer while the choice is carried out.
    /// The content is marked mirrored in the same write, because what ends up stored came
    /// off the server — it is not an unsent local edit, and a stale marker would have the
    /// next sync offer to replace it all over again.
    ///
    /// A row that has since moved account is answered [`Bind::MovedAccount`] with nothing
    /// written: taking the previous account's message over a draft that now belongs to
    /// another one would move a conversation between mailboxes.
    fn take_remote(
        &self,
        id: DraftId,
        filled: &DraftRecord,
        remote: &RemoteDraft,
    ) -> Result<Bind, DraftError> {
        let filled = filled.clone();
        let remote = remote.clone();
        let account = filled.account.clone();
        let wanted = account.clone();
        let stored = self.stores.drafts.update_current(id, move |row| {
            if row.account != account {
                return Ok(());
            }

            row.message_id = filled.message_id.clone();
            row.references = filled.references.clone();
            row.in_reply_to = filled.in_reply_to.clone();
            row.subject = filled.subject.clone();
            row.to = filled.to.clone();
            row.cc = filled.cc.clone();
            row.bcc = filled.bcc.clone();
            row.body = filled.body.clone();
            row.attachments = filled.attachments.clone();
            row.note_imported_attachments();
            row.remote_id = Some(remote.id.clone());
            row.remote_revision = Some(remote.revision.clone());
            row.thread_remote_id = Some(remote.thread.clone());
            row.state = DraftState::Editing;
            row.note_mirrored();
            Ok(())
        })?;
        Ok(if stored.account == wanted {
            Bind::Stored(Box::new(stored))
        } else {
            Bind::MovedAccount
        })
    }

    /// Mark the row's current content as held by the provider, without touching it.
    ///
    /// For content that came *from* the provider, so it is not an unsent local edit.
    fn mark_mirrored(&self, id: DraftId) -> Result<DraftRecord, DraftError> {
        self.stores.drafts.update_current(id, |row| {
            row.note_mirrored();
            Ok(())
        })
    }

    /// The provider's copy of a draft, or `None` when there is none: never bound yet, or
    /// gone from the server (sent or deleted elsewhere). A provider that cannot answer or
    /// refuses is `None` as well, because the callers fall back to writing the draft
    /// rather than trusting a copy they could not read.
    fn current_remote(&self, draft: &DraftRecord, account: &str) -> Option<RemoteDraft> {
        let remote_id = draft.remote_id.clone()?;
        match self.compose(account, |compose| match compose.get_draft(&remote_id) {
            Ok(remote) => Ok(Some(remote)),
            Err(ComposeError::NotFound(_)) => Ok(None),
            Err(e) => Err(e),
        }) {
            Attempt::Ok(remote) => remote,
            Attempt::NoBackend | Attempt::Failed(_) => None,
        }
    }

    /// Whether the revision this client persisted is still the one the provider holds.
    fn unchanged_since(&self, draft: &DraftRecord, remote: &RemoteDraft) -> bool {
        draft
            .remote_revision
            .as_deref()
            .is_none_or(|seen| seen == remote.revision)
    }

    /// Record which provider draft a local record is bound to, before it is first stored.
    ///
    /// Only for a record that does not exist yet: everything else binds through
    /// [`Worker::bind_published`], which writes to the row as it stands.
    fn adopt_remote_ids(draft: &mut DraftRecord, remote: &RemoteDraft) {
        draft.remote_id = Some(remote.id.clone());
        draft.remote_revision = Some(remote.revision.clone());
        if draft.thread_remote_id.is_none() {
            draft.thread_remote_id = Some(remote.thread.clone());
        }
    }

    /// Put the draft on the server, creating or replacing it, refusing a remote draft
    /// that moved on. Returns the provider's copy, which the send then uses.
    fn publish(
        &self,
        draft: &DraftRecord,
        account: &str,
        mail: &OutgoingMail,
    ) -> Result<RemoteDraft, PublishError> {
        let handle = self
            .account_handle(account)
            .ok_or_else(|| PublishError::NoBackend(account.to_owned()))?;
        // The token this client persisted, never the one the provider is about to hand
        // back: comparing a fresh revision with itself can never find a conflict.
        let persisted = draft.remote_revision.clone();
        let remote_id = draft.remote_id.clone();
        let step = handle
            .with_compose(|compose| {
                let Some(remote_id) = remote_id.clone() else {
                    // Never bound to a remote draft: there is nothing that could have been
                    // sent in its place, so this is the one case that creates one.
                    return compose.create_draft(mail).map(Step::Created);
                };
                let current = match compose.get_draft(&remote_id) {
                    Ok(current) => current,
                    // The draft this one is bound to is gone from the server. It may have
                    // been sent from the web, so nothing is created in its place: only a
                    // draft that was never bound may create one, and a send that finds it
                    // missing reconciles its own Message-ID instead.
                    Err(ComposeError::NotFound(_)) => {
                        return Ok(Step::Vanished(remote_id));
                    }
                    Err(e) => return Err(e),
                };
                let mut expected = current.clone();
                if let Some(revision) = persisted.as_ref() {
                    if revision != &current.revision {
                        // Somebody replaced the draft since we last saw it.
                        return Ok(Step::Moved(current));
                    }
                    expected.revision = revision.clone();
                }
                match compose.update_draft(&expected, mail) {
                    Ok(updated) => Ok(Step::Updated(updated)),
                    // Lost the race between the read and the write.
                    Err(ComposeError::Conflict { .. }) => Ok(Step::Moved(
                        compose.get_draft(&remote_id).unwrap_or(current),
                    )),
                    Err(e) => Err(e),
                }
            })
            .ok_or_else(|| PublishError::NoBackend(account.to_owned()))?;
        match step {
            Ok(Step::Created(remote)) | Ok(Step::Updated(remote)) => Ok(remote),
            Ok(Step::Moved(remote)) => Err(PublishError::Moved(Box::new(remote))),
            Ok(Step::Vanished(remote)) => Err(PublishError::Vanished(remote)),
            Err(e) => Err(PublishError::Failed(e)),
        }
    }

    /// The MIME to upload for `draft`. Both refusals are reported as they are: a malformed
    /// address, and an attachment whose bytes cannot be read. A draft with no recipient is
    /// *not* one of them — that is a rule of the send path, checked there.
    fn mail_for(
        &self,
        draft: &DraftRecord,
        from: &str,
        now: Timestamp,
    ) -> Result<OutgoingMail, String> {
        let message = OutgoingMessage::from_draft(draft, from, now)?;
        let rfc822 = build_mime(&message, &draft.attachments, draft.revision)
            .map_err(|e| format!("an attachment could not be read: {e}"))?;
        Ok(OutgoingMail {
            rfc822,
            message_id: message.message_id.clone(),
            in_reply_to: message.in_reply_to.clone(),
            references: message.references.clone(),
            thread: draft.thread_remote_id.clone(),
            account: draft.account.clone(),
        })
    }

    /// Mark the local draft as conflicting and hand back the provider's copy. The local
    /// text is kept: a conflict is reported, never resolved by overwriting either side.
    ///
    /// The state is set on the row as it stands. A conflict is discovered after a provider
    /// call, by which time the user may well have typed more; that newer text is what the
    /// draft keeps, and only its state changes.
    fn conflicted(&mut self, draft: &DraftRecord, remote: RemoteDraft) -> Event {
        let state = DraftState::Conflict;
        match self.set_state(draft.id, state) {
            Ok(stored) => Event::Conflicted(Box::new(stored), Box::new(remote)),
            Err(e) => self.store_failed(Some(draft.id), e),
        }
    }

    /// Put a draft back the way the user left it: editable, no attempt recorded, and no
    /// hold row behind it. Used when a send cannot even be attempted — an account that
    /// cannot compose any more — so a draft is never left frozen with a countdown that
    /// will not run, and never carries an attempt nobody made.
    fn restore_editable(&mut self, draft: &DraftRecord, reason: &str) -> Vec<Event> {
        let _ = self.stores.drafts.clear_pending(draft.id);
        let report = |stored: DraftRecord| Event::SendFailed(Box::new(stored), reason.to_owned());
        match self.stores.drafts.update_current(draft.id, |row| {
            row.state = DraftState::Editing;
            row.attempted = false;
            Ok(())
        }) {
            Ok(stored) => vec![report(stored)],
            // Nothing was sent and nothing was left scheduled, which is exactly what a
            // restart would work out from the row still sitting there. So the draft is
            // still reported as the user's again — the UI stops waiting on a send that
            // never happened — and the write that failed is reported beside it rather than
            // in place of it.
            Err(e) => {
                // The answer the UI acts on is still "your draft is editable again",
                // otherwise a held draft stays frozen with a countdown that will not run.
                let mut back = draft.clone();
                back.state = DraftState::Editing;
                back.attempted = false;
                vec![
                    report(back),
                    self.unrecorded(draft.id, NOTHING_SENT_TRUTH, e),
                ]
            }
        }
    }

    /// Refuse an operation with nothing applied, leaving the draft editable with the
    /// reason attached.
    fn refuse(&mut self, draft: &DraftRecord, error: &str) -> Event {
        let state = DraftState::Failed {
            error: error.to_owned(),
        };
        match self.set_state(draft.id, state) {
            Ok(stored) => Event::SendFailed(Box::new(stored), error.to_owned()),
            Err(e) => self.store_failed(Some(draft.id), e),
        }
    }

    /// Move a draft to `state` on the row as it stands, leaving its content alone.
    fn set_state(&self, id: DraftId, state: DraftState) -> Result<DraftRecord, DraftError> {
        self.stores.drafts.update_current(id, |row| {
            row.state = state;
            Ok(())
        })
    }

    // --------------------------------------------------------------- importing

    /// Import the provider's drafts as local ones. A draft already held locally is that
    /// same draft, so it is reported rather than duplicated. One that cannot be imported
    /// whole is never partially adopted, so a local draft can never exist with files
    /// quietly missing from it.
    fn import(&mut self, account: &str, limit: usize) -> Event {
        let listed = match self.compose(account, |compose| compose.list_drafts(None, limit)) {
            Attempt::Ok(page) => page.drafts,
            Attempt::NoBackend => {
                return Event::Unsupported {
                    account: account.to_owned(),
                };
            }
            Attempt::Failed(e) => return Event::JobFailed(e.to_string()),
        };
        let known = self.stores.drafts.list().unwrap_or_default();
        let mut fresh = Vec::new();
        let mut already_local = Vec::new();
        let mut failed = Vec::new();
        for remote in listed {
            // A remote id is scoped to one account: a draft bound to *another* account's
            // provider with the same id is a different draft, not one already held here.
            if known
                .iter()
                .any(|d| d.account == account && d.remote_id.as_deref() == Some(remote.id.as_str()))
            {
                already_local.push(remote);
                continue;
            }
            match self.adopt(account, &remote) {
                Ok(record) => fresh.push(record),
                Err(detail) => {
                    failed.push(ImportFailure {
                        draft: None,
                        remote: remote.id,
                        detail,
                    });
                }
            }
        }
        Event::Imported {
            account: account.to_owned(),
            fresh,
            already_local,
            failed,
        }
    }

    fn import_one(&mut self, account: &str, remote_id: RemoteId) -> Event {
        let remote = match self.compose(account, |compose| compose.get_draft(&remote_id)) {
            Attempt::Ok(remote) => remote,
            Attempt::NoBackend => {
                return Event::Unsupported {
                    account: account.to_owned(),
                };
            }
            Attempt::Failed(ComposeError::NotFound(_)) => {
                return Event::JobFailed("that draft is no longer on the server".into());
            }
            Attempt::Failed(e) => return Event::JobFailed(e.to_string()),
        };
        match self.adopt(account, &remote) {
            Ok(record) => Event::Imported {
                account: account.to_owned(),
                fresh: vec![record],
                already_local: Vec::new(),
                failed: Vec::new(),
            },
            Err(detail) => Event::Imported {
                account: account.to_owned(),
                fresh: Vec::new(),
                already_local: Vec::new(),
                failed: vec![ImportFailure {
                    draft: None,
                    remote: remote_id,
                    detail,
                }],
            },
        }
    }

    /// Turn a provider draft into a local one, with its rich original preserved and its
    /// RFC `Message-ID` recorded, so a send of the untouched draft stays reconcilable.
    fn adopt(&mut self, account: &str, remote: &RemoteDraft) -> Result<DraftRecord, String> {
        let mut draft = DraftRecord::new_email(account, remote.updated);
        self.fill_from_remote(&mut draft, account, remote)?;
        // The import *is* this provider's draft, so the record is bound to it from the
        // start: without the id and its revision the next sync would create a second
        // copy, and a conflict could never be detected against the original.
        Self::adopt_remote_ids(&mut draft, remote);
        let stored = self
            .stores
            .drafts
            .create(&mut draft)
            .map_err(|e| e.to_string())?;
        // What an import adopts came off the server, so it is already mirrored. Saying so
        // keeps the persisted marker honest: an unmirrored draft reads as a local edit
        // still waiting to be pushed, and the next sync would offer to replace the rich
        // original nobody has approved converting.
        self.mark_mirrored(stored.id).map_err(|e| e.to_string())
    }

    /// Read a provider draft into a local record: headers, the rich original body and
    /// every file. Nothing is written to `draft` until the whole remote has been read, so
    /// a failed read can never leave half a message behind.
    fn fill_from_remote(
        &mut self,
        draft: &mut DraftRecord,
        account: &str,
        remote: &RemoteDraft,
    ) -> Result<(), String> {
        let gathered = match self.compose(account, |compose| gather(compose, remote)) {
            Attempt::Ok(gathered) => gathered,
            Attempt::NoBackend => return Err("this account cannot compose".to_owned()),
            Attempt::Failed(e) => return Err(e.to_string()),
        };
        if !gathered.decoded
            && !remote.attachments_known
            && gathered.html.is_none()
            && gathered.text.trim().is_empty()
        {
            // Nothing was read and nothing is known about what is there: that is a draft
            // we cannot import honestly. An empty draft that *was* read is not this.
            return Err(
                "the stored message could not be read: the provider would not hand over its \
                 bytes and could not say what the draft contains"
                    .to_owned(),
            );
        }
        let body = match gathered.html {
            Some(html) if !html.trim().is_empty() => {
                let text = if gathered.text.trim().is_empty() {
                    crate::outgoing::html::html_text(&html)
                } else {
                    gathered.text
                };
                DraftBody::import_rich(&html, &text)
            }
            _ => DraftBody::import_markdown(gathered.text),
        };
        // Files first: one that cannot be stored fails the whole import rather than
        // producing a draft that would go out missing it.
        let mut attachments = Vec::with_capacity(gathered.files.len());
        for (index, file) in gathered.files.iter().enumerate() {
            // An inline part the original named only by its `Content-ID` still needs a
            // name on disk, and that id is the one the body refers to.
            let name = if !file.name.trim().is_empty() {
                file.name.clone()
            } else if let Some(cid) = &file.content_id {
                cid.clone()
            } else {
                format!("inline-{index}")
            };
            let source = AttachmentSource::Forwarded {
                message: remote.message.clone(),
                part: file.part.clone(),
            };
            attachments.push(self.managed_copy(
                &name,
                &file.bytes,
                &file.content_type,
                source,
                file.content_id.clone(),
            )?);
        }
        // A draft the provider holds keeps its `Message-ID`: a send of its untouched bytes
        // has to be reconcilable, and our own generated id is not in those bytes.
        if !gathered.message_id.trim().is_empty() {
            draft.message_id = gathered.message_id.clone();
        }
        draft.references = gathered.references.clone();
        draft.in_reply_to = gathered.in_reply_to.clone();
        draft.subject = remote.subject.clone();
        draft.to = remote.to.clone();
        draft.cc = remote.cc.clone();
        draft.bcc = remote.bcc.clone();
        draft.body = body;
        draft.attachments = attachments;
        draft.note_imported_attachments();
        // The files this import brought in are the baseline a send compares against before
        // it may send the server's own bytes untouched. Replacing one of them with
        // different bytes under the same name changes its identity, and the payload is
        // then ours to build.
        Ok(())
    }

    // ------------------------------------------------------------------ sending

    fn send(&mut self, id: DraftId, account: &str, now: Timestamp) -> Vec<Event> {
        let mut draft = match self.load(id) {
            Ok(draft) => draft,
            Err(event) => return vec![event],
        };
        // The account this send goes through is the one the *durable record* names, not
        // whatever the surface happened to hold when it asked. A request that queued behind
        // a provider call, or behind the user picking a different account, must not deliver
        // from the mailbox that was on screen at the time.
        if draft.account != account {
            return self.restore_editable(
                &draft,
                "this draft now belongs to another account, so it was not sent from the one \
                 this request named; nothing was sent",
            );
        }
        let Some(handle) = self.account_handle(account) else {
            // Nothing was attempted, so a held draft must not be left held: the user gets
            // their draft back with the reason, and no hold row survives it.
            return self.restore_editable(
                &draft,
                "this account cannot send any more, so nothing was sent",
            );
        };
        let from = handle.email.clone();
        // A recipient becomes mandatory here and nowhere else: an unaddressed draft still
        // saves, syncs and previews, but a send is refused before anything is attempted.
        if let Err(reason) = draft.validate() {
            return vec![self.refuse(&draft, &format!("this draft cannot be sent: {reason}"))];
        }
        let mail = match self.mail_for(&draft, &from, now) {
            Ok(mail) => mail,
            Err(reason) => {
                return vec![self.refuse(&draft, &format!("this draft cannot be sent: {reason}"))];
            }
        };
        // What the provider still holds, when it holds anything: the only thing that can
        // say whether this draft is still theirs down to the byte.
        let current = self.current_remote(&draft, account);
        // A rich body imported from the provider and never converted goes out as the
        // provider's own bytes, with its own Message-ID: rebuilding it from Markdown would
        // lose what the conversion cannot express. So such a draft is not even written over
        // on the server. Anything the user changed — recipients, subject, files — makes the
        // payload ours to build instead, and that rebuild keeps what the original had: the
        // body still renders from the preserved HTML, and every inline part carries the
        // `Content-ID` its body points at, so a `cid:` reference still resolves.
        let uploaded = draft.content_revision;
        let (remote, as_stored) = match current {
            Some(current) if self.still_theirs(&draft, &current) => {
                if !self.unchanged_since(&draft, &current) {
                    return vec![self.conflicted(&draft, current)];
                }
                (current, true)
            }
            _ => {
                // The provider's copy is put there first: the send endpoint is only ever
                // asked to deliver bytes it already holds, and a draft that moved on is
                // refused rather than mailed over someone else's copy.
                match self.publish(&draft, account, &mail) {
                    Ok(remote) => (remote, false),
                    Err(PublishError::Moved(remote)) => {
                        return vec![self.conflicted(&draft, *remote)];
                    }
                    // The bound draft is gone: it may have been sent from the web, so the
                    // only safe move is to look for the message, never to send another.
                    Err(PublishError::Vanished(_)) => {
                        return self.vanished(&mut draft, account, now);
                    }
                    Err(PublishError::NoBackend(_)) => {
                        return self.restore_editable(
                            &draft,
                            "this account cannot send any more, so nothing was sent",
                        );
                    }
                    Err(PublishError::Failed(e)) => return vec![self.compose_failed(&draft, e)],
                }
            }
        };
        // The draft the provider now holds is recorded on the row as it stands, before
        // anything is attempted. Text typed while the publish was out is not on the
        // server, so the marker names the snapshot that was uploaded.
        let bound = match self.bind_published(id, account, &remote, uploaded, false) {
            Ok(Bind::Stored(stored)) => *stored,
            // The row moved account while the publish was out, so those remote ids are the
            // previous account's. Sending from them would deliver this message out of the
            // wrong mailbox, and there is nothing to reconcile against afterwards either.
            // Nothing has been sent at this point.
            Ok(Bind::MovedAccount) => {
                return self.restore_editable(
                    &draft,
                    "this draft was switched to another account while it was being sent, so \
                     nothing was sent from its previous account",
                );
            }
            // Without a durable record of what the provider holds there is nothing to
            // reconcile against and nothing to point the next operation at, so the draft
            // goes back to the user instead of being sent from a binding this Mac never
            // kept. Nothing has been sent at this point.
            Err(e) => {
                return self.restore_editable(
                    &draft,
                    &format!(
                        "this draft could not record the provider draft it was put on, so \
                         nothing was sent: {e}"
                    ),
                );
            }
        };
        let mut draft = bound;
        let payload = (!as_stored).then_some(&mail);
        // Without a durable attempt there is nothing to reconcile against, so a store that
        // cannot record one is precisely why nothing is sent: the draft is handed back as
        // it was rather than left looking like a request is in flight.
        if let Err(e) = self.stores.drafts.begin_send(&mut draft, now) {
            return self.restore_editable(
                &draft,
                &format!("this draft could not be recorded as sending, so nothing was sent: {e}"),
            );
        }
        let outcome = match handle.with_compose(|compose| compose.send_draft(&remote, payload)) {
            Some(Ok(SendStatus::Sent(sent))) => Outcome::Sent(sent),
            Some(Ok(SendStatus::Uncertain { detail })) => Outcome::Uncertain(detail),
            Some(Err(e)) => Outcome::from_error(e),
            // The account was composing a moment ago and is not any more.
            None => Outcome::Refused("this account stopped being able to send".into()),
        };
        match outcome {
            Outcome::Sent(sent) => {
                let remote_id = sent.id.clone();
                self.settle(
                    &draft,
                    DraftState::Sent { remote_id },
                    now,
                    SENT_TRUTH,
                    move |stored| {
                        // What went out is what the user saw: the rendered body of the
                        // record that was sent, under the headers it was sent with.
                        let message = OutgoingMessage::from_draft(&stored, &from, now)
                            .unwrap_or_else(|_| describe(&stored, &from));
                        Event::Sent {
                            draft: Box::new(stored),
                            sent: Box::new(sent),
                            message: Box::new(message),
                        }
                    },
                )
            }
            Outcome::Uncertain(detail) => self.settle(
                &draft,
                DraftState::Uncertain,
                now,
                UNCERTAIN_TRUTH,
                move |stored| Event::Uncertain(Box::new(stored), detail),
            ),
            Outcome::AutosaveUncertain(detail) => {
                // No mail was sent. The draft becomes editable again with the reason
                // attached, and the next import shows what the server actually holds.
                let error = format!("the draft may not have been saved to the server: {detail}");
                self.settle(
                    &draft,
                    DraftState::Failed { error },
                    now,
                    NOTHING_SENT_TRUTH,
                    move |stored| Event::SendFailed(Box::new(stored), detail),
                )
            }
            Outcome::Offline => {
                // Nothing was applied and nothing left: the draft is exactly as it was, so
                // no later send is scheduled and no duplicate can appear.
                self.settle(
                    &draft,
                    DraftState::Failed {
                        error: "you appear to be offline".into(),
                    },
                    now,
                    NOTHING_SENT_TRUTH,
                    |stored| Event::Offline(Box::new(stored)),
                )
            }
            Outcome::Refused(error) => {
                let state = DraftState::Failed {
                    error: error.clone(),
                };
                self.settle(&draft, state, now, NOTHING_SENT_TRUTH, move |stored| {
                    Event::SendFailed(Box::new(stored), error)
                })
            }
        }
    }

    /// Persist how a send ended and report it, whether or not the store could keep it.
    ///
    /// The provider's answer is the truth about delivery; the local row is only this
    /// client's memory of it, so a failed write never replaces the answer. The outcome is
    /// still reported — from the record the store would have kept, had it been able to —
    /// and the failed write is reported beside it. `truth` is that answer in words, and it
    /// leads the paired error: a surface that shows only the most recent message must never
    /// leave the last thing it said being a bare storage error, so a confirmed delivery and
    /// a delivery in doubt both survive being the earlier of two messages.
    ///
    /// The state is written to the row *as it stands*, not to the snapshot the send was
    /// built from. A keystroke that landed while the provider was answering is not lost to
    /// the outcome: the row keeps that text and records how the attempt ended, which is
    /// the only thing the send decided. Nothing is retried here, and the row left behind
    /// still carries the attempt when the write fails, which is what a restart reconciles
    /// by `Message-ID` rather than sending a second copy.
    fn settle(
        &mut self,
        draft: &DraftRecord,
        state: DraftState,
        now: Timestamp,
        truth: &str,
        report: impl FnOnce(DraftRecord) -> Event,
    ) -> Vec<Event> {
        // A definite rejection, an offline send or a conflict clears the attempt; only a
        // send that may still arrive keeps it, or every restart would keep such a draft
        // uncertain forever.
        let keeps_attempt = !matches!(
            state,
            DraftState::Failed { .. } | DraftState::Editing | DraftState::Conflict
        );
        let settled = state.clone();
        match self.stores.drafts.update_current(draft.id, |row| {
            if !keeps_attempt {
                row.attempted = false;
            }
            row.state = settled;
            row.touch(now);
            Ok(())
        }) {
            Ok(written) => vec![report(written)],
            Err(e) => vec![
                report(as_stored(draft, &state, now)),
                self.unrecorded(draft.id, truth, e),
            ],
        }
    }

    /// The write that could not keep the record, said *after* what it failed to record.
    fn unrecorded(&self, id: DraftId, truth: &str, error: DraftError) -> Event {
        Event::StoreFailed(
            Some(id),
            format!("{truth}, but this Mac could not record it: {error}"),
        )
    }

    /// The draft this one is bound to is no longer on the server. The likeliest reason is
    /// that it was sent from the web a moment ago, so nothing is created and nothing is
    /// sent: the record's own `Message-ID` is looked up instead. A hit is a confirmed
    /// send, and a miss leaves the draft uncertain rather than sending a second copy.
    fn vanished(&mut self, draft: &mut DraftRecord, account: &str, now: Timestamp) -> Vec<Event> {
        let message_id = draft.message_id.clone();
        let lookup = match self.compose(account, |compose| compose.find_sent(&message_id)) {
            Attempt::Ok(Some(sent)) => Lookup::Found(sent),
            Attempt::Ok(None) => Lookup::NotFound,
            Attempt::NoBackend => {
                Lookup::Failed("this account cannot check Sent until it is linked again".into())
            }
            Attempt::Failed(e) => Lookup::Failed(e.to_string()),
        };
        if let Lookup::NotFound = lookup {
            return self.settle(
                draft,
                DraftState::Uncertain,
                now,
                UNCERTAIN_TRUTH,
                |stored| Event::Reconciled {
                    draft: Box::new(stored),
                    detail: "this draft is no longer on the server, so it may already have \
                             been sent from another device; nothing was sent again"
                        .to_owned(),
                },
            );
        }
        self.settle_attempt(draft, account, now, lookup)
    }

    /// Whether the provider still holds exactly this draft, so that the send can carry
    /// the provider's own bytes: the same recipients and subject, an imported rich body
    /// nobody has converted, exactly the files the import brought in, and the same
    /// `Message-ID` — the identity a send of those bytes is reconciled by.
    ///
    /// Files are compared by identity, not by name. A name is not content: replacing a
    /// file with different bytes under the same name must rebuild the payload, or the
    /// server would go on holding — and send — the old one.
    ///
    /// Both the payload and the identity have to be *known*. A provider that could not
    /// read them leaves a draft whose files may have been removed here, or whose bytes
    /// we cannot name, and the send is then rebuilt from the preserved original under
    /// this draft's own stable `Message-ID` — which is safe, and is not the same thing as
    /// mailing whatever the server still holds.
    fn still_theirs(&self, draft: &DraftRecord, remote: &RemoteDraft) -> bool {
        if !draft.body.needs_conversion()
            || draft.to.trim() != remote.to.trim()
            || draft.cc.trim() != remote.cc.trim()
            || draft.bcc.trim() != remote.bcc.trim()
            || draft.subject.trim() != remote.subject.trim()
            || remote.message_id.as_deref() != Some(draft.message_id.as_str())
            || !remote.attachments_known
        {
            return false;
        }
        if !draft.imported_parts_untouched() {
            // No baseline, or the files are not the ones that were imported: a swap, an
            // addition or a removal all mean the payload is ours to build.
            return false;
        }
        sorted(&attachment_names(&draft.attachments)) == remote_names(remote)
    }

    /// Ask the provider whether an uncertain send went out, and settle the draft by what it
    /// can prove. A found message is a confirmed send and is reported as one, with the
    /// provider's own record of it, so it takes exactly the same path as a send whose
    /// answer was seen. A lookup that finds nothing is not proof of non-delivery, so the
    /// draft stays uncertain: this never resends and never makes it editable as if nothing
    /// had happened.
    fn reconcile(&mut self, id: DraftId, account: &str) -> Vec<Event> {
        let mut draft = match self.load(id) {
            Ok(draft) => draft,
            Err(event) => return vec![event],
        };
        if matches!(draft.state, DraftState::Sent { .. }) {
            return vec![Event::Reconciled {
                draft: Box::new(draft),
                detail: "delivery was already confirmed".into(),
            }];
        }
        // Checked against the account the *durable record* names. A draft that moved
        // account was never attempted from the new one, so a message found in the old
        // mailbox must not be recorded as this draft's delivery.
        if draft.account != account {
            return vec![Event::Reconciled {
                draft: Box::new(draft),
                detail: "this draft now belongs to another account, so the account it was \
                         checked against is not the one it was sent from"
                    .to_owned(),
            }];
        }
        let message_id = draft.message_id.clone();
        let lookup = match self.compose(account, |compose| compose.find_sent(&message_id)) {
            Attempt::Ok(Some(sent)) => Lookup::Found(sent),
            Attempt::Ok(None) => Lookup::NotFound,
            Attempt::NoBackend => {
                Lookup::Failed("this account cannot check Sent until it is linked again".into())
            }
            Attempt::Failed(e) => Lookup::Failed(e.to_string()),
        };
        let now = draft.updated_at;
        self.settle_attempt(&mut draft, account, now, lookup)
    }

    /// Turn the provider's answer about an attempt into the draft's state and the event the
    /// UI acts on. A found message is delivered: the record is stored as sent, with the
    /// provider's own sent id, thread, time and RFC `Message-ID`, and the message is
    /// rebuilt from the attempt so the outbox shows what actually went out.
    fn settle_attempt(
        &mut self,
        draft: &mut DraftRecord,
        account: &str,
        now: Timestamp,
        lookup: Lookup,
    ) -> Vec<Event> {
        let from = self
            .account_handle(account)
            .map(|handle| handle.email)
            .unwrap_or_else(|| account.to_owned());
        match lookup {
            Lookup::Found(sent) => {
                let remote_id = sent.id.clone();
                self.settle(
                    draft,
                    DraftState::Sent { remote_id },
                    now,
                    SENT_TRUTH,
                    move |stored| {
                        let message = OutgoingMessage::from_draft(&stored, &from, now)
                            .unwrap_or_else(|_| describe(&stored, &from));
                        Event::Sent {
                            draft: Box::new(stored),
                            sent: Box::new(sent),
                            message: Box::new(message),
                        }
                    },
                )
            }
            Lookup::NotFound => self.settle(
                draft,
                DraftState::Uncertain,
                now,
                UNCERTAIN_TRUTH,
                |stored| Event::Reconciled {
                    draft: Box::new(stored),
                    detail: "the provider has no sent message with this Message-ID yet, \
                                 which is not proof it was never sent"
                        .to_owned(),
                },
            ),
            Lookup::Failed(message) => self.settle(
                draft,
                DraftState::Uncertain,
                now,
                UNCERTAIN_TRUTH,
                move |stored| Event::Reconciled {
                    draft: Box::new(stored),
                    detail: message,
                },
            ),
        }
    }

    /// A provider failure during a draft write. No mail was sent either way, so this is
    /// never reported as a delivery in doubt.
    fn compose_failed(&mut self, draft: &DraftRecord, error: ComposeError) -> Event {
        match Outcome::from_error(error) {
            Outcome::AutosaveUncertain(detail) | Outcome::Uncertain(detail) => {
                Event::AutosaveFailed(draft.id, detail)
            }
            Outcome::Offline => Event::Offline(Box::new(draft.clone())),
            Outcome::Refused(detail) => Event::AutosaveFailed(draft.id, detail),
            Outcome::Sent(_) => unreachable!("from_error never produces Sent"),
        }
    }

    fn store_failed(&self, draft: Option<DraftId>, error: DraftError) -> Event {
        Event::StoreFailed(draft, error.to_string())
    }
}

/// Read one remote draft as it is actually stored: its headers, its body and every file,
/// decoded from the raw message.
///
/// A transport failure may fall back to what the provider's own answer already carries,
/// but a *decode* failure may not: half a message that looked like a message with no HTML
/// and no files is exactly the silent data loss an import must never produce.
fn gather(
    compose: &mut dyn ComposeProvider,
    remote: &RemoteDraft,
) -> Result<Gathered, ComposeError> {
    let mut decoded: Option<ParsedMessage> = match compose.draft_raw(&remote.id) {
        Ok(raw) => Some(parse_message(&raw).map_err(|e| {
            ComposeError::Provider(ProviderError::Api {
                status: 422,
                message: format!("the stored draft could not be decoded: {e}"),
            })
        })?),
        // Offline or throttled: what the provider already told us is all there is until
        // the next import.
        Err(e) if e.is_retryable() => None,
        // Gone, or the read really failed: the draft is not importable, not half-imported.
        Err(e) => return Err(e),
    };
    let html = decoded
        .as_ref()
        .and_then(|d| d.html.clone())
        .or(remote.body.html.clone());
    let text = match decoded.as_ref() {
        Some(d) if !d.text.trim().is_empty() => d.text.clone(),
        _ => remote.body.text.clone(),
    };
    let mut files: Vec<RemoteFile> = decoded
        .as_mut()
        .map(|d| {
            std::mem::take(&mut d.files)
                .into_iter()
                .enumerate()
                .map(|(i, file)| RemoteFile {
                    name: file.name,
                    content_type: file.content_type,
                    part: format!("part-{i}"),
                    bytes: file.bytes,
                    content_id: file.content_id,
                })
                .collect()
        })
        .unwrap_or_default();
    if files.is_empty() && remote.has_attachments {
        // The provider listed files the stored bytes did not hand over: fetch each one by
        // its part, and report the ones it cannot deliver rather than skipping them.
        for file in &remote.attachments {
            let (bytes, fetched_type) = match &file.inline_data {
                Some(bytes) => (bytes.clone(), String::new()),
                None => {
                    let fetched = compose.attachment_bytes(&remote.message, file)?;
                    (fetched.bytes, fetched.content_type)
                }
            };
            files.push(RemoteFile {
                name: file.name.clone(),
                part: file.id.clone().unwrap_or_default(),
                bytes,
                // The bytes the provider actually serves are the authority on what this
                // file is; a listing that could not name the type must not become an
                // attachment that claims to be whatever the name happens to guess.
                content_type: if file.content_type.trim().is_empty() {
                    fetched_type
                } else {
                    file.content_type.clone()
                },
                content_id: file.content_id.clone(),
            });
        }
    }
    // The stored bytes decide it; when they could not be read, the provider's own answer
    // may still carry the id it read from the full payload. Only when neither has it does
    // the record keep the id it generated for itself.
    let mut message_id = decoded
        .as_ref()
        .and_then(|d| header(d, "message-id"))
        .map(|value| bare_message_id(&value))
        .filter(|id| !id.is_empty())
        .or_else(|| remote.message_id.clone())
        .unwrap_or_default();
    let mut references: Vec<String> = decoded
        .as_ref()
        .and_then(|d| header(d, "references"))
        .map(|value| {
            value
                .split_whitespace()
                .map(bare_message_id)
                .filter(|id| !id.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let in_reply_to = decoded
        .as_ref()
        .and_then(|d| header(d, "in-reply-to"))
        .map(|value| bare_message_id(&value))
        .filter(|id| !id.is_empty());
    if message_id.is_empty() || references.is_empty() {
        // The stored headers did not carry the whole chain — a draft keeps its own
        // `Message-ID` but need not repeat `References` — and the provider knows the thread
        // anyway. The stored bytes still decide the id and whatever chain they did carry; the
        // thread only fills in what they leave out, so a reply to this draft answers the
        // conversation it belongs to instead of starting a new one.
        if let Ok(threading) = compose.threading(&remote.message) {
            if message_id.is_empty() {
                message_id = threading.message_id;
            }
            if references.is_empty() {
                references = threading.references;
            }
        }
    }
    Ok(Gathered {
        decoded: decoded.is_some(),
        html,
        text,
        files,
        message_id,
        references,
        in_reply_to,
    })
}

/// A resolution the draft's own account change invalidated: nothing was written, and the
/// draft is exactly as it was, so the user can make the choice again on the right account.
fn moved_resolution(id: DraftId) -> Event {
    Event::ResolveFailed(
        id,
        "this draft was switched to another account while the conflict was being settled, \
         so nothing was changed and nothing was taken from its previous account"
            .to_owned(),
    )
}

fn sorted(names: &[String]) -> Vec<String> {
    let mut names = names.to_vec();
    names.sort();
    names
}

/// The record as the store would have kept it, had it been able to write.
///
/// Used when it could not. What the provider settled on is still true, so the answer the
/// caller acts on must not depend on whether this Mac could keep a note of it. The
/// revision is deliberately *not* advanced, because nothing was written: a later write
/// with this revision still matches the stored row instead of being refused as stale.
fn as_stored(draft: &DraftRecord, state: &DraftState, now: Timestamp) -> DraftRecord {
    let mut stored = draft.clone();
    // A definite outcome clears the attempt exactly as persisting it would have: only
    // `Uncertain` and the in-flight states can still mean the message is on its way.
    if matches!(
        state,
        DraftState::Failed { .. } | DraftState::Editing | DraftState::Conflict
    ) {
        stored.attempted = false;
    }
    stored.state = state.clone();
    stored.touch(now);
    stored
}

/// The names of the files a provider draft carries, when its payload was loaded.
fn remote_names(remote: &RemoteDraft) -> Vec<String> {
    sorted(
        &remote
            .attachments
            .iter()
            .map(|a| a.name.clone())
            .collect::<Vec<_>>(),
    )
}

/// The message as it would be rendered, for the one case a send cannot be built from a
/// record: a draft that is being sent as the provider already holds it.
fn describe(draft: &DraftRecord, from: &str) -> OutgoingMessage {
    let rendered = draft.body.render();
    OutgoingMessage {
        account: draft.account.clone(),
        from: from.to_owned(),
        to: draft.to.clone(),
        cc: draft.cc.clone(),
        bcc: draft.bcc.clone(),
        subject: draft.subject.clone(),
        html: rendered.html,
        text: rendered.text,
        in_reply_to: draft.in_reply_to.clone(),
        references: draft.references.clone(),
        message_id: draft.message_id.clone(),
        date: draft.updated_at,
    }
}

/// Attachments carried by a draft, for a status line or a report.
pub fn attachment_names(attachments: &[ManagedAttachment]) -> Vec<String> {
    attachments.iter().map(|a| a.file_name.clone()).collect()
}

/// Whether this provider error is one a plain retry may fix.
pub fn retryable(error: &ProviderError) -> bool {
    matches!(
        error,
        ProviderError::Network(_) | ProviderError::RateLimited
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::compose::{AttachmentBytes, DraftPage, RemoteAttachment};
    use crate::provider::mock::{MockCompose, MockFault};

    /// The account *id*, which is opaque: it is not an address, and the address the
    /// account sends from is configured separately.
    const ACCOUNT: &str = "personal";
    /// The address that account sends from.
    const SENDER: &str = "me@x.test";
    const OTHER: &str = "you@x.test";
    /// The `Message-ID` the seeded provider draft carries in its own bytes.
    const SEEDED_MESSAGE_ID: &str = "seeded@x.test";

    /// The mock behind a handle the test still holds, so a test can arm a fault or make
    /// a write this client never hears about. Every call is the mock's own; the flag
    /// pretends the provider cannot state a draft's stored identity or its files, which is
    /// what a listing-only or undecodable answer looks like.
    struct SharedMock(Arc<Mutex<MockCompose>>, bool);

    impl ComposeProvider for SharedMock {
        fn list_drafts(
            &mut self,
            page: Option<&str>,
            max: usize,
        ) -> Result<DraftPage, ComposeError> {
            self.0.lock().expect("the mock").list_drafts(page, max)
        }
        fn get_draft(&mut self, id: &RemoteId) -> Result<RemoteDraft, ComposeError> {
            let mut remote = self.0.lock().expect("the mock").get_draft(id)?;
            if self.1 {
                remote.message_id = None;
                remote.attachments_known = false;
            }
            Ok(remote)
        }
        fn draft_raw(&mut self, id: &RemoteId) -> Result<Vec<u8>, ComposeError> {
            self.0.lock().expect("the mock").draft_raw(id)
        }
        fn create_draft(&mut self, mail: &OutgoingMail) -> Result<RemoteDraft, ComposeError> {
            self.0.lock().expect("the mock").create_draft(mail)
        }
        fn update_draft(
            &mut self,
            draft: &RemoteDraft,
            mail: &OutgoingMail,
        ) -> Result<RemoteDraft, ComposeError> {
            self.0.lock().expect("the mock").update_draft(draft, mail)
        }
        fn delete_draft(&mut self, id: &RemoteId) -> Result<(), ComposeError> {
            self.0.lock().expect("the mock").delete_draft(id)
        }
        fn send_draft(
            &mut self,
            draft: &RemoteDraft,
            mail: Option<&OutgoingMail>,
        ) -> Result<SendStatus, ComposeError> {
            self.0.lock().expect("the mock").send_draft(draft, mail)
        }
        fn find_sent(&mut self, message_id: &str) -> Result<Option<SentMail>, ComposeError> {
            self.0.lock().expect("the mock").find_sent(message_id)
        }
        fn list_attachments(
            &mut self,
            message: &RemoteId,
        ) -> Result<Vec<RemoteAttachment>, ComposeError> {
            self.0.lock().expect("the mock").list_attachments(message)
        }
        fn attachment_bytes(
            &mut self,
            message: &RemoteId,
            attachment: &RemoteAttachment,
        ) -> Result<AttachmentBytes, ComposeError> {
            self.0
                .lock()
                .expect("the mock")
                .attachment_bytes(message, attachment)
        }
        fn threading(&mut self, message: &RemoteId) -> Result<Threading, ComposeError> {
            self.0.lock().expect("the mock").threading(message)
        }
    }

    /// Both lanes over one store, as the app runs them: the local persistence lane beside
    /// the provider worker, sharing a draft store. A test drives them the way the UI
    /// does — by handing a command to whichever lane owns it — so what it exercises is the
    /// real routing rather than a stand-in for it.
    struct Rig {
        worker: Worker,
        local: LocalLane,
    }

    impl std::ops::Deref for Rig {
        type Target = Worker;

        fn deref(&self) -> &Worker {
            &self.worker
        }
    }

    impl Rig {
        /// Answer one command on the lane that owns it, exactly as the client does.
        fn handle(&mut self, command: Command) -> Vec<Event> {
            match lane_of(&command) {
                Lane::Local => self.local.handle(command),
                Lane::Work => self.worker.handle(command),
            }
        }
    }

    fn rig(stores: Stores, now: Timestamp) -> (Rig, Arc<Mutex<MockCompose>>) {
        let mock = Arc::new(Mutex::new(MockCompose::new(now)));
        let compose: Arc<Mutex<Box<dyn ComposeProvider>>> =
            Arc::new(Mutex::new(Box::new(SharedMock(Arc::clone(&mock), false))));
        let account = ComposeAccount {
            backend: ComposeBackend::ComposeOnly(compose),
            email: SENDER.to_owned(),
        };
        let mut accounts = Accounts::new();
        accounts.insert(ACCOUNT.to_owned(), account);
        let (signals, _rx) = Signals::new();
        let local = LocalLane::new(stores.drafts.clone(), signals.clone());
        let worker = Worker {
            stores,
            accounts: Arc::new(Mutex::new(accounts)),
            signals,
        };
        (Rig { worker, local }, mock)
    }

    /// A rig over an in-memory store and a deterministic mock backend, with the backend
    /// handed back so a test can fault it or write behind its back.
    fn worker(now: Timestamp) -> (Rig, Arc<Mutex<MockCompose>>) {
        rig(Stores::open_in_memory().expect("stores"), now)
    }

    /// Register a second, independent account with its own backend, the way a second
    /// mailbox would be. Its remote ids start from the beginning, so a draft here and a
    /// draft there can carry the very same id.
    fn add_account(
        worker: &Rig,
        account: &str,
        email: &str,
        now: Timestamp,
    ) -> Arc<Mutex<MockCompose>> {
        let mock = Arc::new(Mutex::new(MockCompose::new(now)));
        let compose: Arc<Mutex<Box<dyn ComposeProvider>>> =
            Arc::new(Mutex::new(Box::new(SharedMock(Arc::clone(&mock), false))));
        let entry = ComposeAccount {
            backend: ComposeBackend::ComposeOnly(compose),
            email: email.to_owned(),
        };
        if let Ok(mut accounts) = worker.accounts.lock() {
            accounts.insert(account.to_owned(), entry);
        }
        mock
    }

    /// Swap `account`'s backend for one that cannot state a draft's stored `Message-ID`
    /// or its files — what a listing-only or undecodable answer looks like — while the
    /// stored bytes are still readable.
    fn hide_remote_identity(worker: &Rig, account: &str, mock: &Arc<Mutex<MockCompose>>) {
        let compose: Arc<Mutex<Box<dyn ComposeProvider>>> =
            Arc::new(Mutex::new(Box::new(SharedMock(Arc::clone(mock), true))));
        let entry = ComposeAccount {
            backend: ComposeBackend::ComposeOnly(compose),
            email: account_email(worker, account),
        };
        if let Ok(mut accounts) = worker.accounts.lock() {
            accounts.insert(account.to_owned(), entry);
        }
    }

    /// A backend whose *send* answer is lost, and whose draft writes behave normally.
    ///
    /// The mock's own lost-response fault covers every call, so a draft write would fail
    /// before anything was ever sent — a different failure with a different answer. This
    /// wrapper sends for real and then loses the reply, which is the one situation
    /// reconciliation exists for.
    struct LoseSendAnswers(Arc<Mutex<MockCompose>>);

    impl ComposeProvider for LoseSendAnswers {
        fn list_drafts(
            &mut self,
            page: Option<&str>,
            max: usize,
        ) -> Result<DraftPage, ComposeError> {
            self.0.lock().expect("the mock").list_drafts(page, max)
        }

        fn get_draft(&mut self, id: &RemoteId) -> Result<RemoteDraft, ComposeError> {
            self.0.lock().expect("the mock").get_draft(id)
        }

        fn draft_raw(&mut self, id: &RemoteId) -> Result<Vec<u8>, ComposeError> {
            self.0.lock().expect("the mock").draft_raw(id)
        }

        fn create_draft(&mut self, mail: &OutgoingMail) -> Result<RemoteDraft, ComposeError> {
            self.0.lock().expect("the mock").create_draft(mail)
        }

        fn update_draft(
            &mut self,
            draft: &RemoteDraft,
            mail: &OutgoingMail,
        ) -> Result<RemoteDraft, ComposeError> {
            self.0.lock().expect("the mock").update_draft(draft, mail)
        }

        fn delete_draft(&mut self, id: &RemoteId) -> Result<(), ComposeError> {
            self.0.lock().expect("the mock").delete_draft(id)
        }

        fn send_draft(
            &mut self,
            draft: &RemoteDraft,
            mail: Option<&OutgoingMail>,
        ) -> Result<SendStatus, ComposeError> {
            // The mail really goes out; only the answer is lost.
            self.0.lock().expect("the mock").send_draft(draft, mail)?;
            Err(ComposeError::Uncertain {
                detail: "sending: response lost".into(),
            })
        }

        fn find_sent(&mut self, message_id: &str) -> Result<Option<SentMail>, ComposeError> {
            self.0.lock().expect("the mock").find_sent(message_id)
        }

        fn list_attachments(
            &mut self,
            message: &RemoteId,
        ) -> Result<Vec<RemoteAttachment>, ComposeError> {
            self.0.lock().expect("the mock").list_attachments(message)
        }

        fn attachment_bytes(
            &mut self,
            message: &RemoteId,
            attachment: &RemoteAttachment,
        ) -> Result<AttachmentBytes, ComposeError> {
            self.0
                .lock()
                .expect("the mock")
                .attachment_bytes(message, attachment)
        }

        fn threading(&mut self, message: &RemoteId) -> Result<Threading, ComposeError> {
            self.0.lock().expect("the mock").threading(message)
        }
    }

    /// A backend that lists the account's drafts but cannot hand over one's stored
    /// bytes.
    ///
    /// The mock's own faults are armed across every call, so they fail the *listing*
    /// too — which is a different answer (the whole import could not be started). What a
    /// per-draft read failure looks like is the listing succeeding and the one draft's
    /// bytes being unavailable, so that is what this wrapper is.
    struct UnreadableDrafts(Arc<Mutex<MockCompose>>, RemoteId);

    impl ComposeProvider for UnreadableDrafts {
        fn list_drafts(
            &mut self,
            page: Option<&str>,
            max: usize,
        ) -> Result<DraftPage, ComposeError> {
            self.0.lock().expect("the mock").list_drafts(page, max)
        }

        fn get_draft(&mut self, id: &RemoteId) -> Result<RemoteDraft, ComposeError> {
            self.0.lock().expect("the mock").get_draft(id)
        }

        fn draft_raw(&mut self, id: &RemoteId) -> Result<Vec<u8>, ComposeError> {
            if id == &self.1 {
                return Err(ComposeError::Provider(ProviderError::Api {
                    status: 422,
                    message: "the stored draft could not be read back".into(),
                }));
            }
            self.0.lock().expect("the mock").draft_raw(id)
        }

        fn create_draft(&mut self, mail: &OutgoingMail) -> Result<RemoteDraft, ComposeError> {
            self.0.lock().expect("the mock").create_draft(mail)
        }

        fn update_draft(
            &mut self,
            draft: &RemoteDraft,
            mail: &OutgoingMail,
        ) -> Result<RemoteDraft, ComposeError> {
            self.0.lock().expect("the mock").update_draft(draft, mail)
        }

        fn delete_draft(&mut self, id: &RemoteId) -> Result<(), ComposeError> {
            self.0.lock().expect("the mock").delete_draft(id)
        }

        fn send_draft(
            &mut self,
            draft: &RemoteDraft,
            mail: Option<&OutgoingMail>,
        ) -> Result<SendStatus, ComposeError> {
            self.0.lock().expect("the mock").send_draft(draft, mail)
        }

        fn find_sent(&mut self, message_id: &str) -> Result<Option<SentMail>, ComposeError> {
            self.0.lock().expect("the mock").find_sent(message_id)
        }

        fn list_attachments(
            &mut self,
            message: &RemoteId,
        ) -> Result<Vec<RemoteAttachment>, ComposeError> {
            self.0.lock().expect("the mock").list_attachments(message)
        }

        fn attachment_bytes(
            &mut self,
            message: &RemoteId,
            attachment: &RemoteAttachment,
        ) -> Result<AttachmentBytes, ComposeError> {
            self.0
                .lock()
                .expect("the mock")
                .attachment_bytes(message, attachment)
        }

        fn threading(&mut self, message: &RemoteId) -> Result<Threading, ComposeError> {
            self.0.lock().expect("the mock").threading(message)
        }
    }

    /// Swap `account`'s backend for one whose `unreadable` draft cannot be read back.
    fn unreadable_draft(
        worker: &Rig,
        account: &str,
        mock: &Arc<Mutex<MockCompose>>,
        unreadable: RemoteId,
    ) {
        let compose: Arc<Mutex<Box<dyn ComposeProvider>>> = Arc::new(Mutex::new(Box::new(
            UnreadableDrafts(Arc::clone(mock), unreadable),
        )));
        register_backend(worker, account, compose);
    }

    /// Swap `account`'s backend for one that delivers and then loses the answer.
    fn lose_send_answers(worker: &Rig, account: &str, mock: &Arc<Mutex<MockCompose>>) {
        let compose: Arc<Mutex<Box<dyn ComposeProvider>>> =
            Arc::new(Mutex::new(Box::new(LoseSendAnswers(Arc::clone(mock)))));
        register_backend(worker, account, compose);
    }

    /// Put `account`'s plain deterministic backend back, so a later reconciliation can
    /// find the message the lost answer hid.
    fn restore_backend(worker: &Rig, account: &str, mock: &Arc<Mutex<MockCompose>>) {
        let compose: Arc<Mutex<Box<dyn ComposeProvider>>> =
            Arc::new(Mutex::new(Box::new(SharedMock(Arc::clone(mock), false))));
        register_backend(worker, account, compose);
    }

    /// Swap an account's backend, keeping the address it is already configured to send
    /// from: the account *id* is not an address, so the registered one is what a `From:`
    /// header needs.
    fn register_backend(
        worker: &Rig,
        account: &str,
        backend: Arc<Mutex<Box<dyn ComposeProvider>>>,
    ) {
        let email = account_email(worker, account);
        let entry = ComposeAccount {
            backend: ComposeBackend::ComposeOnly(backend),
            email,
        };
        if let Ok(mut accounts) = worker.accounts.lock() {
            accounts.insert(account.to_owned(), entry);
        }
    }

    /// The address an account is registered to send from.
    fn account_email(worker: &Rig, account: &str) -> String {
        worker
            .accounts
            .lock()
            .ok()
            .and_then(|accounts| accounts.get(account).map(|entry| entry.email.clone()))
            .unwrap_or_else(|| SENDER.to_owned())
    }

    /// A rig over a *real* draft database, plus the path of that database, so a test can
    /// make one particular write fail the way a locked or full disk does. The path is
    /// needed because the failure is installed from a second connection, the way the disk
    /// itself would do it.
    fn worker_on_disk(tag: &str, now: Timestamp) -> (Rig, Arc<Mutex<MockCompose>>, PathBuf) {
        let root = Stores::scratch_root(tag);
        let _ = std::fs::remove_dir_all(&root);
        let (rig, mock) = rig(Stores::open(&root).expect("stores"), now);
        (rig, mock, root.join("drafts.sqlite"))
    }

    /// Make every write that would store `needle` in the record's state fail, exactly as a
    /// disk that cannot take the write does: SQLite aborts that statement and nothing else.
    ///
    /// Keying on the state rather than on a call count is what makes this deterministic —
    /// the same send reaches the same write every time, and the writes on either side of
    /// it still go through, which is what lets a test tell "the provider call happened and
    /// the write after it did not" from "nothing happened at all".
    fn refuse_state_writes(database: &std::path::Path, needle: &str) {
        let conn = rusqlite::Connection::open(database).expect("the draft database");
        // SQLite takes no bound parameters in a trigger's body, so the needle goes in as a
        // literal. It is a JSON fragment written by this test, never a quote.
        let trigger = format!(
            "CREATE TRIGGER IF NOT EXISTS refuse_state BEFORE UPDATE ON drafts
             WHEN NEW.json LIKE '%' || '{needle}' || '%'
             BEGIN SELECT RAISE(ABORT, 'injected: the disk would not take it'); END"
        );
        conn.execute(&trigger, []).expect("arm the write failure");
    }

    /// Let the writes through again, the way a disk that refused one write goes on
    /// accepting them: the failure is in the write, not in the row.
    fn allow_state_writes(database: &std::path::Path) {
        let conn = rusqlite::Connection::open(database).expect("the draft database");
        conn.execute("DROP TRIGGER IF EXISTS refuse_state", [])
            .expect("stop refusing writes");
    }

    /// A draft with one recipient, stored, as the UI would have it after a first save.
    fn stored(worker: &mut Rig, subject: &str, now: Timestamp) -> DraftRecord {
        let mut draft = DraftRecord::new_email(ACCOUNT, now);
        draft.to = OTHER.to_owned();
        draft.subject = subject.to_owned();
        draft.body.set_source("hello");
        match only(worker.handle(Command::Create {
            draft: Box::new(draft),
        })) {
            Event::Created(created) => *created,
            other => panic!("expected a created draft, got {other:?}"),
        }
    }

    #[test]
    fn discarding_a_mirrored_draft_removes_the_provider_copy_too() {
        let (mut worker, mock) = worker(100);
        let draft = stored(&mut worker, "Discard me", 100);
        only(worker.handle(Command::Sync { draft: draft.id }));
        let saved = worker
            .stores
            .drafts
            .get(draft.id)
            .expect("get")
            .expect("draft");
        let remote = saved.remote_id.expect("binding");
        assert!(
            matches!(only(worker.handle(Command::Discard(draft.id))), Event::Discarded(id) if id == draft.id)
        );
        assert!(worker.stores.drafts.get(draft.id).expect("get").is_none());
        assert!(matches!(
            mock.lock().expect("mock").get_draft(&remote),
            Err(ComposeError::NotFound(_))
        ));
    }

    #[test]
    fn discarding_does_not_delete_a_draft_changed_on_another_device() {
        let (mut worker, mock) = worker(100);
        let draft = stored(&mut worker, "Local", 100);
        only(worker.handle(Command::Sync { draft: draft.id }));
        let saved = worker
            .stores
            .drafts
            .get(draft.id)
            .expect("get")
            .expect("draft");
        let remote = saved.remote_id.expect("binding");
        write_behind_our_back(&mock, &remote, "Other device");
        assert!(matches!(
            only(worker.handle(Command::Discard(draft.id))),
            Event::Conflicted { .. }
        ));
        assert_eq!(
            mock.lock()
                .expect("mock")
                .get_draft(&remote)
                .expect("server draft")
                .subject,
            "Other device"
        );
        assert_eq!(
            worker
                .stores
                .drafts
                .get(draft.id)
                .expect("get")
                .expect("local draft")
                .subject,
            "Local"
        );
    }

    #[test]
    fn a_confirmed_delivery_cannot_become_uncertain_when_an_account_is_removed() {
        let (mut worker, _) = worker(100);
        let draft = stored(&mut worker, "Confirmed", 100);
        only(worker.handle(Command::Send {
            draft: draft.id,
            account: ACCOUNT.into(),
            now: 101,
        }));
        worker.accounts.lock().expect("accounts").clear();
        let Event::Reconciled { draft: checked, .. } = only(worker.handle(Command::Reconcile {
            draft: draft.id,
            account: ACCOUNT.into(),
        })) else {
            panic!("expected delivery state")
        };
        assert!(matches!(checked.state, DraftState::Sent { .. }));
        assert!(matches!(
            worker
                .stores
                .drafts
                .get(draft.id)
                .expect("get")
                .expect("draft")
                .state,
            DraftState::Sent { .. }
        ));
    }

    /// A unique path for a file a test hands the worker, so no two tests share one.
    fn scratch_file(name: &str) -> PathBuf {
        let mut path = Stores::scratch_root("picked");
        std::fs::create_dir_all(&path).expect("the scratch directory");
        path.push(name);
        path
    }

    fn only(events: Vec<Event>) -> Event {
        let mut events = events;
        assert_eq!(events.len(), 1, "exactly one answer per command");
        events.pop().expect("an answer")
    }

    fn fault(mock: &Arc<Mutex<MockCompose>>, fault: Option<MockFault>) {
        mock.lock().expect("the mock").set_fault(fault);
    }

    /// Replace the content of a remote draft behind this client's back, the way another
    /// device would: the same backend, a write this client never hears about.
    fn write_behind_our_back(mock: &Arc<Mutex<MockCompose>>, remote: &RemoteId, subject: &str) {
        let mail = OutgoingMail {
            rfc822: format!(
                "From: me@x.test\r\nTo: them@x.test\r\nSubject: {subject}\r\n\
                 Message-ID: <theirs@x.test>\r\n\r\ntheirs\r\n"
            )
            .into_bytes(),
            message_id: "theirs@x.test".into(),
            in_reply_to: None,
            references: Vec::new(),
            thread: None,
            account: ACCOUNT.into(),
        };
        let mut guard = mock.lock().expect("the mock");
        let current = guard.get_draft(remote).expect("the mirrored draft");
        guard
            .update_draft(&current, &mail)
            .expect("the other writer's update");
    }

    #[test]
    fn a_send_publishes_the_draft_first_and_delivers_it() {
        let (mut worker, mock) = worker(100);
        let draft = stored(&mut worker, "Lunch", 100);

        let event = only(worker.handle(Command::Sync { draft: draft.id }));
        let Event::Synced(synced, _remote) = event else {
            panic!("expected the draft to be mirrored, got {event:?}");
        };
        assert!(
            synced.remote_id.is_some(),
            "the draft is bound to the provider's copy"
        );
        assert_eq!(
            mock.lock().expect("the mock").draft_ids().len(),
            1,
            "the provider holds it"
        );

        let event = only(worker.handle(Command::Send {
            draft: synced.id,
            account: ACCOUNT.into(),
            now: 200,
        }));
        let Event::Sent {
            draft: sent,
            message,
            ..
        } = event
        else {
            panic!("expected a confirmed send, got {event:?}");
        };
        assert!(matches!(sent.state, DraftState::Sent { .. }));
        assert_eq!(
            message.from, SENDER,
            "the From: is the configured address, not the id"
        );
        assert_eq!(message.to, OTHER);
        assert!(
            mock.lock().expect("the mock").draft_ids().is_empty(),
            "sending the draft clears it on the provider"
        );
    }

    /// A bound draft that has vanished is the dangerous case: it may have been sent from
    /// the web, so the worker must not quietly make another and send it again.
    #[test]
    fn a_send_whose_bound_remote_draft_vanished_never_creates_another() {
        let (mut worker, mock) = worker(100);
        let draft = stored(&mut worker, "Lunch", 100);
        only(worker.handle(Command::Sync { draft: draft.id }));
        let bound = worker
            .stores
            .drafts
            .get(draft.id)
            .expect("get")
            .expect("present");
        let remote_id = bound.remote_id.clone().expect("remote");
        mock.lock()
            .expect("the mock")
            .delete_draft(&remote_id)
            .expect("deleted elsewhere");

        let event = only(worker.handle(Command::Send {
            draft: bound.id,
            account: ACCOUNT.into(),
            now: 200,
        }));
        let Event::Reconciled {
            draft: after,
            detail,
        } = event
        else {
            panic!("a vanished draft is reported as a reconciliation, not a send: {event:?}");
        };
        assert!(!detail.is_empty());
        assert_eq!(
            after.state,
            DraftState::Uncertain,
            "not sent again for the user"
        );
        assert!(
            !after.attempted,
            "and nothing of ours was ever requested, so nothing is retried"
        );
        assert!(
            mock.lock().expect("the mock").log().sent.is_empty(),
            "this client sent nothing at all"
        );
        assert_eq!(
            after.remote_id.as_deref(),
            Some(remote_id.as_str()),
            "the binding is kept, so the disappearance is not hidden"
        );
        assert!(
            mock.lock().expect("the mock").draft_ids().is_empty(),
            "nothing was created in its place: a second copy is how one lands in an inbox twice"
        );
    }

    /// The same disappearance, seen from a mirror rather than a send: the autosync must
    /// report it and keep the binding rather than recreating the draft behind the user's
    /// back.
    #[test]
    fn a_sync_whose_bound_remote_draft_vanished_reports_instead_of_recreating() {
        let (mut worker, mock) = worker(100);
        let draft = stored(&mut worker, "Lunch", 100);
        only(worker.handle(Command::Sync { draft: draft.id }));
        let bound = worker
            .stores
            .drafts
            .get(draft.id)
            .expect("get")
            .expect("present");
        let remote_id = bound.remote_id.clone().expect("remote");
        mock.lock()
            .expect("the mock")
            .delete_draft(&remote_id)
            .expect("deleted elsewhere");

        let event = only(worker.handle(Command::Sync { draft: bound.id }));
        let Event::AutosaveFailed(id, detail) = event else {
            panic!("the disappearance has to be visible, not papered over: {event:?}");
        };
        assert_eq!(id, bound.id);
        assert!(!detail.is_empty());
        assert!(
            mock.lock().expect("the mock").draft_ids().is_empty(),
            "still not recreated"
        );
        let after = worker
            .stores
            .drafts
            .get(bound.id)
            .expect("get")
            .expect("present");
        assert_eq!(after.remote_id.as_deref(), Some(remote_id.as_str()));
        assert_eq!(after.revision, bound.revision, "and nothing was written");
    }

    /// If the message *was* sent from the web, the provider can prove it: the send is
    /// confirmed rather than repeated, and nothing is put in the outbox twice.
    #[test]
    fn a_vanished_draft_whose_message_is_in_sent_is_confirmed_not_resent() {
        let (mut worker, mock) = worker(100);
        let first = stored(&mut worker, "Lunch", 100);
        only(worker.handle(Command::Sync { draft: first.id }));
        let first = worker
            .stores
            .drafts
            .get(first.id)
            .expect("get")
            .expect("present");
        let sent_elsewhere = only(worker.handle(Command::Send {
            draft: first.id,
            account: ACCOUNT.into(),
            now: 150,
        }));
        assert!(matches!(sent_elsewhere, Event::Sent { .. }));

        // A second draft carrying the same Message-ID, whose server copy has been deleted.
        let mut second = DraftRecord::new_email(ACCOUNT, 160);
        second.to = OTHER.to_owned();
        second.subject = "Lunch".into();
        second.message_id = first.message_id.clone();
        let created = only(worker.handle(Command::Create {
            draft: Box::new(second),
        }));
        let Event::Created(second) = created else {
            panic!("created")
        };
        only(worker.handle(Command::Sync { draft: second.id }));
        let second = worker
            .stores
            .drafts
            .get(second.id)
            .expect("get")
            .expect("present");
        let remote_id = second.remote_id.clone().expect("remote");
        mock.lock()
            .expect("the mock")
            .delete_draft(&remote_id)
            .expect("deleted elsewhere");

        let event = only(worker.handle(Command::Send {
            draft: second.id,
            account: ACCOUNT.into(),
            now: 250,
        }));
        let Event::Sent {
            draft: settled,
            sent,
            ..
        } = event
        else {
            panic!("the provider can prove this one went out: {event:?}");
        };
        assert!(matches!(settled.state, DraftState::Sent { .. }));
        assert_eq!(
            sent.message_id, first.message_id,
            "the message the provider recorded"
        );
        assert!(!sent.id.is_empty(), "with the server's own sent id");
        assert!(
            mock.lock().expect("the mock").draft_ids().is_empty(),
            "nothing is left on the server: the first draft was sent, the second was deleted \
             elsewhere, and the reconciliation created nothing"
        );
    }

    /// A send whose answer was lost is not a send that failed: the message left, the
    /// record says so, and the provider can prove it. The confirmed attempt is reported
    /// exactly like a directly observed send — the same event, the provider's own record of
    /// the message — and nothing is sent a second time.
    #[test]
    fn an_unreadable_send_is_confirmed_later_by_the_message_it_left() {
        // The provider's own clock is the send time, so what it records as the delivery
        // time is what this test reads back.
        let (mut worker, mock) = worker(200);
        let draft = stored(&mut worker, "Lunch", 100);
        // The request went out; the answer did not come back.
        lose_send_answers(&worker, ACCOUNT, &mock);

        let event = only(worker.handle(Command::Send {
            draft: draft.id,
            account: ACCOUNT.into(),
            now: 200,
        }));
        let Event::Uncertain(after, _) = event else {
            panic!("a lost answer must be uncertain, not a failure: {event:?}");
        };
        assert_eq!(after.state, DraftState::Uncertain);
        assert!(
            after.attempted,
            "the attempt is recorded, so it can be reconciled"
        );
        let message_id = after.message_id.clone();

        restore_backend(&worker, ACCOUNT, &mock);
        let event = only(worker.handle(Command::Reconcile {
            draft: after.id,
            account: ACCOUNT.into(),
        }));
        let Event::Sent {
            draft: settled,
            sent,
            message,
        } = event
        else {
            panic!("the provider can prove this one went out: {event:?}");
        };
        assert_eq!(
            settled.state,
            DraftState::Sent {
                remote_id: sent.id.clone()
            },
            "and the draft is a sent draft, keyed by the server's own id"
        );
        assert_eq!(
            settled.message_id, message_id,
            "the Message-ID it was reconciled by"
        );
        assert!(
            !sent.id.is_empty(),
            "the server's sent id is kept, not dropped"
        );
        assert!(!sent.thread.is_empty(), "and so is its thread");
        assert!(
            sent.sent_at >= 200,
            "and when the provider says it was sent"
        );
        assert_eq!(
            message.from, SENDER,
            "the outbox shows what actually went out"
        );
        assert_eq!(message.subject, "Lunch");
        assert_eq!(
            mock.lock().expect("the mock").log().sent.len(),
            1,
            "and nothing was sent a second time"
        );
    }

    /// A lookup that finds nothing proves nothing: the draft stays uncertain, is not sent
    /// again, and says why.
    #[test]
    fn a_negative_check_leaves_an_uncertain_draft_uncertain() {
        let (mut worker, mock) = worker(100);
        // An attempt recovered from a crash: the request may have been issued, and the
        // provider has no record of it.
        let mut draft = stored(&mut worker, "Lunch", 100);
        draft.state = DraftState::Uncertain;
        draft.note_send_attempt();
        let expected = draft.revision;
        let draft = worker
            .stores
            .drafts
            .save(&mut draft, expected)
            .expect("save");

        let event = only(worker.handle(Command::Reconcile {
            draft: draft.id,
            account: ACCOUNT.into(),
        }));
        let Event::Reconciled {
            draft: checked,
            detail,
        } = event
        else {
            panic!("an unconfirmed attempt is not a send: {event:?}");
        };
        assert!(!detail.is_empty(), "and it says why");
        assert_eq!(checked.state, DraftState::Uncertain);
        assert!(checked.attempted, "still recorded as attempted");
        assert!(
            mock.lock().expect("the mock").log().sent.is_empty(),
            "and the worker never sends it for the user"
        );
    }

    #[test]
    fn a_lost_draft_write_is_never_reported_as_a_delivery_in_doubt() {
        let (mut worker, mock) = worker(100);
        let draft = stored(&mut worker, "Lunch", 100);
        fault(&mock, Some(MockFault::LostResponse));

        let event = only(worker.handle(Command::Send {
            draft: draft.id,
            account: ACCOUNT.into(),
            now: 200,
        }));
        let Event::AutosaveFailed(id, detail) = event else {
            panic!("an interrupted draft write is not a delivery in doubt: {event:?}");
        };
        assert_eq!(id, draft.id, "the draft the status belongs to is named");
        assert!(!detail.is_empty());
        let after = worker
            .stores
            .drafts
            .get(draft.id)
            .expect("get")
            .expect("present");
        assert!(!after.attempted, "no send was ever requested");
        assert_eq!(
            after.state,
            DraftState::Editing,
            "and the draft is editable as it was"
        );
    }

    #[test]
    fn an_offline_send_leaves_the_draft_as_it_was_and_schedules_nothing() {
        let (mut worker, mock) = worker(100);
        let draft = stored(&mut worker, "Lunch", 100);
        fault(&mock, Some(MockFault::Offline));

        let event = only(worker.handle(Command::Send {
            draft: draft.id,
            account: ACCOUNT.into(),
            now: 200,
        }));
        let Event::Offline(offline) = event else {
            panic!("an offline send is reported as offline: {event:?}");
        };
        assert_eq!(offline.subject, "Lunch", "the draft is exactly as it was");
        assert!(
            !offline.attempted,
            "nothing was attempted, so there is nothing to reconcile"
        );
        assert!(
            worker
                .stores
                .drafts
                .pending_holds()
                .expect("holds")
                .is_empty(),
            "no hold survives an offline send, so nothing can go out later by itself"
        );
    }

    #[test]
    fn a_stale_editor_token_keeps_the_stored_copy_and_hands_it_back() {
        let (mut worker, _mock) = worker(100);
        let draft = stored(&mut worker, "first", 100);

        let mut edit = draft.clone();
        edit.subject = "second".into();
        only(worker.handle(Command::Save {
            draft: Box::new(edit),
            expected: draft.editor_revision,
        }));

        // A second surface still holds the editor revision it started from.
        let mut late = draft.clone();
        late.subject = "clobbered".into();
        let event = only(worker.handle(Command::Save {
            draft: Box::new(late),
            expected: draft.editor_revision,
        }));
        let Event::SaveRejected(stored) = event else {
            panic!("a stale save must be refused, got {event:?}");
        };
        assert_eq!(
            stored.subject, "second",
            "the newer copy wins and is handed back"
        );
        assert!(stored.revision > draft.revision);
    }

    /// A file copy or a finished mirror moves the row on without touching anything the
    /// user typed. The next keystroke must not be refused over a revision the user never
    /// saw, or their work is lost to bookkeeping.
    #[test]
    fn a_save_survives_a_row_moved_by_attachments_and_mirroring() {
        let (mut worker, _mock) = worker(100);
        let draft = stored(&mut worker, "Lunch", 100);

        // The row moves twice under the editor's feet: once by a mirror that binds remote
        // ids, once by a picked file arriving. Neither is an edit the surface could know
        // about.
        only(worker.handle(Command::Sync { draft: draft.id }));
        let picked = scratch_file("moved-by-attachment.txt");
        std::fs::write(&picked, b"a file the user picked").expect("write the picked file");
        let Event::Attached {
            draft: with_file,
            failed,
        } = only(worker.handle(Command::Attach {
            draft: draft.id,
            files: vec![picked],
        }))
        else {
            panic!("the file lands on the draft");
        };
        assert!(failed.is_empty(), "the file was copied: {failed:?}");
        assert_eq!(with_file.attachments.len(), 1);
        assert!(
            with_file.revision > draft.revision,
            "the row really did move on"
        );

        let mut edit = draft.clone();
        edit.body.set_source("typed after the file arrived");
        let event = only(worker.handle(Command::Save {
            draft: Box::new(edit),
            expected: draft.editor_revision,
        }));
        let Event::Saved(saved) = event else {
            panic!("the editor's own token is still current: {event:?}");
        };
        assert_eq!(saved.body.source, "typed after the file arrived");
        assert_eq!(
            saved.attachments.len(),
            1,
            "and the file the user never touched is still attached"
        );
    }

    #[test]
    fn a_remote_draft_that_moved_on_is_refused_rather_than_overwritten() {
        let (mut worker, mock) = worker(100);
        let draft = stored(&mut worker, "Lunch", 100);
        only(worker.handle(Command::Sync { draft: draft.id }));
        let local = worker
            .stores
            .drafts
            .get(draft.id)
            .expect("get")
            .expect("present");
        write_behind_our_back(&mock, local.remote_id.as_ref().expect("remote"), "Theirs");

        let event = only(worker.handle(Command::Sync { draft: local.id }));
        let Event::Conflicted(kept, current) = event else {
            panic!("a moved-on draft must be reported, not overwritten: {event:?}");
        };
        assert_eq!(kept.state, DraftState::Conflict);
        assert_eq!(
            kept.subject, "Lunch",
            "the local copy is kept as the user wrote it"
        );
        assert_eq!(
            current.subject, "Theirs",
            "the provider's current copy comes back too"
        );
        let after = worker
            .stores
            .drafts
            .get(local.id)
            .expect("get")
            .expect("present");
        assert_eq!(after.subject, "Lunch");
    }

    #[test]
    fn keeping_the_local_copy_adopts_the_newer_revision_and_pushes_it() {
        let (mut worker, mock) = worker(100);
        let draft = stored(&mut worker, "Mine", 100);
        only(worker.handle(Command::Sync { draft: draft.id }));
        let local = worker
            .stores
            .drafts
            .get(draft.id)
            .expect("get")
            .expect("present");
        let remote_id = local.remote_id.clone().expect("remote");
        write_behind_our_back(&mock, &remote_id, "Theirs");
        // What the provider holds now, so the push below is known to be against a
        // revision that has moved on since this client last wrote.

        let event = only(worker.handle(Command::Resolve {
            draft: local.id,
            account: ACCOUNT.into(),
            mode: ResolveMode::KeepLocal,
        }));
        let Event::Resolved { draft: kept, mode } = event else {
            panic!("expected the resolution to be reported, got {event:?}");
        };
        assert_eq!(mode, ResolveMode::KeepLocal);
        assert_eq!(kept.subject, "Mine", "the local content is untouched");
        let on_server = mock
            .lock()
            .expect("the mock")
            .get_draft(&remote_id)
            .expect("kept");
        assert_eq!(
            kept.remote_revision.as_deref(),
            Some(on_server.revision.as_str()),
            "the binding names the provider's copy as it is after the push, not the one \
             that was read before it"
        );
        assert_eq!(
            on_server.subject, "Mine",
            "and our content is what the server holds now"
        );
        assert_eq!(
            on_server.id, remote_id,
            "the server's draft was replaced, not deleted"
        );
    }

    #[test]
    fn taking_the_remote_replaces_the_same_local_draft() {
        let (mut worker, mock) = worker(100);
        seed_remote_draft(&mock);
        let mut draft = adopt_seeded(&mut worker);
        draft.subject = "Mine".into();
        draft.body.set_source("my words");
        let expected = draft.revision;
        let draft = worker
            .stores
            .drafts
            .save(&mut draft, expected)
            .expect("save");
        let remote_id = draft.remote_id.clone().expect("remote");

        let event = only(worker.handle(Command::Resolve {
            draft: draft.id,
            account: ACCOUNT.into(),
            mode: ResolveMode::TakeRemote,
        }));
        let Event::Resolved { draft: taken, .. } = event else {
            panic!("expected the resolution to be reported, got {event:?}");
        };
        assert_eq!(
            taken.id, draft.id,
            "the surface the user was looking at stays put"
        );
        assert_eq!(
            taken.subject, "Rich",
            "the server's content is what the draft now holds"
        );
        assert!(
            taken.body.original.is_some(),
            "with its rich original preserved"
        );
        assert_eq!(
            taken.message_id, SEEDED_MESSAGE_ID,
            "and the id that can be reconciled"
        );
        assert!(
            taken.attachments.iter().any(|a| a.file_name == "note.txt"),
            "and its attachments: {:?}",
            attachment_names(&taken.attachments)
        );
        assert_eq!(
            mock.lock()
                .expect("the mock")
                .get_draft(&remote_id)
                .expect("still there")
                .subject,
            "Rich",
            "nothing was deleted on the server"
        );
    }

    #[test]
    fn keeping_both_leaves_two_drafts_that_cannot_be_confused() {
        let (mut worker, mock) = worker(100);
        seed_remote_draft(&mock);
        let mut draft = adopt_seeded(&mut worker);
        draft.subject = "Mine".into();
        draft.body.set_source("my words");
        let expected = draft.revision;
        let draft = worker
            .stores
            .drafts
            .save(&mut draft, expected)
            .expect("save");
        let remote_id = draft.remote_id.clone().expect("remote");

        let event = only(worker.handle(Command::Resolve {
            draft: draft.id,
            account: ACCOUNT.into(),
            mode: ResolveMode::KeepBoth,
        }));
        let Event::ResolvedBoth { bound, copy } = event else {
            panic!("expected both drafts, got {event:?}");
        };
        assert_eq!(bound.id, draft.id, "the bound id the user was looking at");
        assert_ne!(copy.id, bound.id, "and a new one beside it");
        assert_ne!(
            copy.message_id, bound.message_id,
            "two sends must never share an id"
        );
        assert_eq!(
            bound.subject, "Rich",
            "the bound draft carries the server's content"
        );
        assert_eq!(
            copy.subject, "Mine",
            "and the copy carries what the user wrote"
        );
        assert!(
            copy.body.source.contains("my words"),
            "including the edited body"
        );
        assert!(
            copy.body.original.is_some(),
            "the rich original it came from is kept"
        );
        assert!(copy.remote_id.is_none(), "the copy is unambiguously local");
        assert!(copy.remote_revision.is_none());
        assert_eq!(copy.state, DraftState::Editing);
        assert_eq!(
            copy.account, bound.account,
            "both belong to the same account"
        );
        // The copy carries its own Message-ID, so it can never be mistaken for the server's
        // copy and raw-sent as if it were the original message.
        let theirs = mock
            .lock()
            .expect("the mock")
            .get_draft(&remote_id)
            .expect("their copy");
        assert!(
            !worker.still_theirs(&copy, &theirs),
            "a fresh id is never their identity"
        );
        assert_eq!(
            mock.lock()
                .expect("the mock")
                .get_draft(&remote_id)
                .expect("still there")
                .subject,
            "Rich",
            "nothing was deleted on the server"
        );
    }

    #[test]
    fn a_resolution_that_cannot_read_the_remote_changes_nothing() {
        let (mut worker, mock) = worker(100);
        let draft = stored(&mut worker, "Lunch", 100);
        only(worker.handle(Command::Sync { draft: draft.id }));
        let before = worker
            .stores
            .drafts
            .get(draft.id)
            .expect("get")
            .expect("present");
        fault(&mock, Some(MockFault::Rejected));

        let event = only(worker.handle(Command::Resolve {
            draft: draft.id,
            account: ACCOUNT.into(),
            mode: ResolveMode::TakeRemote,
        }));
        let Event::ResolveFailed(id, detail) = event else {
            panic!("a failed resolution is reported, not guessed at: {event:?}");
        };
        assert_eq!(id, draft.id);
        assert!(!detail.is_empty());
        let after = worker
            .stores
            .drafts
            .get(draft.id)
            .expect("get")
            .expect("present");
        assert_eq!(after.subject, before.subject);
        assert_eq!(after.revision, before.revision, "and nothing was written");
    }

    #[test]
    fn resolving_an_unbound_draft_is_refused() {
        let (mut worker, _mock) = worker(100);
        let draft = stored(&mut worker, "Lunch", 100);

        let event = only(worker.handle(Command::Resolve {
            draft: draft.id,
            account: ACCOUNT.into(),
            mode: ResolveMode::KeepBoth,
        }));
        let Event::ResolveFailed(id, _) = event else {
            panic!("there is nothing to resolve on an unbound draft: {event:?}");
        };
        assert_eq!(id, draft.id);
    }

    #[test]
    fn an_untouched_rich_import_goes_out_as_the_provider_still_holds_it() {
        let (mut worker, mock) = worker(100);
        seed_remote_draft(&mock);
        let draft = adopt_seeded(&mut worker);
        assert_eq!(
            draft.to, OTHER,
            "the decoded headers came from the stored bytes"
        );
        assert!(
            draft.body.original.is_some(),
            "the rich original is preserved"
        );
        assert!(
            draft.body.needs_conversion(),
            "an unapproved body is not Markdown yet"
        );
        assert_eq!(
            draft.message_id, SEEDED_MESSAGE_ID,
            "their Message-ID is kept"
        );
        let remote = draft.remote_id.clone().map(|id| {
            mock.lock()
                .expect("the mock")
                .get_draft(&id)
                .expect("their draft")
        });
        assert!(
            remote
                .as_ref()
                .is_some_and(|remote| worker.still_theirs(&draft, remote)),
            "an unedited import is still theirs, so nothing rebuilds it"
        );
        assert!(
            draft.attachments.iter().any(|a| a.file_name == "note.txt"),
            "the files of the stored message are carried over: {:?}",
            attachment_names(&draft.attachments)
        );

        let event = only(worker.handle(Command::Send {
            draft: draft.id,
            account: ACCOUNT.into(),
            now: 300,
        }));
        let Event::Sent {
            draft: sent,
            message,
            ..
        } = event
        else {
            panic!("expected a confirmed send, got {event:?}");
        };
        assert!(matches!(sent.state, DraftState::Sent { .. }));
        assert_eq!(
            sent.message_id, SEEDED_MESSAGE_ID,
            "their bytes went out, so the send is still reconcilable"
        );
        assert!(
            message.html.contains("<img"),
            "the original HTML went out as it is"
        );
        assert!(
            mock.lock().expect("the mock").log().updated.is_empty(),
            "their bytes went out untouched: the server's copy was never rewritten"
        );
    }

    /// A provider that cannot state the stored message's identity must not have its bytes
    /// sent on trust: the send is rebuilt under the id this draft knows, which is the only
    /// one reconciliation can use.
    #[test]
    fn a_remote_identity_the_provider_cannot_state_is_rebuilt_under_our_own_id() {
        let (mut worker, mock) = worker(100);
        seed_remote_draft(&mock);
        let draft = adopt_seeded(&mut worker);
        hide_remote_identity(&worker, ACCOUNT, &mock);

        // Read the provider's copy the way the worker does, through the wrapper that
        // cannot state its identity — that is the whole point of the arrangement.
        let remote = worker.current_remote(&draft, ACCOUNT).expect("their copy");
        assert!(
            !worker.still_theirs(&draft, &remote),
            "an id we cannot confirm is not an id we may send their bytes under"
        );

        let event = only(worker.handle(Command::Send {
            draft: draft.id,
            account: ACCOUNT.into(),
            now: 300,
        }));
        let Event::Sent {
            draft: sent,
            sent: mail,
            ..
        } = event
        else {
            panic!("expected a confirmed send, got {event:?}");
        };
        assert!(
            !mock.lock().expect("the mock").log().updated.is_empty(),
            "the payload was rebuilt"
        );
        // The provider reads the id out of the message it actually emitted, so this is a
        // statement about the bytes that went out: a lost answer is still reconcilable.
        assert_eq!(sent.message_id, mail.message_id);
        assert_eq!(
            mail.message_id, draft.message_id,
            "and it is the id this draft knows"
        );
    }

    /// A file replaced by different bytes under the same name is not the same file: the
    /// send must carry the new bytes, not the ones the server happens to still hold.
    #[test]
    fn a_replaced_file_is_sent_with_its_new_bytes() {
        let (mut worker, mock) = worker(100);
        seed_remote_draft(&mock);
        let draft = adopt_seeded(&mut worker);
        let imported = draft
            .attachments
            .iter()
            .find(|a| a.file_name == "note.txt")
            .expect("the imported file")
            .id
            .clone();
        let replacement = worker
            .stores
            .attachments
            .import_bytes(
                "note.txt",
                b"my own note",
                AttachmentSource::Added {
                    origin: PathBuf::from("/tmp/note.txt"),
                },
                None,
            )
            .expect("store the replacement");
        let mut swapped = draft.clone();
        assert!(swapped.detach(&imported));
        swapped.attach(replacement.clone());
        let expected = swapped.revision;
        let draft = worker
            .stores
            .drafts
            .save(&mut swapped, expected)
            .expect("save");

        let remote = mock
            .lock()
            .expect("the mock")
            .get_draft(draft.remote_id.as_ref().expect("remote"))
            .expect("their copy");
        assert!(
            !worker.still_theirs(&draft, &remote),
            "same name, different bytes: their copy is no longer this draft"
        );

        let event = only(worker.handle(Command::Send {
            draft: draft.id,
            account: ACCOUNT.into(),
            now: 300,
        }));
        let Event::Sent {
            draft: sent,
            sent: mail,
            ..
        } = event
        else {
            panic!("expected a confirmed send, got {event:?}");
        };
        assert!(matches!(sent.state, DraftState::Sent { .. }));
        assert_eq!(
            mock.lock().expect("the mock").log().sent.len(),
            1,
            "one delivery"
        );
        // What the recipient actually receives, read back out of the sent message: the
        // local record would only echo what we already had locally.
        let note = mock
            .lock()
            .expect("the mock")
            .list_attachments(&mail.id)
            .expect("the sent message carries its files")
            .into_iter()
            .find(|part| part.name == "note.txt")
            .expect("the file went out");
        let delivered = mock
            .lock()
            .expect("the mock")
            .attachment_bytes(&mail.id, &note)
            .expect("its bytes")
            .bytes;
        assert_eq!(
            delivered, b"my own note",
            "the new bytes are what the recipient gets"
        );
    }

    #[test]
    fn a_header_edit_sends_the_original_body_with_the_new_headers() {
        let (mut worker, mock) = worker(100);
        seed_remote_draft(&mock);
        let mut draft = adopt_seeded(&mut worker);
        draft.set_subject("Re: Rich");
        let expected = draft.revision;
        let draft = worker
            .stores
            .drafts
            .save(&mut draft, expected)
            .expect("save");

        let event = only(worker.handle(Command::Send {
            draft: draft.id,
            account: ACCOUNT.into(),
            now: 300,
        }));
        let Event::Sent { message, .. } = event else {
            panic!("expected a confirmed send, got {event:?}");
        };
        assert_eq!(
            message.subject, "Re: Rich",
            "the edited header is what goes out"
        );
        assert!(
            message.html.contains("<img"),
            "and the preserved original body with it"
        );
        assert!(
            mock.lock().expect("the mock").draft_ids().is_empty(),
            "the sent draft is gone from the provider"
        );
    }

    #[test]
    fn an_import_that_cannot_be_read_is_never_partially_adopted() {
        let (mut worker, mock) = worker(100);
        let seeded = seed_remote_draft(&mock);
        unreadable_draft(&worker, ACCOUNT, &mock, seeded);

        let event = only(worker.handle(Command::Import {
            account: ACCOUNT.into(),
            limit: 10,
        }));
        let Event::Imported {
            account,
            fresh,
            failed,
            ..
        } = event
        else {
            panic!("expected an import report, got {event:?}");
        };
        assert_eq!(
            account, ACCOUNT,
            "the report names whose drafts it is about"
        );
        assert!(
            fresh.is_empty(),
            "a draft that could not be read is not adopted at all"
        );
        assert_eq!(failed.len(), 1, "and the failure is reported: {failed:?}");
        assert!(!failed[0].detail.is_empty());
        assert!(worker.stores.drafts.list().expect("list").is_empty());
    }

    #[test]
    fn a_restart_brings_every_draft_back_without_sending_anything() {
        let (mut worker, _mock) = worker(100);
        let held = stored(&mut worker, "held", 100);
        only(worker.handle(Command::BeginHold {
            draft: held.id,
            now: 100,
            at: None,
        }));
        // A send that was in flight when the app stopped: persisted as attempted, exactly
        // as a crash between the write and the provider's answer would leave it.
        let mut flying = stored(&mut worker, "flying", 150);
        flying.state = DraftState::Sending;
        flying.note_send_attempt();
        let expected = flying.revision;
        worker
            .stores
            .drafts
            .save(&mut flying, expected)
            .expect("save");

        let event = only(worker.handle(Command::Recover { now: 10_000 }));
        let Event::Recovered(recovery, drafts) = event else {
            panic!("expected a recovery report, got {event:?}");
        };
        assert_eq!(
            recovery.restored,
            vec![held.id],
            "a hold never left the app"
        );
        assert_eq!(
            recovery.uncertain,
            vec![flying.id],
            "an in-flight send needs reconciling"
        );
        let mut subjects: Vec<String> = drafts.iter().map(|d| d.subject.clone()).collect();
        subjects.sort();
        assert_eq!(
            subjects,
            vec!["flying".to_owned(), "held".to_owned()],
            "every draft is back"
        );
        let back = worker
            .stores
            .drafts
            .get(held.id)
            .expect("get")
            .expect("present");
        assert_eq!(back.state, DraftState::Editing, "and editable, not sent");
    }

    /// Send later is a promise: a restart keeps the draft held until its time instead of
    /// handing it back as an editable draft, and it goes out once it is due.
    #[test]
    fn a_send_later_hold_survives_a_restart_and_sends_when_due() {
        let (mut worker, _mock) = worker(100);
        let draft = stored(&mut worker, "later", 100);
        let Event::Held(hold) = only(worker.handle(Command::BeginHold {
            draft: draft.id,
            now: 100,
            at: Some(5_000),
        })) else {
            panic!("held");
        };
        assert_eq!(hold.hold_until, 5_000);

        let Event::Recovered(recovery, _) = only(worker.handle(Command::Recover { now: 200 }))
        else {
            panic!("expected a recovery report");
        };
        assert_eq!(recovery.scheduled, vec![hold.clone()]);
        assert!(recovery.restored.is_empty());
        let back = worker.stores.drafts.get(draft.id).expect("get").expect("present");
        assert_eq!(back.state, DraftState::Held, "still waiting, not editable");
        assert_eq!(worker.stores.drafts.pending_holds().expect("holds"), vec![hold.clone()]);

        let event = only(worker.handle(Command::Send {
            draft: draft.id,
            account: ACCOUNT.to_owned(),
            now: 5_000,
        }));
        assert!(matches!(event, Event::Sent { .. }), "{event:?}");
    }

    /// Store a provider draft with a rich body, an inline image and a file attachment,
    /// exactly as a remote draft is found on the server.
    fn seed_remote_draft(mock: &Arc<Mutex<MockCompose>>) -> RemoteId {
        let raw = concat!(
            "From: me@x.test\r\n",
            "To: you@x.test\r\n",
            "Subject: Rich\r\n",
            "Message-ID: <seeded@x.test>\r\n",
            "MIME-Version: 1.0\r\n",
            "Content-Type: multipart/mixed; boundary=\"outer\"\r\n",
            "\r\n",
            "--outer\r\n",
            "Content-Type: text/html\r\n",
            "\r\n",
            "<p>hello <img src=\"cid:pic1\"></p>\r\n",
            "--outer\r\n",
            "Content-Type: image/png\r\n",
            "Content-ID: <pic1>\r\n",
            "Content-Disposition: inline; filename=\"pic1.png\"\r\n",
            "\r\n",
            "PNG-not-really\r\n",
            "--outer\r\n",
            "Content-Type: text/plain; name=\"note.txt\"\r\n",
            "Content-Disposition: attachment; filename=\"note.txt\"\r\n",
            "\r\n",
            "a note\r\n",
            "--outer--\r\n",
        );
        let mail = OutgoingMail {
            rfc822: raw.as_bytes().to_vec(),
            message_id: SEEDED_MESSAGE_ID.to_owned(),
            in_reply_to: None,
            references: Vec::new(),
            thread: None,
            account: ACCOUNT.to_owned(),
        };
        mock.lock()
            .expect("the mock")
            .create_draft(&mail)
            .expect("seed")
            .id
    }

    /// Import the seeded provider draft and hand back the local record.
    fn adopt_seeded(worker: &mut Rig) -> DraftRecord {
        let event = only(worker.handle(Command::Import {
            account: ACCOUNT.into(),
            limit: 10,
        }));
        let Event::Imported { account, fresh, .. } = event else {
            panic!("expected an import, got {event:?}");
        };
        assert_eq!(
            account, ACCOUNT,
            "the report names whose drafts it is about"
        );
        fresh.into_iter().next().expect("one adopted draft")
    }

    /// A remote id is scoped to one account. Two accounts' providers hand out the same
    /// id, and the second one must still be imported as its own draft rather than
    /// mistaken for the first one's.
    #[test]
    fn the_same_remote_id_in_two_accounts_stays_two_drafts() {
        const SECOND: &str = "work";
        const SECOND_SENDER: &str = "them@x.test";
        let (mut worker, mine) = worker(100);
        let theirs = add_account(&worker, SECOND, SECOND_SENDER, 100);
        let my_id = seed_remote_draft(&mine);
        let their_id = seed_remote_draft(&theirs);
        assert_eq!(
            my_id, their_id,
            "the two providers really do hand out the same id"
        );

        for (account, expected) in [(ACCOUNT, my_id.as_str()), (SECOND, their_id.as_str())] {
            let event = only(worker.handle(Command::Import {
                account: account.into(),
                limit: 10,
            }));
            let Event::Imported {
                account: reported,
                fresh,
                already_local,
                failed,
            } = event
            else {
                panic!("expected an import report, got {event:?}");
            };
            assert_eq!(reported, account);
            assert!(failed.is_empty(), "{failed:?}");
            assert!(
                already_local.is_empty(),
                "another account's draft is not this one's"
            );
            let adopted = fresh.into_iter().next().expect("one adopted draft");
            assert_eq!(
                adopted.account, account,
                "adopted for the account it came from"
            );
            assert_eq!(adopted.remote_id.as_deref(), Some(expected));
        }

        let all = worker.stores.drafts.list().expect("list");
        assert_eq!(
            all.len(),
            2,
            "one draft per account, both bound to the same remote id"
        );
        assert_eq!(all[0].message_id, SEEDED_MESSAGE_ID);
        assert_ne!(all[0].id, all[1].id, "and never the same local draft");
    }
    /// A refresh of the account list must not hand a signed-in account a brand new
    /// backend — the deterministic one holds the sent mail an uncertain send is still
    /// reconciled against — and an account that is no longer listed must stop answering
    /// altogether.
    #[test]
    fn a_refresh_keeps_a_compose_only_backend_and_drops_a_removed_account() {
        const SECOND: &str = "work";
        const SECOND_SENDER: &str = "them@x.test";
        let stores = Stores::open_in_memory().expect("stores");
        let client = Client::start(stores, Accounts::new());
        client.register(ACCOUNT, ComposeAccount::mock(100, SENDER.to_owned()));
        client.register(SECOND, ComposeAccount::mock(100, SECOND_SENDER.to_owned()));

        // A send on the first account, so the backend has something only it remembers.
        let mut draft = DraftRecord::new_email(ACCOUNT, 100);
        draft.to = OTHER.to_owned();
        draft.subject = "Lunch".into();
        draft.body.set_source("hello");
        client.send(Command::Create {
            draft: Box::new(draft),
        });
        let Event::Created(created) = wait_for(&client, |event| matches!(event, Event::Created(_)))
        else {
            unreachable!("filtered")
        };
        client.send(Command::Send {
            draft: created.id,
            account: ACCOUNT.into(),
            now: 200,
        });
        let sent = wait_for(&client, |event| matches!(event, Event::Sent { .. }));
        let Event::Sent { sent, .. } = sent else {
            unreachable!("filtered")
        };

        // What the UI does on every refresh: rebuild the list and hand it over whole.
        let mut refreshed = Accounts::new();
        refreshed.insert(
            ACCOUNT.to_owned(),
            ComposeAccount::mock(100, SENDER.to_owned()),
        );
        client.update_accounts(refreshed);

        assert!(
            client.has_account(ACCOUNT),
            "the account is still signed in"
        );
        assert!(
            !client.has_account(SECOND),
            "and the one that is gone cannot answer at all"
        );
        client.send(Command::Probe {
            account: SECOND.into(),
        });
        let event = wait_for(
            &client,
            |event| matches!(event, Event::Unsupported { account } if account == SECOND),
        );
        assert!(
            matches!(event, Event::Unsupported { .. }),
            "a removed account cannot compose: {event:?}"
        );

        // The sent message the backend already recorded is still there: reconciliation
        // against it still works after the refresh.
        client.send(Command::Probe {
            account: ACCOUNT.into(),
        });
        let probe = wait_for(&client, |event| matches!(event, Event::Composable { .. }));
        assert!(matches!(probe, Event::Composable { .. }));
        let mut next = DraftRecord::new_email(ACCOUNT, 201);
        next.to = OTHER.into();
        next.body.set_source("the next message");
        client.send(Command::Create {
            draft: Box::new(next),
        });
        let Event::Created(next) = wait_for(&client, |event| matches!(event, Event::Created(_)))
        else {
            unreachable!("filtered")
        };
        client.send(Command::Send {
            draft: next.id,
            account: ACCOUNT.into(),
            now: 202,
        });
        let Event::Sent {
            sent: next_sent, ..
        } = wait_for(&client, |event| matches!(event, Event::Sent { .. }))
        else {
            unreachable!("filtered")
        };
        assert_ne!(
            next_sent.id, sent.id,
            "refresh must retain the provider's delivery history"
        );
    }

    /// Wait for the first event the worker produces that matches, so a test can drive the
    /// real client instead of a stand-in for it.
    ///
    /// This blocks on the worker's own channel under a deadline rather than polling an
    /// empty drain on a timer: the worker pushes its events and *then* wakes, so a wake
    /// that arrives with nothing queued is the gap this must tolerate, and a test that
    /// waited for a wake could miss one already sitting in the queue.
    fn wait_for(client: &Client, matches: impl Fn(&Event) -> bool) -> Event {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            assert!(
                !remaining.is_zero(),
                "the worker produced no matching event"
            );
            match client.rx.recv_timeout(remaining) {
                Ok(event) if matches(&event) => return event,
                // A different event from the same job: drop it and keep listening.
                Ok(_) => continue,
                Err(e) => panic!("the worker produced no matching event: {e}"),
            }
        }
    }

    /// An unaddressed draft is a real draft: it saves and mirrors to the provider, and only
    /// the send refuses it — with the draft itself left editable.
    #[test]
    fn a_draft_without_a_recipient_saves_but_does_not_send() {
        let (mut worker, mock) = worker(100);
        let mut draft = DraftRecord::new_email(ACCOUNT, 100);
        draft.subject = "Later".into();
        draft.body.set_source("thinking");
        let created = only(worker.handle(Command::Create {
            draft: Box::new(draft),
        }));
        let Event::Created(created) = created else {
            panic!("created")
        };
        assert!(created.to.is_empty());

        let event = only(worker.handle(Command::Sync { draft: created.id }));
        let Event::Synced(synced, _) = event else {
            panic!("an unaddressed draft still belongs on the server: {event:?}");
        };
        assert!(synced.remote_id.is_some(), "it was mirrored");

        let event = only(worker.handle(Command::Send {
            draft: synced.id,
            account: ACCOUNT.into(),
            now: 200,
        }));
        let Event::SendFailed(refused, reason) = event else {
            panic!("a send with no recipient is refused, not attempted: {event:?}");
        };
        assert!(reason.contains("recipient"), "{reason}");
        assert!(!refused.attempted, "nothing was ever requested");
        assert!(
            mock.lock().expect("the mock").log().sent.is_empty(),
            "and nothing went out"
        );
    }

    /// An account that cannot compose any more must not leave a held draft frozen with a
    /// countdown that will not run, or a hold row that fires later by itself.
    #[test]
    fn a_send_to_an_account_without_a_backend_gives_the_held_draft_back() {
        let (mut worker, _mock) = worker(100);
        let draft = stored(&mut worker, "Lunch", 100);
        only(worker.handle(Command::BeginHold {
            draft: draft.id,
            now: 100,
            at: None,
        }));
        assert_eq!(
            worker.stores.drafts.pending_holds().expect("holds").len(),
            1
        );

        let event = only(worker.handle(Command::Send {
            draft: draft.id,
            account: "gone".into(),
            now: 200,
        }));
        let Event::SendFailed(back, reason) = event else {
            panic!("the user gets their draft back, not a capability error: {event:?}");
        };
        assert!(!reason.is_empty());
        assert_eq!(back.state, DraftState::Editing, "editable again");
        assert!(!back.attempted, "nothing was attempted");
        assert!(
            worker
                .stores
                .drafts
                .pending_holds()
                .expect("holds")
                .is_empty(),
            "and the scheduled send is gone, so nothing can go out later by itself"
        );
    }

    /// A send the provider confirmed is a delivery, whatever this Mac managed to write
    /// afterwards. Reporting it as a store failure would take the message out of the outbox
    /// and the mailbox while the recipient has it, and would leave the UI showing a
    /// problem where there is none to fix.
    #[test]
    fn a_confirmed_send_is_still_a_send_when_the_local_record_cannot_be_written() {
        let (mut worker, mock, database) = worker_on_disk("confirmed", 100);
        let draft = stored(&mut worker, "Lunch", 100);
        refuse_state_writes(&database, r#""state":{"Sent":"#);

        let events = worker.handle(Command::Send {
            draft: draft.id,
            account: ACCOUNT.into(),
            now: 200,
        });
        let Event::Sent { sent, message, .. } = &events[0] else {
            panic!("a confirmed delivery is reported as one: {events:?}");
        };
        assert!(
            sent.id.starts_with("sent-"),
            "with the provider's own record of it"
        );
        assert_eq!(
            message.subject, "Lunch",
            "and the message that actually went out"
        );
        let Event::StoreFailed(Some(id), _detail) = &events[1] else {
            panic!("the failed write is reported beside the delivery: {events:?}");
        };
        assert_eq!(
            *id, draft.id,
            "and it names the draft whose record could not be written"
        );
        // The detail is the disk's own error, not a replacement for the delivery above.
        assert_eq!(
            mock.lock().expect("the mock").log().sent.len(),
            1,
            "one delivery"
        );
        // The row still says the attempt is in flight, so a restart reconciles it by
        // Message-ID instead of sending it a second time.
        let stored_row = worker
            .stores
            .drafts
            .get(draft.id)
            .expect("get")
            .expect("present");
        assert_eq!(stored_row.state, DraftState::Sending);
        assert!(stored_row.attempted, "the attempt is still on the record");
    }

    /// The same failure on the write that would have said "this may have been delivered".
    /// The UI has to be left knowing the delivery is in doubt, because that is what stops
    /// it offering to send the draft again.
    #[test]
    fn an_uncertain_send_stays_uncertain_when_the_local_record_cannot_be_written() {
        let (mut worker, mock, database) = worker_on_disk("uncertain", 100);
        let draft = stored(&mut worker, "Lunch", 100);
        lose_send_answers(&worker, ACCOUNT, &mock);
        refuse_state_writes(&database, r#""state":"Uncertain""#);

        let events = worker.handle(Command::Send {
            draft: draft.id,
            account: ACCOUNT.into(),
            now: 200,
        });
        let Event::Uncertain(uncertain, detail) = &events[0] else {
            panic!("a delivery in doubt must not be reported as anything else: {events:?}");
        };
        assert!(!detail.is_empty());
        assert_eq!(
            uncertain.state,
            DraftState::Uncertain,
            "so nothing resends it"
        );
        assert!(
            uncertain.attempted,
            "and it is still reconcilable by its Message-ID"
        );
        let Event::StoreFailed(Some(id), _error) = &events[1] else {
            panic!("the failed write is reported beside the uncertainty: {events:?}");
        };
        assert_eq!(*id, draft.id);
        // Likewise here: the uncertainty above is what the UI acts on, not this error.
        // It really did go out, and the recovery below finds it rather than sending again.
        assert_eq!(mock.lock().expect("the mock").log().sent.len(), 1);
        // The disk takes writes again: what was refused was one write, not the row. With
        // the injection still armed, recovery itself would fail on the very write it is
        // here to make.
        allow_state_writes(&database);
        let Event::Recovered(recovery, _) = only(worker.handle(Command::Recover { now: 10_000 }))
        else {
            panic!("expected a recovery report");
        };
        assert_eq!(
            recovery.uncertain,
            vec![draft.id],
            "a restart reconciles the attempt instead of retrying it"
        );
    }

    /// The attempt has to be on disk before the request leaves, so a store that cannot
    /// record one is precisely why nothing goes out — and the draft must not be left
    /// looking like a send is in flight, with no way to send or edit it again.
    #[test]
    fn nothing_is_sent_when_the_attempt_cannot_be_recorded_first() {
        let (mut worker, mock, database) = worker_on_disk("attempt", 100);
        let draft = stored(&mut worker, "Lunch", 100);
        only(worker.handle(Command::BeginHold {
            draft: draft.id,
            now: 100,
            at: None,
        }));
        refuse_state_writes(&database, r#""state":"Sending""#);

        let events = worker.handle(Command::Send {
            draft: draft.id,
            account: ACCOUNT.into(),
            now: 200,
        });
        let Event::SendFailed(back, reason) = &events[0] else {
            panic!("the user gets their draft back: {events:?}");
        };
        assert!(reason.contains("nothing was sent"), "{reason}");
        assert!(!back.attempted, "nothing was attempted");
        assert_eq!(back.state, DraftState::Editing, "so it is editable again");
        assert!(
            mock.lock().expect("the mock").log().sent.is_empty(),
            "and the request never left this machine"
        );
        assert!(
            worker
                .stores
                .drafts
                .pending_holds()
                .expect("holds")
                .is_empty(),
            "nor is a send left scheduled to go out later by itself"
        );
    }

    /// A provider draft that is genuinely empty — read, and simply without a body — is a
    /// draft, and imports as one.
    #[test]
    fn an_empty_provider_draft_imports_as_an_empty_draft() {
        let (mut worker, mock) = worker(100);
        let mail = OutgoingMail {
            rfc822: b"From: me@x.test\r\nTo: you@x.test\r\nSubject: Empty\r\n\r\n".to_vec(),
            message_id: "empty@x.test".into(),
            in_reply_to: None,
            references: Vec::new(),
            thread: None,
            account: ACCOUNT.to_owned(),
        };
        let id = mock
            .lock()
            .expect("the mock")
            .create_draft(&mail)
            .expect("seed")
            .id;

        let event = only(worker.handle(Command::ImportOne {
            account: ACCOUNT.into(),
            remote: id,
        }));
        let Event::Imported { fresh, failed, .. } = event else {
            panic!("expected an import, got {event:?}");
        };
        assert!(
            failed.is_empty(),
            "an empty draft is not a failed read: {failed:?}"
        );
        let draft = fresh.into_iter().next().expect("one adopted draft");
        assert_eq!(draft.subject, "Empty");
        assert!(draft.body.source.trim().is_empty());
    }

    /// A provider whose draft writes block until the test says so.
    ///
    /// The barrier is two channels: the provider announces that it has entered the call,
    /// and waits for the release. Nothing here polls or sleeps, so the test's ordering is
    /// exactly the ordering it asked for — the provider really is still inside the call
    /// while the local write is made.
    struct BlockingProvider {
        inner: Box<dyn ComposeProvider>,
        entered: Sender<()>,
        release: Receiver<()>,
    }

    impl ComposeProvider for BlockingProvider {
        fn list_drafts(
            &mut self,
            page: Option<&str>,
            max: usize,
        ) -> Result<DraftPage, ComposeError> {
            self.inner.list_drafts(page, max)
        }

        fn get_draft(&mut self, id: &RemoteId) -> Result<RemoteDraft, ComposeError> {
            self.inner.get_draft(id)
        }

        fn draft_raw(&mut self, id: &RemoteId) -> Result<Vec<u8>, ComposeError> {
            self.inner.draft_raw(id)
        }

        fn create_draft(&mut self, mail: &OutgoingMail) -> Result<RemoteDraft, ComposeError> {
            self.enter();
            self.inner.create_draft(mail)
        }

        fn update_draft(
            &mut self,
            draft: &RemoteDraft,
            mail: &OutgoingMail,
        ) -> Result<RemoteDraft, ComposeError> {
            self.enter();
            self.inner.update_draft(draft, mail)
        }

        fn delete_draft(&mut self, id: &RemoteId) -> Result<(), ComposeError> {
            self.inner.delete_draft(id)
        }

        fn send_draft(
            &mut self,
            draft: &RemoteDraft,
            mail: Option<&OutgoingMail>,
        ) -> Result<SendStatus, ComposeError> {
            self.inner.send_draft(draft, mail)
        }

        fn find_sent(&mut self, message_id: &str) -> Result<Option<SentMail>, ComposeError> {
            self.inner.find_sent(message_id)
        }

        fn list_attachments(
            &mut self,
            message: &RemoteId,
        ) -> Result<Vec<RemoteAttachment>, ComposeError> {
            self.inner.list_attachments(message)
        }

        fn attachment_bytes(
            &mut self,
            message: &RemoteId,
            attachment: &RemoteAttachment,
        ) -> Result<AttachmentBytes, ComposeError> {
            self.inner.attachment_bytes(message, attachment)
        }

        fn threading(&mut self, message: &RemoteId) -> Result<Threading, ComposeError> {
            self.inner.threading(message)
        }
    }

    impl BlockingProvider {
        /// Announce that a draft write has begun, and stay inside it until released.
        fn enter(&mut self) {
            let _ = self.entered.send(());
            let _ = self.release.recv();
        }
    }

    /// The composer with both lanes running over real threads, one account whose draft
    /// writes block on a barrier the test controls, and the draft store so the test can
    /// read what is durably written while the provider is still inside its call.
    struct Live {
        client: Client,
        entered: Receiver<()>,
        release: Sender<()>,
        drafts: DraftStore,
    }

    impl Live {
        fn start(now: Timestamp) -> Self {
            let stores = Stores::open_in_memory().expect("stores");
            let drafts = stores.drafts.clone();
            let (entered_tx, entered_rx) = channel();
            let (release_tx, release_rx) = channel();
            let inner: Box<dyn ComposeProvider> = Box::new(MockCompose::new(now));
            let blocked = BlockingProvider {
                inner,
                entered: entered_tx,
                release: release_rx,
            };
            let compose: Arc<Mutex<Box<dyn ComposeProvider>>> =
                Arc::new(Mutex::new(Box::new(blocked)));
            let mut accounts = Accounts::new();
            accounts.insert(
                ACCOUNT.to_owned(),
                ComposeAccount {
                    backend: ComposeBackend::ComposeOnly(compose),
                    email: SENDER.to_owned(),
                },
            );
            Self {
                client: Client::start(stores, accounts),
                entered: entered_rx,
                release: release_tx,
                drafts,
            }
        }

        /// Block until the provider is really inside a draft write. A deadline, never a
        /// sleep: the test fails if the call never begins rather than waiting forever.
        fn provider_entered(&self) {
            self.entered
                .recv_timeout(std::time::Duration::from_secs(30))
                .expect("the provider reached its draft write");
        }
    }

    /// Typing must not wait for the network. With the provider stuck inside a draft write,
    /// the keystroke's own save is still written and readable, and the write that finally
    /// comes back from the provider binds its ids and marks the snapshot it uploaded
    /// without touching the newer body.
    #[test]
    fn a_keystroke_is_durable_while_the_provider_is_still_blocked() {
        let live = Live::start(100);
        let mut draft = DraftRecord::new_email(ACCOUNT, 100);
        draft.to = OTHER.to_owned();
        draft.subject = "Lunch".into();
        draft.body.set_source("hello");
        live.client.send(Command::Create {
            draft: Box::new(draft),
        });
        let Event::Created(created) = wait_for(&live.client, |e| matches!(e, Event::Created(_)))
        else {
            unreachable!("filtered")
        };
        let id = created.id;

        // The mirror goes out first and stops inside the provider.
        live.client.send(Command::Sync { draft: id });
        live.provider_entered();

        // The user keeps typing while that call is out.
        let mut typed = (*created).clone();
        typed.body.set_source("hello, and about lunch");
        live.client.send(Command::Save {
            draft: Box::new(typed),
            expected: created.editor_revision,
        });
        let Event::Saved(saved) = wait_for(&live.client, |e| matches!(e, Event::Saved(_))) else {
            unreachable!("filtered")
        };
        assert_eq!(
            saved.body.source, "hello, and about lunch",
            "the save is answered even though the provider has not come back"
        );
        let durable = live.drafts.get(id).expect("get").expect("present");
        assert_eq!(
            durable.body.source, "hello, and about lunch",
            "and it is already on disk, before the provider is released"
        );

        // Now the provider answers.
        live.release.send(()).expect("release the provider");
        let Event::Synced(synced, _) = wait_for(&live.client, |e| matches!(e, Event::Synced(..)))
        else {
            unreachable!("filtered")
        };
        assert!(
            synced.remote_id.is_some(),
            "the completion binds the provider draft it created"
        );
        assert_eq!(
            synced.body.source, "hello, and about lunch",
            "the newer body survives the late binding"
        );
        let row = live.drafts.get(id).expect("get").expect("present");
        assert_ne!(
            row.synced_content_revision,
            Some(row.content_revision),
            "the snapshot that was uploaded is the older one, so the row is honestly \
             unmirrored rather than claiming the newer text was sent"
        );
    }

    /// A managed file must land on the draft *as it is when the copy finishes*, not on the
    /// snapshot the command started from. Here the copy is still queued behind a blocked
    /// provider call while the user rewrites the body, so the completion writes onto a row
    /// that has already moved on.
    #[test]
    fn a_finished_attachment_keeps_the_body_edited_while_it_was_queued() {
        let live = Live::start(100);
        let mut draft = DraftRecord::new_email(ACCOUNT, 100);
        draft.to = OTHER.to_owned();
        draft.subject = "Lunch".into();
        draft.body.set_source("hello");
        live.client.send(Command::Create {
            draft: Box::new(draft),
        });
        let Event::Created(created) = wait_for(&live.client, |e| matches!(e, Event::Created(_)))
        else {
            unreachable!("filtered")
        };
        let id = created.id;

        // A file is picked while the provider is busy: its copy queues behind the blocked
        // call, so the row moves on before that copy lands.
        let picked = scratch_file("copied-while-typing.txt");
        std::fs::write(&picked, b"the picked bytes").expect("write the picked file");
        live.client.send(Command::Sync { draft: id });
        live.provider_entered();
        live.client.send(Command::Attach {
            draft: id,
            files: vec![picked.clone()],
        });

        let mut typed = (*created).clone();
        typed
            .body
            .set_source("hello, rewritten while the copy was queued");
        live.client.send(Command::Save {
            draft: Box::new(typed),
            expected: created.editor_revision,
        });
        wait_for(&live.client, |e| matches!(e, Event::Saved(_)));

        live.release.send(()).expect("release the provider");
        let Event::Attached {
            draft: attached,
            failed,
        } = wait_for(&live.client, |e| matches!(e, Event::Attached { .. }))
        else {
            unreachable!("filtered")
        };
        assert!(failed.is_empty(), "the file was copied: {failed:?}");
        assert_eq!(
            attached.body.source, "hello, rewritten while the copy was queued",
            "the edit made while the copy was queued is still the draft's body"
        );
        assert_eq!(attached.attachments.len(), 1, "and the file is on it");
        let _ = std::fs::remove_file(&picked);
    }

    /// Put a stored draft on another account the way the surface does when the user picks a
    /// different one: the record's account changes and the provider binding goes with it,
    /// because those ids are the previous account's.
    fn move_to_account(worker: &Rig, draft: &DraftRecord, account: &str) -> DraftRecord {
        let mut moved = draft.clone();
        moved.account = account.to_owned();
        moved.remote_id = None;
        moved.remote_revision = None;
        moved.thread_remote_id = None;
        moved.synced_content_revision = None;
        let expected = moved.revision;
        worker
            .stores
            .drafts
            .save(&mut moved, expected)
            .expect("the account switch is saved")
    }

    /// A mirror is routed by the durable record, never by whatever account the surface held
    /// when it asked: the copy lands on the account the draft actually belongs to, so the
    /// account it left keeps nothing to clean up.
    #[test]
    fn a_mirror_follows_the_account_the_record_now_has() {
        const SECOND: &str = "work";
        let (mut worker, mine) = worker(100);
        let theirs = add_account(&worker, SECOND, "them@x.test", 100);
        let draft = stored(&mut worker, "Lunch", 100);
        let moved = move_to_account(&worker, &draft, SECOND);

        let event = only(worker.handle(Command::Sync { draft: moved.id }));
        let Event::Synced(synced, remote) = event else {
            panic!("the mirror follows the record's account: {event:?}");
        };
        assert_eq!(synced.account, SECOND);
        let bound = synced
            .remote_id
            .clone()
            .expect("it is bound to a provider draft");
        assert!(
            theirs
                .lock()
                .expect("the second account")
                .get_draft(&bound)
                .is_ok(),
            "the copy is on the account the record names"
        );
        assert!(
            mine.lock()
                .expect("the first account")
                .draft_ids()
                .is_empty(),
            "and the account it left is untouched: no orphan copy to clean up"
        );
        assert_eq!(
            remote.id, bound,
            "the reported provider draft is the one bound"
        );
    }

    /// A send request that named the account the draft has left delivers nothing: the message
    /// would go out of the wrong mailbox, from an address the user did not choose.
    #[test]
    fn a_send_that_names_an_account_the_draft_has_left_sends_nothing() {
        const SECOND: &str = "work";
        let (mut worker, mine) = worker(100);
        let _theirs = add_account(&worker, SECOND, "them@x.test", 100);
        let draft = stored(&mut worker, "Lunch", 100);
        let moved = move_to_account(&worker, &draft, SECOND);

        let event = only(worker.handle(Command::Send {
            draft: moved.id,
            account: ACCOUNT.into(),
            now: 200,
        }));
        let Event::SendFailed(back, reason) = event else {
            panic!("a send from the wrong mailbox is refused, not delivered: {event:?}");
        };
        assert!(!reason.is_empty());
        assert!(!back.attempted, "nothing was ever requested");
        assert!(
            mine.lock()
                .expect("the first account")
                .log()
                .sent
                .is_empty(),
            "and nothing went out"
        );
        assert!(
            worker
                .stores
                .drafts
                .pending_holds()
                .expect("holds")
                .is_empty(),
            "nor is a send left scheduled to go out later by itself"
        );
    }

    /// The same guard on a conflict: settling it against the account the draft has left would
    /// read — and on "keep mine" — write another mailbox's draft.
    ///
    /// The binding is deliberately *kept* here, so the only thing that can stop the
    /// resolution is the account it is being carried out against. A switch normally drops the
    /// binding too, and then the resolution would refuse for the wrong reason.
    #[test]
    fn a_conflict_cannot_be_settled_against_an_account_the_draft_has_left() {
        const SECOND: &str = "work";
        let (mut worker, _mine) = worker(100);
        let _theirs = add_account(&worker, SECOND, "them@x.test", 100);
        let draft = stored(&mut worker, "Mine", 100);
        only(worker.handle(Command::Sync { draft: draft.id }));
        let bound = worker
            .stores
            .drafts
            .get(draft.id)
            .expect("get")
            .expect("present");
        assert!(
            bound.remote_id.is_some(),
            "so there is a conflict to settle at all"
        );
        // The account the record moves to while the resolution is still queued on the
        // surface, before any binding of its own could be dropped.
        let mut moved = bound.clone();
        moved.account = SECOND.to_owned();
        let expected = moved.revision;
        let moved = worker
            .stores
            .drafts
            .save(&mut moved, expected)
            .expect("the move is saved");

        for mode in [
            ResolveMode::KeepLocal,
            ResolveMode::TakeRemote,
            ResolveMode::KeepBoth,
        ] {
            let event = only(worker.handle(Command::Resolve {
                draft: moved.id,
                account: ACCOUNT.into(),
                mode,
            }));
            let Event::ResolveFailed(id, detail) = event else {
                panic!(
                    "{mode:?} must refuse rather than settle another mailbox's draft: {event:?}"
                );
            };
            assert_eq!(id, moved.id);
            assert!(!detail.is_empty());
        }
        let after = worker
            .stores
            .drafts
            .get(moved.id)
            .expect("get")
            .expect("present");
        assert_eq!(
            after.subject, bound.subject,
            "the local content is untouched"
        );
        assert_eq!(
            after.remote_id, bound.remote_id,
            "and no provider id from the account it left was taken on"
        );
        assert_eq!(
            worker.stores.drafts.list().expect("list").len(),
            1,
            "a refused KeepBoth leaves no stray second draft behind"
        );
    }

    /// A delivery check must not take an answer from a mailbox the draft was not sent from.
    /// The message really is in the old account's Sent, and reporting it as *this* draft's
    /// delivery would file it under the wrong account entirely.
    #[test]
    fn a_delivery_check_never_confirms_against_the_account_the_draft_has_left() {
        const SECOND: &str = "work";
        let (mut worker, mine) = worker(100);
        let _theirs = add_account(&worker, SECOND, "them@x.test", 100);
        let draft = stored(&mut worker, "Lunch", 100);
        only(worker.handle(Command::Send {
            draft: draft.id,
            account: ACCOUNT.into(),
            now: 150,
        }));
        let sent = worker
            .stores
            .drafts
            .get(draft.id)
            .expect("get")
            .expect("present");

        // A draft that is in doubt, and now on the other account: the message it may have
        // produced really is in the first account's Sent, under the id it carried there.
        let mut in_doubt = sent.clone();
        in_doubt.state = DraftState::Uncertain;
        in_doubt.note_send_attempt();
        let expected = in_doubt.revision;
        worker
            .stores
            .drafts
            .save(&mut in_doubt, expected)
            .expect("save");
        let moved = move_to_account(&worker, &in_doubt, SECOND);

        let event = only(worker.handle(Command::Reconcile {
            draft: moved.id,
            account: ACCOUNT.into(),
        }));
        let Event::Reconciled {
            draft: checked,
            detail,
        } = event
        else {
            panic!("nothing is confirmed from another mailbox: {event:?}");
        };
        assert!(!detail.is_empty());
        assert_eq!(
            checked.state,
            DraftState::Uncertain,
            "the delivery is still in doubt, so it is still never sent again"
        );
        assert_eq!(
            mine.lock().expect("mock").log().sent.len(),
            1,
            "and nothing was resent"
        );
    }

    /// A hold counts down to a send request that has not been made. For a draft whose send
    /// was already requested — or that may already have gone out — there is nothing to count
    /// towards, so no countdown starts and, crucially, the state that stops it being sent
    /// twice survives.
    #[test]
    fn a_delivery_in_doubt_cannot_be_put_back_on_a_send_hold() {
        let (mut worker, mock) = worker(100);
        let draft = stored(&mut worker, "Lunch", 100);
        lose_send_answers(&worker, ACCOUNT, &mock);
        only(worker.handle(Command::Send {
            draft: draft.id,
            account: ACCOUNT.into(),
            now: 200,
        }));

        let event = only(worker.handle(Command::BeginHold {
            draft: draft.id,
            now: 220,
            at: None,
        }));
        assert!(
            !matches!(event, Event::Held(..)),
            "a draft that may already have been delivered does not wait out a countdown: {event:?}"
        );
        assert!(
            worker
                .stores
                .drafts
                .pending_holds()
                .expect("holds")
                .is_empty(),
            "so there is no countdown that could fire a second send later by itself"
        );
        let after = worker
            .stores
            .drafts
            .get(draft.id)
            .expect("get")
            .expect("present");
        assert_eq!(
            after.state,
            DraftState::Uncertain,
            "the draft keeps the state it was in"
        );
        assert!(
            after.attempted,
            "and the attempt a restart reconciles is still on the record"
        );
    }

    /// Undo takes a held draft back out of its hold. A draft that is not held — because its
    /// send already went — must not be made editable by it: that would drop the attempt and
    /// put the same message back in front of the user to send a second time.
    #[test]
    fn undoing_a_hold_does_not_make_an_uncertain_draft_editable_again() {
        let (mut worker, mock) = worker(100);
        let draft = stored(&mut worker, "Lunch", 100);
        lose_send_answers(&worker, ACCOUNT, &mock);
        only(worker.handle(Command::Send {
            draft: draft.id,
            account: ACCOUNT.into(),
            now: 200,
        }));

        let event = only(worker.handle(Command::CancelHold(draft.id)));
        let Event::HoldCancelled(after) = event else {
            panic!("the hold is gone: {event:?}");
        };
        assert_eq!(
            after.state,
            DraftState::Uncertain,
            "a delivery in doubt stays in doubt: nothing here makes it sendable again"
        );
        assert!(after.attempted);
        assert_eq!(
            worker
                .stores
                .drafts
                .get(draft.id)
                .expect("get")
                .expect("present")
                .state,
            DraftState::Uncertain,
            "and it is what a restart finds"
        );
    }

    /// An imported file keeps the media type the provider stated for it, even where the file
    /// name would say otherwise: the bytes came out of that MIME part, so guessing from the
    /// name would relabel exactly the files whose names lie.
    #[test]
    fn an_imported_file_keeps_the_media_type_the_provider_stated() {
        let (mut worker, mock) = worker(100);
        // A `.txt` name on a part the provider says is JSON: the type is the authority.
        let raw = concat!(
            "From: me@x.test\r\n",
            "To: you@x.test\r\n",
            "Subject: Report\r\n",
            "Message-ID: <typed@x.test>\r\n",
            "MIME-Version: 1.0\r\n",
            "Content-Type: multipart/mixed; boundary=\"b\"\r\n",
            "\r\n",
            "--b\r\n",
            "Content-Type: text/plain\r\n",
            "\r\n",
            "see attached\r\n",
            "--b\r\n",
            "Content-Type: application/json; name=\"report.txt\"\r\n",
            "Content-Disposition: attachment; filename=\"report.txt\"\r\n",
            "\r\n",
            "{\"ok\":true}\r\n",
            "--b--\r\n",
        );
        seed_raw(&mock, raw, "typed@x.test");

        let event = only(worker.handle(Command::Import {
            account: ACCOUNT.into(),
            limit: 10,
        }));
        let Event::Imported { fresh, failed, .. } = event else {
            panic!("expected an import report, got {event:?}");
        };
        assert!(failed.is_empty(), "{failed:?}");
        let draft = fresh.into_iter().next().expect("one adopted draft");
        let file = draft
            .attachments
            .iter()
            .find(|a| a.file_name == "report.txt")
            .expect("the file is carried over");
        assert_eq!(
            file.content_type, "application/json",
            "the type the part stated, not the one the file name guesses"
        );
    }

    /// A backend whose `threading` answer is the test's own, so the chain an import falls
    /// back to is exactly what this test states rather than whatever the message happened to
    /// carry.
    struct KnownThread(Box<dyn ComposeProvider>, Threading);

    impl ComposeProvider for KnownThread {
        fn list_drafts(
            &mut self,
            page: Option<&str>,
            max: usize,
        ) -> Result<DraftPage, ComposeError> {
            self.0.list_drafts(page, max)
        }
        fn get_draft(&mut self, id: &RemoteId) -> Result<RemoteDraft, ComposeError> {
            self.0.get_draft(id)
        }
        fn draft_raw(&mut self, id: &RemoteId) -> Result<Vec<u8>, ComposeError> {
            self.0.draft_raw(id)
        }
        fn create_draft(&mut self, mail: &OutgoingMail) -> Result<RemoteDraft, ComposeError> {
            self.0.create_draft(mail)
        }
        fn update_draft(
            &mut self,
            draft: &RemoteDraft,
            mail: &OutgoingMail,
        ) -> Result<RemoteDraft, ComposeError> {
            self.0.update_draft(draft, mail)
        }
        fn delete_draft(&mut self, id: &RemoteId) -> Result<(), ComposeError> {
            self.0.delete_draft(id)
        }
        fn send_draft(
            &mut self,
            draft: &RemoteDraft,
            mail: Option<&OutgoingMail>,
        ) -> Result<SendStatus, ComposeError> {
            self.0.send_draft(draft, mail)
        }
        fn find_sent(&mut self, message_id: &str) -> Result<Option<SentMail>, ComposeError> {
            self.0.find_sent(message_id)
        }
        fn list_attachments(
            &mut self,
            message: &RemoteId,
        ) -> Result<Vec<RemoteAttachment>, ComposeError> {
            self.0.list_attachments(message)
        }
        fn attachment_bytes(
            &mut self,
            message: &RemoteId,
            attachment: &RemoteAttachment,
        ) -> Result<AttachmentBytes, ComposeError> {
            self.0.attachment_bytes(message, attachment)
        }
        fn threading(&mut self, _message: &RemoteId) -> Result<Threading, ComposeError> {
            Ok(self.1.clone())
        }
    }

    /// Swap `account`'s backend for one whose threading answer is `threading`.
    fn thread_known(
        worker: &Rig,
        account: &str,
        mock: &Arc<Mutex<MockCompose>>,
        threading: Threading,
    ) {
        let compose: Arc<Mutex<Box<dyn ComposeProvider>>> = Arc::new(Mutex::new(Box::new(
            KnownThread(Box::new(SharedMock(Arc::clone(mock), false)), threading),
        )));
        register_backend(worker, account, compose);
    }

    /// A draft that answers a conversation carries the chain it belongs to, even when the
    /// provider's stored bytes state only their own `Message-ID`. A reply built on the
    /// imported draft has to answer the thread it came from, not start a fresh one.
    #[test]
    fn an_import_whose_bytes_carry_no_chain_still_answers_its_thread() {
        let (mut worker, mock) = worker(100);
        let remote = seed_raw(
            &mock,
            concat!(
                "From: me@x.test\r\n",
                "To: you@x.test\r\n",
                "Subject: Re: Lunch\r\n",
                "Message-ID: <reply@x.test>\r\n",
                "MIME-Version: 1.0\r\n",
                "Content-Type: text/plain\r\n",
                "\r\n",
                "sounds good\r\n"
            ),
            "reply@x.test",
        );
        thread_known(
            &worker,
            ACCOUNT,
            &mock,
            Threading {
                message_id: "reply@x.test".into(),
                references: vec!["root@x.test".into()],
                subject: "Re: Lunch".into(),
            },
        );

        let event = only(worker.handle(Command::ImportOne {
            account: ACCOUNT.into(),
            remote,
        }));
        let Event::Imported { fresh, failed, .. } = event else {
            panic!("expected an import report, got {event:?}");
        };
        assert!(failed.is_empty(), "{failed:?}");
        let draft = fresh.into_iter().next().expect("one adopted draft");
        assert_eq!(
            draft.message_id, "reply@x.test",
            "its own id is kept, so a send of those bytes stays reconcilable"
        );
        assert!(
            draft.references.contains(&"root@x.test".to_owned()),
            "and the conversation it answers is carried with it: {:?}",
            draft.references
        );
    }

    /// Put a provider draft holding exactly `raw`, and hand back its remote id.
    fn seed_raw(mock: &Arc<Mutex<MockCompose>>, raw: &str, message_id: &str) -> RemoteId {
        let mail = OutgoingMail {
            rfc822: raw.as_bytes().to_vec(),
            message_id: message_id.to_owned(),
            in_reply_to: None,
            references: Vec::new(),
            thread: None,
            account: ACCOUNT.to_owned(),
        };
        mock.lock()
            .expect("the mock")
            .create_draft(&mail)
            .expect("seed")
            .id
    }
}
