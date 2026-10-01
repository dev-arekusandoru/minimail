//! Mapping between local triage state and remote mail operations.
//!
//! Nothing here runs a clock or touches UI state. A sync round is planned on
//! the main thread ([`plan_round`]), runs on a background thread ([`run_round`])
//! and is merged back on the main thread ([`apply_round`]). Rounds are small:
//! pending moves and reads, one server change check, one page of load-more and
//! one page of backfill, so an import never blocks the UI or the provider.

pub mod cache;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::clock::{DAY, Timestamp};
use crate::model::{
    Folder, FolderId, Location, Mailbox, Message, MessageId, TriageState, format_rfc3339,
};
use crate::provider::{
    Body, MailProvider, ProviderError, RemoteFlags, RemoteFolder, RemoteId, RemoteMessage,
    RemoteState, Scope, Window,
};

use cache::{Backfill, Cache};

/// How far back a first sync reaches.
pub const WINDOW_DAYS: i64 = 30;
/// Ids per listing page (Gmail counts a list as 5 quota units, a batch get as 5).
const PAGE: usize = 50;

/// A provider shared between the sync loop and body downloads; each step locks
/// it separately so a body fetch can interleave with a round.
pub type SharedProvider = Arc<Mutex<Box<dyn MailProvider>>>;

/// The remote counterpart of a local state, or `None` when the target folder
/// has no remote counterpart yet.
///
/// Snoozed messages archive remotely: there is no snooze operation to push,
/// so `tick` is what wakes them locally.
pub fn to_remote(
    state: TriageState,
    folder_remote: impl Fn(FolderId) -> Option<RemoteId>,
) -> Option<RemoteState> {
    match state {
        TriageState::Inbox => Some(RemoteState::Inbox),
        TriageState::Archived | TriageState::Snoozed => Some(RemoteState::Archived),
        TriageState::Deleted => Some(RemoteState::Trash),
        TriageState::Filed(folder) => folder_remote(folder).map(RemoteState::Folder),
    }
}

// ------------------------------------------------------------------ moves

/// One triage change waiting to be pushed to a provider.
#[derive(Clone, Debug, PartialEq)]
pub struct Move {
    pub id: MessageId,
    pub remote_id: RemoteId,
    pub from: RemoteState,
    pub to: RemoteState,
    /// Set when `to` is an as-yet uncreated `Folder`: the local folder and the
    /// path to create for it.
    pub create_folder: Option<(FolderId, String)>,
}

impl Move {
    fn to(&self) -> &RemoteState {
        &self.to
    }
}

/// Messages of `account` whose local state no longer matches the remote state
/// the cache last confirmed. Messages with no cache row (locally materialised
/// replies, for instance) are skipped, since the provider never saw them.
pub fn pending_moves(mb: &Mailbox, cache: &Cache, account: &str) -> Vec<Move> {
    let mut moves = Vec::new();
    for message in mb.messages().iter().filter(|m| m.account == account) {
        let Some((_, remote_id, cached)) = cache.remote_state(message.id).ok().flatten() else {
            continue;
        };
        // A folder the provider has never seen still needs a move, so build
        // the target by hand; `to_remote` alone reports nothing to do.
        let (to, create_folder) = match message.state {
            TriageState::Filed(folder) => {
                match cache.folder_remote_id(folder).ok().flatten() {
                    Some(rid) => (RemoteState::Folder(rid), None),
                    None => (
                        RemoteState::Folder(String::new()),
                        Some((folder, folder_path(mb, folder))),
                    ),
                }
            }
            _ => match to_remote(message.state, |f| {
                cache.folder_remote_id(f).ok().flatten()
            }) {
                Some(to) => (to, None),
                None => continue,
            },
        };
        if to == cached {
            continue;
        }
        moves.push(Move {
            id: message.id,
            remote_id,
            from: cached,
            to,
            create_folder,
        });
    }
    moves
}

/// Parent chain of `folder`, root first, joined by '/'.
fn folder_path(mb: &Mailbox, folder: FolderId) -> String {
    let mut names = Vec::new();
    let mut current = Some(folder);
    while let Some(id) = current {
        let Some(f) = mb.folder(id) else { break };
        names.push(f.name.clone());
        current = f.parent;
    }
    names.reverse();
    names.join("/")
}

