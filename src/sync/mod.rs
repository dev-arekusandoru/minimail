//! Mapping between local triage state and remote mail operations.
//!
//! Nothing here runs a clock or touches UI state: pending moves are a pure
//! diff of the mailbox against the last remote state the cache confirmed, and
//! each half of a sync round runs on its own thread (`background_pull` /
//! `run_moves` off-thread, `apply_*` on the main thread).

pub mod cache;

use crate::model::{
    Folder, FolderId, Mailbox, Message, MessageId, TriageState, format_rfc3339,
};
use crate::provider::{
    MailProvider, ProviderError, RemoteFolder, RemoteId, RemoteMessage, RemoteState,
};

use cache::Cache;

/// Messages a pull may import on the very first sync.
const INITIAL_LIMIT: usize = 500;

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
/// Stops at the first `Auth` or `Network` failure; moves after it are dropped
/// from the result so they stay pending for the next attempt. The returned
/// `Move`s carry the label id substituted into `to`.
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

fn is_fatal(e: &ProviderError) -> bool {
    matches!(e, ProviderError::Auth(_) | ProviderError::Network(_))
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

/// One round of provider reads, done off the main thread.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pull {
    pub folders: Vec<RemoteFolder>,
    pub fetched: Vec<RemoteMessage>,
    pub removed: Vec<RemoteId>,
    pub cursor: String,
}

/// Fetch what changed since `cursor`, falling back to a full recent listing
/// when the provider no longer knows that cursor.
pub fn background_pull(
    p: &mut dyn MailProvider,
    cursor: Option<String>,
) -> Result<Pull, ProviderError> {
    let folders = p.folders()?;
    let (fetched, removed, cursor) = match cursor.as_deref() {
        Some(cursor) => match p.changes(cursor) {
            Ok(changes) => {
                let messages = p.fetch(&changes.changed)?;
                (messages, changes.removed, changes.cursor)
            }
            Err(ProviderError::CursorExpired) => {
                let (ids, cursor) = p.recent(INITIAL_LIMIT)?;
                (p.fetch(&ids)?, Vec::new(), cursor)
            }
            Err(e) => return Err(e),
        },
        None => {
            let (ids, cursor) = p.recent(INITIAL_LIMIT)?;
            (p.fetch(&ids)?, Vec::new(), cursor)
        }
    };
    Ok(Pull {
        folders,
        fetched,
        removed,
        cursor,
    })
}

/// Merge a pull into the mailbox and the cache, and store the new cursor.
///
/// Every remote folder gets a local folder, even when no fetched message is in it.
///
/// A message with an unconfirmed local change keeps its local state; otherwise
/// the remote state wins, except that a snoozed message stays snoozed while it
/// is archived remotely.
pub fn apply_fetched(
    mb: &mut Mailbox,
    cache: &Cache,
    account: &str,
    pull: &Pull,
) -> Result<(), String> {
    let by_path: std::collections::HashMap<&str, &str> = pull
        .folders
        .iter()
        .map(|f| (f.path.as_str(), f.id.as_str()))
        .collect();
    let by_id: std::collections::HashMap<&str, &str> = pull
        .folders
        .iter()
        .map(|f| (f.id.as_str(), f.path.as_str()))
        .collect();

    for folder in &pull.folders {
        ensure_folder(mb, cache, account, &by_path, &folder.path)?;
    }

    for remote in &pull.fetched {
        let cached = cache
            .message_by_remote(account, &remote.id)
            .map_err(err)?;
        let existing = cached.and_then(|id| mb.get(id).cloned());
        let filed = |mb: &mut Mailbox, rid: &str| -> Result<TriageState, String> {
            match by_id.get(rid).copied() {
                Some(path) => Ok(TriageState::Filed(ensure_folder(
                    mb, cache, account, &by_path, path,
                )?)),
                None => Ok(TriageState::Archived),
            }
        };

        let state = match &existing {
            None => match &remote.state {
                RemoteState::Inbox => TriageState::Inbox,
                RemoteState::Archived => TriageState::Archived,
                RemoteState::Trash => TriageState::Deleted,
                RemoteState::Folder(rid) => filed(mb, rid)?,
            },
            Some(existing) => {
                let stored = cache
                    .remote_state(existing.id)
                    .map_err(err)?
                    .map(|(_, _, s)| s);
                let want = to_remote(existing.state, |f| {
                    cache.folder_remote_id(f).ok().flatten()
                });
                // A local change the provider has not confirmed yet wins.
                let pending = want != stored;
                let snoozed = existing.state == TriageState::Snoozed;
                if pending || (snoozed && remote.state == RemoteState::Archived) {
                    existing.state
                } else {
                    match &remote.state {
                        RemoteState::Inbox => TriageState::Inbox,
                        RemoteState::Archived => TriageState::Archived,
                        RemoteState::Trash => TriageState::Deleted,
                        RemoteState::Folder(rid) => filed(mb, rid)?,
                    }
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
        let snooze = mb.snoozed_until(id).map(format_rfc3339);
        let message = Message {
            id,
            thread_id,
            from_name: remote.from_name.clone(),
            from_email: remote.from_email.clone(),
            to: remote.to.clone(),
            subject: remote.subject.clone(),
            body: remote.body.clone(),
            received: format_rfc3339(remote.received),
            state,
            account: account.to_owned(),
            outgoing: remote.outgoing,
            snooze,
            cc: remote.cc.clone(),
            bcc: remote.bcc.clone(),
            html: remote.html.clone(),
            attachments: remote.attachments.clone(),
        };
        mb.upsert_remote(message.clone());
        cache.upsert_message(&message, &remote.id, &remote.state).map_err(err)?;
    }

    for rid in &pull.removed {
        let Some(id) = cache.message_by_remote(account, rid).map_err(err)? else {
            continue;
        };
        mb.remove_message(id);
        cache.delete_message(id).map_err(err)?;
    }

    cache.set_cursor(account, &pull.cursor).map_err(err)
}

/// The local folder for a remote path, creating it (and any missing ancestor)
/// on the way.
fn ensure_folder(
    mb: &mut Mailbox,
    cache: &Cache,
    account: &str,
    by_path: &std::collections::HashMap<&str, &str>,
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

fn err(e: rusqlite::Error) -> String {
    e.to_string()
}

/// Write local-only changes (pending states and snoozes) back to the cache so
/// they survive a restart.
pub fn persist_local(mb: &Mailbox, cache: &Cache) {
    let Ok(cached) = cache.cached_messages() else {
        return;
    };
    let by_id: std::collections::HashMap<MessageId, Message> =
        cached.into_iter().map(|m| (m.id, m)).collect();
    for message in mb.messages() {
        let Some(old) = by_id.get(&message.id) else {
            continue;
        };
        if old.state == message.state && old.snooze == message.snooze {
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