/// Push `moves`, creating any needed labels once per folder per batch.
///
/// Stops at the first fatal failure; moves after it are dropped from the
/// result so they stay pending for the next attempt. The returned `Move`s carry
/// the label id substituted into `to`.
pub fn run_moves(
    p: &mut dyn MailProvider,
    moves: Vec<Move>,
) -> Vec<(Move, Result<Option<RemoteFolder>, ProviderError>)> {
    let mut created: Vec<(FolderId, RemoteFolder)> = Vec::new();
    let mut results = Vec::with_capacity(moves.len());
    for mut mv in moves {
        let mut folder = None;
        if let Some((id, path)) = &mv.create_folder {
            let made = match created.iter().find(|(f, _)| f == id) {
                Some((_, rf)) => Ok(rf.clone()),
                None => match p.create_folder(path) {
                    Ok(rf) => {
                        created.push((*id, rf.clone()));
                        Ok(rf)
                    }
                    Err(e) => Err(e),
                },
            };
            match made {
                Ok(rf) => {
                    if let RemoteState::Folder(rid) = &mut mv.to {
                        *rid = rf.id.clone();
                    }
                    folder = Some(rf);
                }
                Err(e) => {
                    let stop = is_fatal(&e);
                    results.push((mv, Err(e)));
                    if stop {
                        break;
                    }
                    continue;
                }
            }
        }
        let from = mv.from.clone();
        let to = mv.to.clone();
        match p.move_message(&mv.remote_id, &from, &to) {
            Ok(()) => results.push((mv, Ok(folder))),
            Err(e) => {
                let stop = is_fatal(&e);
                results.push((mv, Err(e)));
                if stop {
                    break;
                }
            }
        }
    }
    results
}

/// Record the moves the provider confirmed. Returns the first error text.
pub fn apply_moves(
    mb: &Mailbox,
    cache: &Cache,
    results: &[(Move, Result<Option<RemoteFolder>, ProviderError>)],
) -> Option<String> {
    let mut first = None;
    for (mv, outcome) in results {
        let created = match outcome {
            Ok(created) => created.as_ref(),
            Err(e) => {
                first.get_or_insert_with(|| e.to_string());
                continue;
            }
        };
        // A folder created during the push now maps to its remote label, so
        // later moves into it skip creation.
        let folder = match (&mv.create_folder, created) {
            (Some((local, _)), Some(remote)) => mb.folder(*local).map(|f| (f, remote)),
            _ => None,
        };
        if let Some((folder, remote)) = folder {
            let stored = cache.upsert_folder(folder, Some(&remote.id));
            if let Err(e) = stored {
                first.get_or_insert_with(|| e.to_string());
                continue;
            }
        }
        if let Err(e) = cache.set_remote_state(mv.id, mv.to()) {
            first.get_or_insert_with(|| e.to_string());
        }
    }
    first
}

// ------------------------------------------------------------------ reads

/// A read/unread change waiting to be pushed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadChange {
    pub id: MessageId,
    pub remote_id: RemoteId,
    pub read: bool,
}

/// Messages of `account` whose read state differs from what the server last confirmed.
/// Rows with an unknown server state are skipped until a check fills it in.
pub fn pending_reads(mb: &Mailbox, cache: &Cache, account: &str) -> Vec<ReadChange> {
    mb.messages()
        .iter()
        .filter(|m| m.account == account)
        .filter_map(|m| {
            let confirmed = cache.remote_read(m.id).ok().flatten()?;
            if confirmed == m.read {
                return None;
            }
            let (_, remote_id, _) = cache.remote_state(m.id).ok().flatten()?;
            Some(ReadChange { id: m.id, remote_id, read: m.read })
        })
        .collect()
}

/// Push read changes, stopping at the first fatal failure.
pub fn run_reads(
    p: &mut dyn MailProvider,
    changes: Vec<ReadChange>,
) -> Vec<(ReadChange, Result<(), ProviderError>)> {
    let mut results = Vec::with_capacity(changes.len());
    for change in changes {
        let result = p.set_read(&change.remote_id, change.read);
        let stop = result.as_ref().is_err_and(is_fatal);
        results.push((change, result));
        if stop {
            break;
        }
    }
    results
}

/// Record confirmed read changes; returns the first error, if any.
pub fn apply_reads(cache: &Cache, results: &[(ReadChange, Result<(), ProviderError>)]) -> Option<String> {
    let mut first = None;
    for (change, result) in results {
        let outcome = match result {
            Ok(()) => cache.set_remote_read(change.id, change.read).map_err(err),
            Err(e) => Err(e.to_string()),
        };
        if let Err(e) = outcome {
            first.get_or_insert(e);
        }
    }
    first
}

/// Errors that make the rest of a round pointless.
fn is_fatal(e: &ProviderError) -> bool {
    matches!(e, ProviderError::Auth(_) | ProviderError::Network(_) | ProviderError::RateLimited)
}

fn err(e: rusqlite::Error) -> String {
    e.to_string()
}

// ------------------------------------------------------------------ rounds

/// All provider work for one account in one round, planned on the main thread.
pub struct Round {
    account: String,
    moves: Vec<Move>,
    reads: Vec<ReadChange>,
    /// Run the server change check this round.
    check: bool,
    /// Re-list the 30-day window instead of trusting the cursor.
    reconcile: bool,
    cursor: Option<String>,
    since: Timestamp,
    known: HashSet<RemoteId>,
    /// One listing page per requested scope (load-more).
    older: Vec<PageJob>,
    /// One page of the first scope still to backfill, if the cursor is known.
    backfill: Option<PageJob>,
    /// More scopes wait behind `backfill`.
    more_backfill: bool,
}

/// One listing page to fetch, with the progress it advances.
struct PageJob {
    scope: Scope,
    window: Window,
    page: Option<String>,
    state: Backfill,
    /// Whether this page advances the `Before(since)` (load-more) pass.
    older: bool,
}

impl Round {
    /// Nothing to do: no push, no check, no page.
    pub fn is_empty(&self) -> bool {
        self.moves.is_empty()
            && self.reads.is_empty()
            && !self.check
            && self.older.is_empty()
            && self.backfill.is_none()
    }

    pub fn account(&self) -> &str {
        &self.account
    }
}

/// The scopes a first sync walks, in the order it walks them: the inbox
/// matters most, then what the user filed away, then the labels.
fn backfill_scopes(cache: &Cache, account: &str) -> Vec<Scope> {
    let mut scopes = vec![Scope::Inbox, Scope::Archive, Scope::Trash];
    let folders = cache.folder_list(account).unwrap_or_default();
    let path_of = |id: FolderId| -> String {
        let mut names = Vec::new();
        let mut current = folders.iter().find(|f| f.0 == id).and_then(|f| f.2);
        while let Some(fid) = current {
            let Some((_, name, parent, _)) = folders.iter().find(|f| f.0 == fid) else {
                break;
            };
            names.push(name.clone());
            current = *parent;
        }
        names.reverse();
        names.join("/")
    };
    let mut labelled: Vec<(String, Scope)> = folders
        .iter()
        .filter_map(|(id, _, _, remote)| Some((path_of(*id), Scope::Folder(remote.clone()?))))
        .collect();
    labelled.sort_by(|a, b| a.0.cmp(&b.0));
    scopes.extend(labelled.into_iter().map(|(_, scope)| scope));
    scopes
}

/// Plan one round of provider work for `account`.
///
/// `check` runs the server change check (startup, every 60 s, Fetch mail);
/// `older` are the scopes the UI wants extended past the cached window.
pub fn plan_round(
    mb: &Mailbox,
    cache: &Cache,
    account: &str,

    now: Timestamp,
    check: bool,
    older: &[Scope],
) -> Round {
    let moves = pending_moves(mb, cache, account);
    let reads = pending_reads(mb, cache, account);
    let cursor = cache.cursor(account).ok().flatten();
    let since = cache.window_since(account, now).unwrap_or(now - WINDOW_DAYS * DAY);
    // Rows cached before read sync existed have no server read state; only a
    // fresh window listing can fill it in.
    let reconcile = cache.has_unknown_read(account).unwrap_or(false);
    let known = cache.remote_ids(account).unwrap_or_default();

    let mut older_jobs = Vec::new();
    for scope in older {
        let mut state = cache.backfill(account, scope).unwrap_or_else(|_| Backfill::new(since));
        if state.since == 0 {
            state.since = since;
        }
        let is_older = state.recent_done;
        let (window, page) = if is_older {
            (Window::Before(state.since), state.older_page.clone())
        } else {
            (Window::Since(state.since), state.recent_page.clone())
        };
        older_jobs.push(PageJob { scope: scope.clone(), window, page, state, older: is_older });
    }

    // The backfill runs one page per round, in scope order, and only once the
    // account has a cursor to check changes against.
    let backfill = cursor.as_ref().and_then(|_| {
        backfill_scopes(cache, account).into_iter().find_map(|scope| {
            if older_jobs.iter().any(|job| job.scope == scope) {
                return None;
            }
            let mut state = cache.backfill(account, &scope).unwrap_or_else(|_| Backfill::new(since));
            if state.since == 0 {
                state.since = since;
            }
            if state.recent_done {
                return None;
            }
            let page = state.recent_page.clone();
            Some(PageJob { scope, window: Window::Since(state.since), page, state, older: false })
        })
    });
    let more_backfill = cursor.is_some()
        && backfill_scopes(cache, account).iter().any(|scope| {
            !cache.backfill(account, scope).map(|s| s.recent_done).unwrap_or(false)
        });

    Round {
        account: account.to_owned(),
        moves,
        reads,
        check: check || reconcile,
        reconcile,
        cursor,
        since,
        known,
        older: older_jobs,
        backfill,
        more_backfill,
    }
}

/// Everything one round learned off the main thread.
pub struct RoundResult {
    account: String,
    moves: Vec<(Move, Result<Option<RemoteFolder>, ProviderError>)>,
    reads: Vec<(ReadChange, Result<(), ProviderError>)>,
    folders: Option<Vec<RemoteFolder>>,
    since: Timestamp,
    cursor: Option<String>,
    changes: Option<crate::provider::Changes>,
    snapshot: Option<Vec<(RemoteId, RemoteFlags)>>,
    reset_recent: bool,
    fetched: Vec<RemoteMessage>,
    progress: Vec<(Scope, Backfill)>,
    more: bool,
    throttled: bool,
    checked: bool,
    error: Option<String>,
}

/// Merge a round into the mailbox and the cache.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RoundSummary {
    /// Messages added to the mailbox this round.
    pub new_messages: usize,
    /// Backfill or requested older pages still remain: run the next round right away.
    pub more: bool,
    /// The provider is still rate-limited after its own retries: pause ~60 s.
    pub throttled: bool,
    /// First error, user-facing (toast once per distinct text).
    pub error: Option<String>,
    /// The check ran and completed this round.
    pub checked: bool,
}

fn lock(provider: &SharedProvider) -> Result<MutexGuard<'_, Box<dyn MailProvider>>, ProviderError> {
    provider
        .lock()
        .map_err(|_| ProviderError::Network("sync provider lock poisoned".into()))
}

/// Run one round off the main thread.
///
/// Each provider call group takes the lock on its own, so a body download for
/// the opened message interleaves with the round instead of waiting for it.
/// A rate-limited, unauthenticated or unreachable provider ends the round.
pub fn run_round(provider: &SharedProvider, round: Round) -> RoundResult {
    let Round {
        account,
        moves,
        reads,
        check: run_check,
        reconcile: reconcile_first,
        cursor,
        since,
        known,
        older,
        backfill,
        more_backfill,
    } = round;
    let mut result = RoundResult {
        account,
        since,
        moves: Vec::new(),
        reads: Vec::new(),
        folders: None,
        cursor: None,
        changes: None,
        snapshot: None,
        reset_recent: false,
        fetched: Vec::new(),
        progress: Vec::new(),
        more: false,
        throttled: false,
        checked: false,
        error: None,
    };

    // Push: moves first, so a message is filed before it is marked read.
    if !moves.is_empty() || !reads.is_empty() {
        let stop = match lock(provider) {
            Ok(mut p) => {
                let moves = run_moves(p.as_mut(), moves);
                let stop = first_fatal(&mut result, &moves);
                result.moves = moves;
                if stop {
                    true
                } else {
                    let reads = run_reads(p.as_mut(), reads);
                    let stop = first_fatal(&mut result, &reads);
                    result.reads = reads;
                    stop
                }
            }
            Err(e) => stop_error(&mut result, &e),
        };
        if stop {
            return result;
        }
    }

    // Check: the folder list first, then the cursor (or the changes since it).
    if run_check {
        let mut fetched: Vec<RemoteId> = Vec::new();
        let mut stop = false;
        result.folders = match lock(provider) {
            Ok(mut p) => match p.folders() {
                Ok(folders) => Some(folders),
                Err(e) => {
                    stop = stop_error(&mut result, &e);
                    None
                }
            },
            Err(e) => {
                stop = stop_error(&mut result, &e);
                None
            }
        };
        if !stop {
            stop = if reconcile_first {
                reconcile(provider, &mut result)
            } else {
                check(provider, &known, cursor.as_deref(), &mut result, &mut fetched)
            };
        }
        if !stop && !fetched.is_empty() {
            let ids = uncached(&known, fetched);
            if let Ok(mut p) = lock(provider) {
                match p.fetch_headers(&ids) {
                    Ok(messages) => result.fetched = messages,
                    Err(e) => {
                        stop_error(&mut result, &e);
                    }
                }
            }
        }
        if stop {
            return result;
        }
    }

    // Load-more, then one page of backfill.
    let mut paged: HashSet<Scope> = HashSet::new();
    for job in older {
        if run_page(provider, &known, job, &mut result, &mut paged) {
            return result;
        }
    }
    if let Some(job) = backfill
        && run_page(provider, &known, job, &mut result, &mut paged)
    {
        return result;
    }
    result.more |= more_backfill;
    result
}

/// Record a provider error: fatal ones end the round, rate limits are not a
/// user-facing failure.
fn stop_error(result: &mut RoundResult, e: &ProviderError) -> bool {
    if matches!(e, ProviderError::RateLimited) {
        result.throttled = true;
    } else {
        result.error.get_or_insert_with(|| e.to_string());
    }
    is_fatal(e)
}

/// Record the first fatal error among a batch's results; non-fatal ones are
/// reported by [`stop_error`] when they are collected.
fn first_fatal<T, R>(
    result: &mut RoundResult,
    results: &[(T, Result<R, ProviderError>)],
) -> bool {
    let mut stop = false;
    for (_, r) in results {
        if let Err(e) = r
            && stop_error(result, e)
        {
            stop = true;
        }
    }
    stop
}


/// The cursor is stored first when there is none, so nothing that lands during
/// the backfill is missed; otherwise the change history is read.
fn check(
    provider: &SharedProvider,
    known: &HashSet<RemoteId>,
    cursor: Option<&str>,
    result: &mut RoundResult,
    fetched: &mut Vec<RemoteId>,
) -> bool {
    let Ok(mut p) = lock(provider) else {
        return true;
    };
    let Some(cursor) = cursor else {
        // The first round stores a cursor before any backfill starts, so
        // nothing that lands during the import is missed.
        return match p.cursor() {
            Ok(cursor) => {
                result.cursor = Some(cursor);
                result.checked = true;
                false
            }
            Err(e) => stop_error(result, &e),
        };
    };
    match p.changes(cursor) {
        Ok(changes) => {
            result.cursor = Some(changes.cursor.clone());
            let mut to_fetch: Vec<RemoteId> = changes
                .updated
                .iter()
                .map(|(id, _)| id)
                .filter(|id| !known.contains(*id))
                .cloned()
                .collect();
            to_fetch.extend(changes.added.iter().cloned());
            fetched.extend(to_fetch);
            result.changes = Some(changes);
            result.checked = true;
            false
        }
        Err(ProviderError::CursorExpired) => {
            drop(p);
            reconcile(provider, result)
        }
        Err(e) => stop_error(result, &e),
    }
}

/// An expired cursor (or a cache from before read sync) is answered by listing
/// the whole window: ids only, no per-message gets, then a fresh cursor.
fn reconcile(provider: &SharedProvider, result: &mut RoundResult) -> bool {
    let mut p = match lock(provider) {
        Ok(p) => p,
        Err(e) => return stop_error(result, &e),
    };
    let cursor = match p.cursor() {
        Ok(cursor) => cursor,
        Err(e) => return stop_error(result, &e),
    };
    match p.snapshot(result.since) {
        Ok(flags) => {
            result.cursor = Some(cursor);
            result.snapshot = Some(flags);
            result.reset_recent = true;
            result.checked = true;
            false
        }
        Err(e) => stop_error(result, &e),
    }
}

/// One listing page, its uncached headers, and the progress it advances.
/// Returns true when the round must stop.
fn run_page(
    provider: &SharedProvider,
    known: &HashSet<RemoteId>,
    job: PageJob,
    result: &mut RoundResult,
    paged: &mut HashSet<Scope>,
) -> bool {
    if !paged.insert(job.scope.clone()) {
        return false;
    }
    let page = {
        let Ok(mut p) = lock(provider) else {
            return true;
        };
        match p.list(&job.scope, job.window, job.page.as_deref(), PAGE) {
            Ok(page) => page,
            Err(e) => return stop_error(result, &e),
        }
    };
    let ids = uncached(known, page.ids.clone());
    if !ids.is_empty() {
        let mut p = match lock(provider) {
            Ok(p) => p,
            Err(e) => return stop_error(result, &e),
        };
        match p.fetch_headers(&ids) {
            Ok(messages) => result.fetched.extend(messages),
            Err(e) => return stop_error(result, &e),
        }
    }
    let mut state = job.state;
    let next = page.next.clone();
    if job.older {
        state.older_page = next.clone();
        state.older_done = next.is_none();
    } else {
        state.recent_page = next.clone();
        state.recent_done = next.is_none();
    }
    if next.is_some() {
        result.more = true;
    }
    result.progress.push((job.scope, state));
    false
}

fn uncached(known: &HashSet<RemoteId>, ids: Vec<RemoteId>) -> Vec<RemoteId> {
    ids.into_iter().filter(|id| !known.contains(id)).collect()
}

/// Merge a round into the mailbox and the cache, and report what changed.
///
/// Every remote folder gets a local folder, even when no message is in it.
/// A local change the provider has not confirmed wins over what the server
/// says; a snoozed message stays snoozed while it is archived remotely; a
/// downloaded body is never replaced by a snippet.
pub fn apply_round(mb: &mut Mailbox, cache: &Cache, result: RoundResult) -> RoundSummary {
    let mut summary = RoundSummary {
        more: result.more,
        throttled: result.throttled,
        error: result.error,
        checked: result.checked,
        new_messages: 0,
    };
    let mut error = |e: String| {
        if summary.error.is_none() {
            summary.error = Some(e);
        }
    };
    if let Some(e) = apply_moves(mb, cache, &result.moves) {
        error(e);
    }
    if let Some(e) = apply_reads(cache, &result.reads) {
        error(e);
    }

    // Without a check this round there is no fresh folder list, so the mapping
    // the cache already holds stands in for it.
    let folders = result.folders.clone().unwrap_or_else(|| {
        cache
            .folder_list(&result.account)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(id, _, _, remote)| {
                Some(RemoteFolder { id: remote?, path: folder_path(mb, id) })
            })
            .collect()
    });
    let by_path: HashMap<&str, &str> =
        folders.iter().map(|f| (f.path.as_str(), f.id.as_str())).collect();
    let by_id: HashMap<&str, &str> = folders.iter().map(|f| (f.id.as_str(), f.path.as_str())).collect();
    for folder in &folders {
        if let Err(e) = ensure_folder(mb, cache, &result.account, &by_path, &folder.path) {
            error(e);
        }
    }

    if let Some(changes) = &result.changes {
        if let Err(e) = apply_flags(mb, cache, &result.account, &by_path, &by_id, &changes.updated) {
            error(e);
        }
        if let Err(e) = remove_ids(mb, cache, &result.account, &changes.removed) {
            error(e);
        }
    }
    if let Some(snapshot) = &result.snapshot {
        if let Err(e) = apply_flags(mb, cache, &result.account, &by_path, &by_id, snapshot) {
            error(e);
        }
        if let Err(e) = drop_vanished(mb, cache, &result.account, result.since, snapshot) {
            error(e);
        }
    }
    if result.reset_recent
        && let Err(e) = cache.reset_recent(&result.account)
    {
        error(err(e));
    }

    for remote in &result.fetched {
        match apply_message(mb, cache, &result.account, &by_path, &by_id, remote) {
            Ok(new) => summary.new_messages += usize::from(new),
            Err(e) => error(e),
        }
    }

    if let Some(cursor) = &result.cursor
        && let Err(e) = cache.set_cursor(&result.account, cursor)
    {
        error(err(e));
    }
    for (scope, state) in &result.progress {
        if let Err(e) = cache.save_backfill(&result.account, scope, state) {
            error(err(e));
        }
    }
    summary
}

/// The local state behind a remote one.
fn from_remote(
    mb: &mut Mailbox,
    cache: &Cache,
    account: &str,
    by_path: &HashMap<&str, &str>,
    by_id: &HashMap<&str, &str>,
    state: &RemoteState,
) -> Result<TriageState, String> {
    match state {
        RemoteState::Inbox => Ok(TriageState::Inbox),
        RemoteState::Archived => Ok(TriageState::Archived),
        RemoteState::Trash => Ok(TriageState::Deleted),
        RemoteState::Folder(rid) => match by_id.get(rid.as_str()).copied() {
            Some(path) => Ok(TriageState::Filed(ensure_folder(
                mb, cache, account, by_path, path,
            )?)),
            None => Ok(TriageState::Archived),
        },
    }
}

/// The local state of a cached message after the server's flags, honouring
/// local changes that have not been pushed yet.
fn merged_state(
    existing: &Message,
    flags_state: &RemoteState,
    pending: bool,
) -> TriageState {
    let snoozed = existing.state == TriageState::Snoozed;
    if pending || (snoozed && flags_state == &RemoteState::Archived) {
        existing.state
    } else {
        match flags_state {
            RemoteState::Inbox => TriageState::Inbox,
            RemoteState::Archived => TriageState::Archived,
            RemoteState::Trash => TriageState::Deleted,
            RemoteState::Folder(_) => TriageState::Archived,
        }
    }
}

/// Whether a local change is still waiting for the provider to confirm it.
fn has_pending(cache: &Cache, existing: &Message) -> bool {
    let stored = cache.remote_state(existing.id).ok().flatten().map(|(_, _, s)| s);
    let want = to_remote(existing.state, |f| cache.folder_remote_id(f).ok().flatten());
    want != stored
}

/// Apply server flags to cached messages, without fetching them.
fn apply_flags(
    mb: &mut Mailbox,
    cache: &Cache,
    account: &str,
    by_path: &HashMap<&str, &str>,
    by_id: &HashMap<&str, &str>,
    flags: &[(RemoteId, RemoteFlags)],
) -> Result<(), String> {
    for (remote_id, remote) in flags {
        let Some(id) = cache.message_by_remote(account, remote_id).map_err(err)? else {
            continue;
        };
        let Some(existing) = mb.get(id).cloned() else {
            continue;
        };
        let pending = has_pending(cache, &existing);
        let state = if pending || existing.state == TriageState::Snoozed {
            merged_state(&existing, &remote.state, pending)
        } else {
            from_remote(mb, cache, account, by_path, by_id, &remote.state)?
        };
        let read = match cache.remote_read(id).map_err(err)? {
            Some(confirmed) if confirmed != existing.read => existing.read,
            _ => !remote.unread,
        };
        let mut message = existing;
        message.state = state;
        message.read = read;
        // A snooze lives in the mailbox's metadata, not in the flags.
        message.snooze = mb.snoozed_until(id).map(format_rfc3339);
        mb.upsert_remote(message.clone());
        cache.upsert_message(&message, remote_id, &remote.state, !remote.unread).map_err(err)?;
    }
    Ok(())
}

/// Cached messages inside the window that the snapshot does not list have been
/// deleted on the server.
fn drop_vanished(
    mb: &mut Mailbox,
    cache: &Cache,
    account: &str,
    since: Timestamp,
    snapshot: &[(RemoteId, RemoteFlags)],
) -> Result<(), String> {
    let present: HashSet<&str> = snapshot.iter().map(|(id, _)| id.as_str()).collect();
    let gone: Vec<RemoteId> = cache
        .cached_messages()
        .map_err(err)?
        .into_iter()
        .filter(|m| m.account == account)
        .filter(|m| m.received_at().is_some_and(|t| t >= since))
        .filter_map(|m| cache.remote_state(m.id).ok().flatten().map(|(_, remote, _)| remote))
        .filter(|remote| !present.contains(remote.as_str()))
        .collect();
    remove_ids(mb, cache, account, &gone)
}

fn remove_ids(
    mb: &mut Mailbox,
    cache: &Cache,
    account: &str,
    removed: &[RemoteId],
) -> Result<(), String> {
    for remote_id in removed {
        let Some(id) = cache.message_by_remote(account, remote_id).map_err(err)? else {
            continue;
        };
        mb.remove_message(id);
        cache.delete_message(id).map_err(err)?;
    }
    Ok(())
}

/// Merge one header-only remote message. Returns whether it is new locally.
fn apply_message(
    mb: &mut Mailbox,
    cache: &Cache,
    account: &str,
    by_path: &HashMap<&str, &str>,
    by_id: &HashMap<&str, &str>,
    remote: &RemoteMessage,
) -> Result<bool, String> {
    let cached = cache.message_by_remote(account, &remote.id).map_err(err)?;
    let existing = cached.and_then(|id| mb.get(id).cloned());
    let state = match &existing {
        None => from_remote(mb, cache, account, by_path, by_id, &remote.state)?,
        Some(existing) => {
            let pending = has_pending(cache, existing);
            if pending || existing.state == TriageState::Snoozed {
                merged_state(existing, &remote.state, pending)
            } else {
                from_remote(mb, cache, account, by_path, by_id, &remote.state)?
            }
        }
    };
    let id = match cached {
        Some(id) => id,
        None => mb.next_message_id().max(cache.next_message_id().map_err(err)?),
    };
    let thread_id = cache
        .thread_id(account, &remote.thread, || {
            mb.next_thread_id().max(cache.next_thread_id().unwrap_or(1))
        })
        .map_err(err)?;
    let remote_read = !remote.unread;
    let read = match &existing {
        // A local read the server has not confirmed yet wins; unknown
        // (pre-migration) rows take the server's value.
        Some(existing) => match cache.remote_read(existing.id).map_err(err)? {
            Some(confirmed) if confirmed != existing.read => existing.read,
            _ => remote_read,
        },
        None => remote_read,
    };
    let snooze = mb.snoozed_until(id).map(format_rfc3339);
    // A downloaded body is never replaced by the snippet.
    let (body, html, attachments, partial) = match existing.as_ref().filter(|e| !e.partial) {
        Some(e) => (e.body.clone(), e.html.clone(), e.attachments.clone(), false),
        None => (remote.snippet.clone(), None, Vec::new(), true),
    };
    let message = Message {
        id,
        thread_id,
        from_name: remote.from_name.clone(),
        from_email: remote.from_email.clone(),
        to: remote.to.clone(),
        subject: remote.subject.clone(),
        body,
        received: format_rfc3339(remote.received),
        state,
        account: account.to_owned(),
        outgoing: remote.outgoing,
        snooze,
        cc: remote.cc.clone(),
        bcc: remote.bcc.clone(),
        html,
        attachments,
        read,
        partial,
    };
    mb.upsert_remote(message.clone());
    cache
        .upsert_message(&message, &remote.id, &remote.state, remote_read)
        .map_err(err)?;
    Ok(cached.is_none())
}

/// The local folder for a remote path, creating it (and any missing ancestor)
/// on the way.
fn ensure_folder(
    mb: &mut Mailbox,
    cache: &Cache,
    account: &str,
    by_path: &HashMap<&str, &str>,
    path: &str,
) -> Result<FolderId, String> {
    let (name, parent_path) = match path.rfind('/') {
        Some(slash) => (&path[slash + 1..], Some(&path[..slash])),
        None => (path, None),
    };
    // The parent path is only a parent when the provider lists it too.
    let parent = match parent_path.filter(|p| by_path.contains_key(*p)) {
        Some(parent) => Some(ensure_folder(mb, cache, account, by_path, parent)?),
        None => None,
    };
    let existing = mb
        .folders(account)
        .into_iter()
        .find(|f| f.parent == parent && f.name == name)
        .map(|f| f.id);
    if let Some(id) = existing {
        return Ok(id);
    }
    let folder = Folder {
        id: mb.next_folder_id().max(cache.next_folder_id().map_err(err)?),
        account: account.to_owned(),
        name: name.to_owned(),
        parent,
    };
    mb.upsert_folder(folder.clone());
    if let Some(rid) = by_path.get(path) {
        cache.upsert_folder(&folder, Some(rid)).map_err(err)?;
    }
    Ok(folder.id)
}

// -------------------------------------------------------------- load-more

/// The remote scopes behind a view, for the accounts that are cached.
pub fn scopes_for(mb: &Mailbox, cache: &Cache, location: &Location) -> Vec<(String, Scope)> {
    let scoped = |account: &str, scope: Scope| -> Vec<(String, Scope)> {
        if cache.has_account(account).unwrap_or(false) {
            vec![(account.to_owned(), scope)]
        } else {
            Vec::new()
        }
    };
    match location {
        Location::AllInboxes => cache
            .account_ids()
            .unwrap_or_default()
            .into_iter()
            .map(|account| (account, Scope::Inbox))
            .collect(),
        Location::Inbox(account) => scoped(account, Scope::Inbox),
        Location::Archive(account) => scoped(account, Scope::Archive),
        Location::Trash(account) => scoped(account, Scope::Trash),
        Location::Folder(folder) => {
            let Some(f) = mb.folder(*folder) else { return Vec::new() };
            let Ok(Some(remote)) = cache.folder_remote_id(f.id) else {
                return Vec::new();
            };
            scoped(&f.account, Scope::Folder(remote))
        }
        Location::Snoozed(_) | Location::Sent(_) => Vec::new(),
    }
}

/// Whether older mail beyond the cached window may still exist for this scope.
pub fn has_older(cache: &Cache, account: &str, scope: &Scope) -> bool {
    match cache.backfill(account, scope) {
        Ok(state) => !state.recent_done || !state.older_done,
        Err(_) => true,
    }
}

// ------------------------------------------------------------------ bodies

/// A body download for a message that is only known by its headers.
pub struct BodyRequest {
    pub id: MessageId,
    pub account: String,
    pub remote_id: RemoteId,
}

/// The body download needed for `id`, if any: a cached remote message that is
/// still only a snippet.
pub fn body_request(mb: &Mailbox, cache: &Cache, id: MessageId) -> Option<BodyRequest> {
    let message = mb.get(id)?;
    if !message.partial {
        return None;
    }
    let (account, remote_id, _) = cache.remote_state(id).ok().flatten()?;
    Some(BodyRequest { id: message.id, account, remote_id })
}

/// Download one message body, off the main thread.
pub fn run_body(provider: &SharedProvider, req: BodyRequest) -> (BodyRequest, Result<Body, ProviderError>) {
    let (id, account, remote_id) = (req.id, req.account, req.remote_id);
    let result = match lock(provider) {
        Ok(mut p) => p.fetch_body(&remote_id),
        Err(e) => Err(e),
    };
    (BodyRequest { id, account, remote_id }, result)
}

/// Store a downloaded body, on the main thread.
pub fn apply_body(mb: &mut Mailbox, cache: &Cache, req: &BodyRequest, body: Body) -> Result<(), String> {
    if mb.get(req.id).is_none() {
        return Ok(());
    }
    mb.set_body(req.id, body.body, body.html, body.attachments);
    let Some(updated) = mb.get(req.id) else {
        return Ok(());
    };
    cache.save_local(updated).map_err(err)
}

// -------------------------------------------------------------- persistence

/// Write local-only changes (pending states, snoozes, reads) back to the cache so
/// they survive a restart.
pub fn persist_local(mb: &Mailbox, cache: &Cache) {
    let Ok(cached) = cache.cached_messages() else {
        return;
    };
    let by_id: HashMap<MessageId, Message> = cached.into_iter().map(|m| (m.id, m)).collect();
    for message in mb.messages() {
        let Some(old) = by_id.get(&message.id) else {
            continue;
        };
        if old.state == message.state
            && old.snooze == message.snooze
            && old.read == message.read
            && old.partial == message.partial
        {
            continue;
        }
        let mut next = message.clone();
        next.snooze = if message.state == TriageState::Snoozed {
            mb.snoozed_until(message.id).map(format_rfc3339)
        } else {
            None
        };
        let _ = cache.save_local(&next);
    }
}
