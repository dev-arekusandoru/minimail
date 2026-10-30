//! Durable drafts on disk: every change is written immediately, holds survive a
//! restart, and a stale write is refused rather than silently applied.
//!
//! Blocking by design — each method holds the database lock for one short
//! SQLite transaction only. The store is cloneable, so the serial worker and the
//! local save lane share one connection without ever holding the lock across a
//! provider call, a file copy, or another locking method.

use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use rusqlite::{Connection, OptionalExtension};

use crate::clock::Timestamp;

use super::attachment::{AttachmentId, ManagedAttachment};
use super::draft::{DraftId, DraftRecord, DraftState};
use super::hold::PendingHold;

#[derive(Debug)]
pub enum DraftError {
    Sqlite(rusqlite::Error),
    /// Somebody else saved this draft since it was read. Carries the revisions
    /// that were compared: the row's total `revision`, or its `editor_revision`
    /// for a local editor save.
    StaleRevision {
        expected: u64,
        actual: u64,
    },
    /// The provider's copy moved on; the local copy is untouched.
    RemoteConflict {
        local: String,
        remote: String,
    },
    NotFound(DraftId),
    Corrupt(String),
    /// The draft is in a state a local edit must not disturb: a held or in-flight
    /// send, a delivery in doubt, or an already sent message.
    NotEditable(DraftState),
    /// The database lock was poisoned by a panic elsewhere; the file may be
    /// half-written, so nothing is attempted.
    Poisoned,
}

impl fmt::Display for DraftError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DraftError::Sqlite(e) => write!(f, "draft database: {e}"),
            DraftError::StaleRevision { expected, actual } => {
                write!(
                    f,
                    "draft changed since it was read (expected revision {expected}, stored {actual})"
                )
            }
            DraftError::RemoteConflict { local, remote } => {
                write!(f, "provider draft changed (local {local}, remote {remote})")
            }
            DraftError::NotFound(id) => write!(f, "draft {id} is gone"),
            DraftError::Corrupt(m) => write!(f, "draft is corrupt: {m}"),
            DraftError::NotEditable(state) => {
                write!(f, "draft is {state:?} and cannot be edited right now")
            }
            DraftError::Poisoned => write!(f, "draft database lock is poisoned"),
        }
    }
}

impl std::error::Error for DraftError {}

impl From<rusqlite::Error> for DraftError {
    fn from(e: rusqlite::Error) -> Self {
        DraftError::Sqlite(e)
    }
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS drafts(
    id INTEGER PRIMARY KEY,
    json TEXT NOT NULL,
    revision INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS holds(
    draft INTEGER PRIMARY KEY,
    json TEXT NOT NULL
);
";

/// What a restart did to the persisted drafts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Recovery {
    /// Held sends that never left the app and are editable again.
    pub restored: Vec<DraftId>,
    /// Sends that may still be in flight and need reconciliation.
    pub uncertain: Vec<DraftId>,
    /// Send-later holds: still waiting for their time, kept as they were.
    pub scheduled: Vec<PendingHold>,
}

/// A handle on the draft database. Clones share one connection, so the worker
/// and the local save lane write the same rows.
#[derive(Clone)]
pub struct DraftStore {
    conn: Arc<Mutex<Connection>>,
}

impl DraftStore {
    /// Open (creating if needed) the store at `path`, creating its directory.
    pub fn open(path: &Path) -> Result<Self, DraftError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| DraftError::Corrupt(format!("{}: {e}", dir.display())))?;
        }
        Self::connect(path)
    }

    /// A throwaway store for tests.
    pub fn open_in_memory() -> Result<Self, DraftError> {
        Self::connect(Path::new(":memory:"))
    }

    /// `drafts.db` next to the mail cache.
    pub fn default_path() -> PathBuf {
        crate::sync::cache::default_cache_path()
            .parent()
            .map_or_else(|| PathBuf::from("drafts.db"), |p| p.join("drafts.db"))
    }

    fn connect(path: &Path) -> Result<Self, DraftError> {
        let conn = Connection::open(path)?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// The connection, for the length of one short transaction. A poisoned lock
    /// is surfaced, never swallowed: the file may be half-written.
    fn lock(&self) -> Result<MutexGuard<'_, Connection>, DraftError> {
        self.conn.lock().map_err(|_| DraftError::Poisoned)
    }

    /// Every draft, most recently touched first.
    pub fn list(&self) -> Result<Vec<DraftRecord>, DraftError> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare("SELECT json FROM drafts ORDER BY updated_at DESC, id DESC")?;
        let drafts = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .filter_map(Result::ok)
            .map(|json| decode(&json))
            .collect::<Result<Vec<DraftRecord>, DraftError>>()?;
        Ok(drafts)
    }

    pub fn get(&self, id: DraftId) -> Result<Option<DraftRecord>, DraftError> {
        let conn = self.lock()?;
        let json: Option<String> = conn
            .query_row("SELECT json FROM drafts WHERE id = ?1", [id.0], |row| {
                row.get(0)
            })
            .optional()?;
        json.map(|j| decode(&j)).transpose()
    }

    /// Insert a draft, assigning its id and first revision. The record is
    /// updated in place so the caller keeps the identity. A fresh draft has
    /// mirrored nothing yet, so every counter starts at zero.
    pub fn create(&self, draft: &mut DraftRecord) -> Result<DraftRecord, DraftError> {
        let mut conn = self.lock()?;
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO drafts(json, revision, updated_at) VALUES ('', 0, 0)",
            [],
        )?;
        let id = DraftId(tx.last_insert_rowid());
        let mut stored = draft.clone();
        stored.id = id;
        stored.revision = 1;
        stored.editor_revision = 0;
        stored.content_revision = 0;
        stored.synced_content_revision = None;
        write(&tx, &stored)?;
        tx.commit()?;
        *draft = stored.clone();
        Ok(stored)
    }

    /// Save a draft whose stored *total* revision must still be `expected`. The
    /// record is only updated in place once the write succeeded.
    ///
    /// The counters are recomputed from what actually changed against the stored
    /// row, never taken from the caller, so a save that only re-writes the same
    /// fields cannot make the draft look edited or newly mirrored.
    pub fn save(&self, draft: &mut DraftRecord, expected: u64) -> Result<DraftRecord, DraftError> {
        let mut conn = self.lock()?;
        let tx = conn.transaction()?;
        check_revision(&tx, draft.id, expected)?;
        let current = fetch(&tx, draft.id)?.ok_or(DraftError::NotFound(draft.id))?;
        let mut stored = draft.clone();
        stored.note_change_since(&current);
        stored.synced_content_revision =
            synced_after(&current, &stored, stored.synced_content_revision);
        stored.mirrored_now = false;
        stored.revision = expected + 1;
        write(&tx, &stored)?;
        tx.commit()?;
        *draft = stored.clone();
        Ok(stored)
    }

    /// Persist what the UI has, keeping every worker-owned fact of the row.
    ///
    /// `expected_editor_revision` is the `editor_revision` the editor last saw,
    /// so a save is refused only when *another editor* moved the text on —
    /// concurrent attachment, binding or send-state writes do not refuse it.
    /// The editor's own fields are written verbatim: partial recipient input and
    /// blanks persist, because validation belongs to sync and send.
    ///
    /// The latest attachments, the imported baseline, the Message-ID, the remote
    /// binding and the attempted flag come from the stored row, never from the
    /// incoming snapshot, which may predate them. `Conflict` is preserved until
    /// it is resolved explicitly; a genuine edit may clear `Failed`.
    pub fn save_local(
        &self,
        draft: DraftRecord,
        expected_editor_revision: u64,
    ) -> Result<DraftRecord, DraftError> {
        let mut conn = self.lock()?;
        let tx = conn.transaction()?;
        let current = fetch(&tx, draft.id)?.ok_or(DraftError::NotFound(draft.id))?;
        if current.editor_revision != expected_editor_revision {
            return Err(DraftError::StaleRevision {
                expected: expected_editor_revision,
                actual: current.editor_revision,
            });
        }
        if matches!(
            current.state,
            DraftState::Held
                | DraftState::Sending
                | DraftState::Uncertain
                | DraftState::Sent { .. }
        ) {
            return Err(DraftError::NotEditable(current.state));
        }

        let mut next = current.clone();
        next.revision = current.revision + 1;
        next.updated_at = draft.updated_at;
        next.account = draft.account.clone();
        next.kind = draft.kind;
        next.to = draft.to.clone();
        next.cc = draft.cc.clone();
        next.bcc = draft.bcc.clone();
        next.body = draft.body.clone();
        next.set_intent(draft.intent());

        let account_moved = draft.account != current.account;
        let subject_changed = draft.subject.trim() != current.subject.trim();
        next.subject = draft.subject.clone();
        if subject_changed {
            // A subject the user rewrote is no longer the one it was prefilled
            // with, so the draft no longer answers that thread.
            next.clear_threading();
        } else if fresh_reply_context(&current, &draft) {
            // Threading fetched while this editor was typing: take it, but only
            // when it is actually there. An editor that has not seen it yet must
            // not erase it with an empty snapshot.
            next.in_reply_to.clone_from(&draft.in_reply_to);
            next.references.clone_from(&draft.references);
            next.thread_remote_id.clone_from(&draft.thread_remote_id);
        }
        if account_moved {
            // Another account has never heard of this draft, so the provider
            // binding and the mirror marker go with it.
            next.remote_id = None;
            next.remote_revision = None;
            next.thread_remote_id = None;
            next.synced_content_revision = None;
        }

        next.note_change_since(&current);
        if matches!(current.state, DraftState::Failed { .. })
            && next.editor_revision > current.editor_revision
        {
            next.state = DraftState::Editing;
        }

        // Nothing actually changed: leave the row (and its revision) alone.
        let unchanged = !next.editor_changed_since(&current)
            && !next.content_changed_since(&current)
            && next.state == current.state
            && next.updated_at == current.updated_at
            && next.synced_content_revision == current.synced_content_revision;
        if unchanged {
            tx.commit()?;
            return Ok(current);
        }
        write(&tx, &next)?;
        tx.commit()?;
        Ok(next)
    }

    /// Apply `edit` to the current row in one short transaction.
    ///
    /// This is how network and file work lands: it changes only its own metadata
    /// or attachments, onto whatever the body has become since the call started.
    /// A no-op writes nothing and does not advance the row revision.
    pub fn update_current(
        &self,
        id: DraftId,
        edit: impl FnOnce(&mut DraftRecord) -> Result<(), DraftError>,
    ) -> Result<DraftRecord, DraftError> {
        let mut conn = self.lock()?;
        let tx = conn.transaction()?;
        let current = fetch(&tx, id)?.ok_or(DraftError::NotFound(id))?;
        let mut next = current.clone();
        edit(&mut next)?;
        next.note_change_since(&current);
        next.synced_content_revision = synced_after(&current, &next, next.synced_content_revision);
        next.mirrored_now = false;

        let mut probe = next.clone();
        probe.revision = current.revision;
        probe.updated_at = current.updated_at;
        if probe == current {
            tx.commit()?;
            return Ok(current);
        }
        next.revision = current.revision + 1;
        write(&tx, &next)?;
        tx.commit()?;
        Ok(next)
    }

    /// Remove a draft and its hold. Returns what was there.
    pub fn delete(&self, id: DraftId) -> Result<Option<DraftRecord>, DraftError> {
        let mut conn = self.lock()?;
        let tx = conn.transaction()?;
        let removed = fetch(&tx, id)?;
        if removed.is_some() {
            tx.execute("DELETE FROM drafts WHERE id = ?1", [id.0])?;
            tx.execute("DELETE FROM holds WHERE draft = ?1", [id.0])?;
        }
        tx.commit()?;
        Ok(removed)
    }

    pub fn set_pending(&self, hold: &PendingHold) -> Result<(), DraftError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO holds(draft, json) VALUES (?1, ?2)
             ON CONFLICT(draft) DO UPDATE SET json = excluded.json",
            rusqlite::params![hold.draft.0, encode(hold)?],
        )?;
        Ok(())
    }

    pub fn pending_holds(&self) -> Result<Vec<PendingHold>, DraftError> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare("SELECT json FROM holds ORDER BY draft")?;
        let holds = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .filter_map(Result::ok)
            .map(|json| {
                serde_json::from_str(&json).map_err(|e| DraftError::Corrupt(format!("hold: {e}")))
            })
            .collect::<Result<Vec<PendingHold>, DraftError>>()?;
        Ok(holds)
    }

    pub fn clear_pending(&self, id: DraftId) -> Result<Option<PendingHold>, DraftError> {
        let mut conn = self.lock()?;
        let tx = conn.transaction()?;
        let json: Option<String> = tx
            .query_row("SELECT json FROM holds WHERE draft = ?1", [id.0], |row| {
                row.get(0)
            })
            .optional()?;
        tx.execute("DELETE FROM holds WHERE draft = ?1", [id.0])?;
        tx.commit()?;
        json.map(|j| {
            serde_json::from_str(&j).map_err(|e| DraftError::Corrupt(format!("hold: {e}")))
        })
        .transpose()
    }

    /// Persist `Sending` with `attempted` set and drop the hold in one commit,
    /// so a crash can never find a sending draft without the evidence that a
    /// request was issued.
    pub fn begin_send(
        &self,
        draft: &mut DraftRecord,
        now: Timestamp,
    ) -> Result<DraftRecord, DraftError> {
        let expected = draft.revision;
        let mut conn = self.lock()?;
        let tx = conn.transaction()?;
        check_revision(&tx, draft.id, expected)?;
        let current = fetch(&tx, draft.id)?.ok_or(DraftError::NotFound(draft.id))?;
        let mut stored = draft.clone();
        stored.state = DraftState::Sending;
        stored.note_send_attempt();
        stored.touch(now);
        stored.note_change_since(&current);
        stored.revision = expected + 1;
        write(&tx, &stored)?;
        tx.execute("DELETE FROM holds WHERE draft = ?1", [draft.id.0])?;
        tx.commit()?;
        *draft = stored.clone();
        Ok(stored)
    }

    /// Record how a send ended, dropping any hold in the same commit.
    ///
    /// `state` is what the provider settled on: only `Uncertain` (and the
    /// in-flight states) mean the message may still arrive, so only those keep
    /// `attempted`. A definite rejection, an offline send or a conflict clears
    /// it, otherwise every restart would keep such a draft uncertain forever.
    pub fn finish_send(
        &self,
        draft: &mut DraftRecord,
        state: DraftState,
        now: Timestamp,
    ) -> Result<DraftRecord, DraftError> {
        let expected = draft.revision;
        let mut conn = self.lock()?;
        let tx = conn.transaction()?;
        check_revision(&tx, draft.id, expected)?;
        let current = fetch(&tx, draft.id)?.ok_or(DraftError::NotFound(draft.id))?;
        let mut stored = draft.clone();
        if matches!(
            state,
            DraftState::Failed { .. } | DraftState::Editing | DraftState::Conflict
        ) {
            stored.attempted = false;
        }
        stored.state = state;
        stored.touch(now);
        stored.note_change_since(&current);
        stored.revision = expected + 1;
        write(&tx, &stored)?;
        tx.execute("DELETE FROM holds WHERE draft = ?1", [draft.id.0])?;
        tx.commit()?;
        *draft = stored.clone();
        Ok(stored)
    }

    /// Every attachment any stored draft still references.
    pub fn referenced_attachments(&self) -> Result<Vec<ManagedAttachment>, DraftError> {
        let mut out = Vec::new();
        for draft in self.list()? {
            out.extend(draft.attachments);
        }
        Ok(out)
    }

    /// The attachment ids no draft references any more, so their bytes can be
    /// pruned once the draft that detached them is gone.
    pub fn detached_attachments(
        &self,
        drafts: &[ManagedAttachment],
    ) -> Result<Vec<AttachmentId>, DraftError> {
        let referenced: HashSet<AttachmentId> = self
            .referenced_attachments()?
            .into_iter()
            .map(|a| a.id)
            .collect();
        Ok(drafts
            .iter()
            .filter(|a| !referenced.contains(&a.id))
            .map(|a| a.id.clone())
            .collect())
    }

    /// Conservative restart recovery over *every* stored draft, not just the
    /// ones with a hold row: a hold can be cleared before the request is
    /// issued. A held send never left the app and becomes editable again; a
    /// send that was in flight becomes uncertain, keeping its Message-ID for
    /// reconciliation. Nothing is sent on startup.
    pub fn recover(&self, now: Timestamp) -> Result<Recovery, DraftError> {
        let mut conn = self.lock()?;
        let tx = conn.transaction()?;
        let mut report = Recovery::default();
        let scheduled: Vec<PendingHold> = {
            let mut stmt = tx.prepare("SELECT json FROM holds")?;
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<String>, rusqlite::Error>>()?;
            rows.iter()
                .map(|json| {
                    serde_json::from_str::<PendingHold>(json)
                        .map_err(|e| DraftError::Corrupt(format!("hold: {e}")))
                })
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .filter(PendingHold::is_scheduled)
                .collect()
        };
        let mut stmt = tx.prepare("SELECT json FROM drafts")?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<String>, rusqlite::Error>>()?;
        drop(stmt);
        for json in rows {
            let mut draft: DraftRecord = decode(&json)?;
            let state = match (&draft.state, draft.attempted) {
                (DraftState::Held, _) => {
                    // A send-later hold is a promise to send, not an interrupted countdown.
                    if let Some(hold) = scheduled.iter().find(|h| h.draft == draft.id) {
                        report.scheduled.push(hold.clone());
                        continue;
                    }
                    report.restored.push(draft.id);
                    Some(DraftState::Editing)
                }
                (DraftState::Sending, _) | (DraftState::Uncertain, _) => {
                    report.uncertain.push(draft.id);
                    Some(DraftState::Uncertain)
                }
                (_, true) => {
                    // A send was attempted but the state is not in flight any
                    // more: only a confirmed send may claim otherwise.
                    if !matches!(draft.state, DraftState::Sent { .. }) {
                        report.uncertain.push(draft.id);
                        Some(DraftState::Uncertain)
                    } else {
                        None
                    }
                }
                _ => None,
            };
            let Some(state) = state else {
                continue;
            };
            let previous = draft.clone();
            draft.state = state;
            draft.touch(now);
            draft.note_change_since(&previous);
            draft.revision += 1;
            write(&tx, &draft)?;
        }
        tx.execute("DELETE FROM holds", [])?;
        for hold in &report.scheduled {
            tx.execute(
                "INSERT INTO holds(draft, json) VALUES (?1, ?2)",
                rusqlite::params![hold.draft.0, encode(hold)?],
            )?;
        }
        tx.commit()?;
        report.restored.sort();
        report.uncertain.sort();
        Ok(report)
    }
}

/// Whether `draft` carries reply context the stored row does not have yet: a
/// context fetched while the editor was typing, rather than an empty snapshot.
fn fresh_reply_context(current: &DraftRecord, draft: &DraftRecord) -> bool {
    let nonempty = draft
        .in_reply_to
        .as_deref()
        .is_some_and(|id| !id.trim().is_empty())
        || draft.references.iter().any(|r| !r.trim().is_empty());
    nonempty
        && (draft.in_reply_to != current.in_reply_to
            || draft.references != current.references
            || draft.thread_remote_id != current.thread_remote_id)
}

/// The mirror marker a write may claim.
///
/// A claim may name any content revision this row has already reached, because
/// `content_revision` only advances on a real content change: that is exactly
/// the snapshot a provider call uploaded while the user kept typing. A claim
/// *above* the row's revision is refused — it would mark content the row has
/// never had as mirrored. Otherwise the previous marker stands, so a content
/// change leaves the row visibly unmirrored rather than silently claiming
/// otherwise.
fn synced_after(current: &DraftRecord, next: &DraftRecord, claimed: Option<u64>) -> Option<u64> {
    if next.account != current.account {
        return None;
    }
    if next.mirrored_now {
        // The caller says the provider holds whatever this write ends up being,
        // so the marker is the finished content revision, not a guess.
        return Some(next.content_revision);
    }
    match claimed {
        Some(mirrored) if mirrored <= next.content_revision => Some(mirrored),
        _ => current.synced_content_revision,
    }
}

fn encode<T: serde::Serialize>(value: &T) -> Result<String, DraftError> {
    serde_json::to_string(value).map_err(|e| DraftError::Corrupt(format!("cannot serialize: {e}")))
}

fn decode(json: &str) -> Result<DraftRecord, DraftError> {
    if json.is_empty() {
        return Err(DraftError::Corrupt("stored draft row is empty".to_owned()));
    }
    serde_json::from_str(json).map_err(|e| DraftError::Corrupt(e.to_string()))
}

fn fetch(conn: &Connection, id: DraftId) -> Result<Option<DraftRecord>, DraftError> {
    let json: Option<String> = conn
        .query_row("SELECT json FROM drafts WHERE id = ?1", [id.0], |row| {
            row.get(0)
        })
        .optional()?;
    json.map(|j| decode(&j)).transpose()
}

fn check_revision(conn: &Connection, id: DraftId, expected: u64) -> Result<(), DraftError> {
    let actual: Option<u64> = conn
        .query_row("SELECT revision FROM drafts WHERE id = ?1", [id.0], |row| {
            row.get(0)
        })
        .optional()?;
    match actual {
        None => Err(DraftError::NotFound(id)),
        Some(actual) if actual != expected => Err(DraftError::StaleRevision { expected, actual }),
        Some(_) => Ok(()),
    }
}

fn write(conn: &Connection, draft: &DraftRecord) -> Result<(), DraftError> {
    let json = encode(draft)?;
    conn.execute(
        "INSERT INTO drafts(id, json, revision, updated_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(id) DO UPDATE SET json = excluded.json, revision = excluded.revision,
             updated_at = excluded.updated_at",
        rusqlite::params![draft.id.0, json, draft.revision, draft.updated_at],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outgoing::attachment::{AttachmentSource, AttachmentStore};

    fn store() -> DraftStore {
        DraftStore::open_in_memory().expect("store")
    }

    fn draft() -> DraftRecord {
        let mut draft = DraftRecord::new_email("me@x.test", 100);
        draft.to = "sam@x.test".to_owned();
        draft.body.set_source("hello");
        draft
    }

    fn attachment(tag: &str) -> ManagedAttachment {
        let dir = std::env::temp_dir().join(format!(
            "mail-classifier-store-{tag}-{}",
            std::process::id()
        ));
        AttachmentStore::open(&dir)
            .expect("store")
            .import_bytes("a.txt", tag.as_bytes(), AttachmentSource::Generated, None)
            .expect("import")
    }

    #[test]
    fn drafts_survive_a_reopen() {
        let path =
            std::env::temp_dir().join(format!("mail-classifier-db-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let (id, message_id) = {
            let store = DraftStore::open(&path).expect("open");
            let mut d = draft();
            let stored = store.create(&mut d).expect("create");
            (stored.id, stored.message_id)
        };
        let store = DraftStore::open(&path).expect("reopen");
        let loaded = store.get(id).expect("get").expect("present");
        assert_eq!(loaded.to, "sam@x.test");
        assert_eq!(loaded.body.source, "hello");
        assert_eq!(
            loaded.message_id, message_id,
            "the send attempt keeps its Message-ID"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn stale_writes_are_refused_and_change_nothing() {
        let store = store();
        let mut d = draft();
        store.create(&mut d).expect("create");
        let mut stale = d.clone();

        let expected = d.revision;
        store.save(&mut d, expected).expect("first save");
        let stale_revision = stale.revision;
        let err = store.save(&mut stale, stale_revision).expect_err("stale");
        assert!(matches!(err, DraftError::StaleRevision { .. }), "{err:?}");
        assert_eq!(stale.revision, 1, "the refused record is untouched");
        let stored = store.get(d.id).expect("get").expect("present");
        assert_eq!(stored.revision, 2);
    }

    #[test]
    fn holds_are_persisted_and_cleared() {
        let store = store();
        let mut d = draft();
        store.create(&mut d).expect("create");
        let hold = PendingHold::start(d.id, d.account.clone(), 100);
        store.set_pending(&hold).expect("hold");
        assert_eq!(store.pending_holds().expect("holds"), vec![hold.clone()]);

        let cleared = store.clear_pending(d.id).expect("clear");
        assert_eq!(cleared, Some(hold));
        assert!(store.pending_holds().expect("holds").is_empty());
    }

    #[test]
    fn beginning_a_send_is_atomic_with_clearing_the_hold() {
        let store = store();
        let mut d = draft();
        store.create(&mut d).expect("create");
        let hold = PendingHold::start(d.id, d.account.clone(), 100);
        store.set_pending(&hold).expect("hold");

        let sent = store.begin_send(&mut d, 105).expect("begin send");
        assert_eq!(sent.state, DraftState::Sending);
        assert!(sent.attempted);
        assert_eq!(sent.revision, 2);
        assert!(store.pending_holds().expect("holds").is_empty());
        assert!(store.get(d.id).expect("get").expect("present").attempted);
    }

    #[test]
    fn finishing_a_send_records_the_outcome() {
        let store = store();
        let mut d = draft();
        store.create(&mut d).expect("create");
        store.begin_send(&mut d, 105).expect("begin");
        let sent = store
            .finish_send(
                &mut d,
                DraftState::Sent {
                    remote_id: "m1".to_owned(),
                },
                106,
            )
            .expect("finish");
        assert_eq!(
            sent.state,
            DraftState::Sent {
                remote_id: "m1".to_owned()
            }
        );
        assert!(sent.attempted, "the attempt stays on the record");

        let mut failed = draft();
        store.create(&mut failed).expect("create");
        store.begin_send(&mut failed, 105).expect("begin");
        let uncertain = store
            .finish_send(&mut failed, DraftState::Uncertain, 106)
            .expect("finish");
        assert_eq!(uncertain.state, DraftState::Uncertain);
    }

    #[test]
    fn recovery_restores_holds_and_never_resets_an_in_flight_send() {
        let store = store();
        let mut held = draft();
        store.create(&mut held).expect("create");
        store
            .set_pending(&PendingHold::start(held.id, held.account.clone(), 100))
            .expect("hold");
        let expected = held.revision;
        store.save(&mut held, expected).expect("save");
        held.state = DraftState::Held;
        let expected = held.revision;
        store.save(&mut held, expected).expect("save held");

        let mut sending = draft();
        sending.to = "lee@x.test".to_owned();
        store.create(&mut sending).expect("create");
        store.begin_send(&mut sending, 105).expect("begin send");

        let report = store.recover(1_000).expect("recover");
        assert_eq!(report.restored, vec![held.id]);
        assert_eq!(report.uncertain, vec![sending.id]);

        let restored = store.get(held.id).expect("get").expect("present");
        assert_eq!(restored.state, DraftState::Editing);
        assert_eq!(restored.to, "sam@x.test");

        let uncertain = store.get(sending.id).expect("get").expect("present");
        assert_eq!(
            uncertain.state,
            DraftState::Uncertain,
            "a possible send is not an ordinary retry"
        );
        assert!(uncertain.attempted);
        assert_eq!(
            uncertain.message_id, sending.message_id,
            "the Message-ID survives for reconciliation"
        );
        assert!(
            store.pending_holds().expect("holds").is_empty(),
            "no hold outlives a restart"
        );
    }

    #[test]
    fn a_definitely_rejected_send_does_not_come_back_uncertain() {
        for state in [
            DraftState::Failed {
                error: "rejected".to_owned(),
            },
            DraftState::Editing,
            DraftState::Conflict,
        ] {
            let store = store();
            let mut d = draft();
            store.create(&mut d).expect("create");
            store.begin_send(&mut d, 105).expect("begin send");
            assert!(d.attempted);
            let settled = store
                .finish_send(&mut d, state.clone(), 106)
                .expect("finish");
            assert!(!settled.attempted, "{state:?} is not a delivery in doubt");

            let report = store.recover(1_000).expect("recover");
            assert!(
                report.uncertain.is_empty(),
                "{state:?} was reported uncertain"
            );
            let after = store.get(d.id).expect("get").expect("present");
            assert_eq!(after.state, state);
            assert!(
                after.state.is_editable(),
                "{state:?} must be editable again"
            );
        }
    }

    #[test]
    fn recovery_leaves_confirmed_sends_and_untouched_drafts_alone() {
        let store = store();
        let mut sent = draft();
        store.create(&mut sent).expect("create");
        store.begin_send(&mut sent, 105).expect("begin");
        store
            .finish_send(
                &mut sent,
                DraftState::Sent {
                    remote_id: "m1".to_owned(),
                },
                106,
            )
            .expect("finish");

        let mut editing = draft();
        editing.to = "kim@x.test".to_owned();
        store.create(&mut editing).expect("create");
        let revision = editing.revision;

        let report = store.recover(1_000).expect("recover");
        assert!(report.restored.is_empty());
        assert!(report.uncertain.is_empty());
        assert_eq!(
            store.get(sent.id).expect("get").expect("present").revision,
            3
        );
        assert_eq!(
            store
                .get(editing.id)
                .expect("get")
                .expect("present")
                .revision,
            revision
        );
    }

    #[test]
    fn recovery_keeps_a_send_later_hold_waiting() {
        let store = store();
        let mut later = draft();
        store.create(&mut later).expect("create");
        let hold = PendingHold::scheduled(later.id, later.account.clone(), 100, 9_000);
        store.set_pending(&hold).expect("hold");
        let expected = later.revision;
        later.state = DraftState::Held;
        store.save(&mut later, expected).expect("save");

        let report = store.recover(1_000).expect("recover");
        assert_eq!(report.scheduled, vec![hold.clone()]);
        assert!(report.restored.is_empty());
        assert_eq!(
            store.get(later.id).expect("get").expect("present").state,
            DraftState::Held
        );
        assert_eq!(store.pending_holds().expect("holds"), vec![hold]);
    }

    #[test]
    fn recovery_covers_sends_whose_hold_row_was_already_gone() {
        let store = store();
        let mut d = draft();
        store.create(&mut d).expect("create");
        store
            .set_pending(&PendingHold::start(d.id, d.account.clone(), 100))
            .expect("hold");
        store
            .clear_pending(d.id)
            .expect("hold cleared before the request");
        store.begin_send(&mut d, 105).expect("begin send");

        let report = store.recover(1_000).expect("recover");
        assert_eq!(report.uncertain, vec![d.id]);
        assert_eq!(
            store.get(d.id).expect("get").expect("present").state,
            DraftState::Uncertain
        );
    }

    #[test]
    fn attachments_stay_referenced_while_another_draft_uses_them() {
        let store = store();
        let shared = attachment("shared");
        let mut a = draft();
        a.attach(shared.clone());
        store.create(&mut a).expect("create");
        let mut b = draft();
        b.to = "kim@x.test".to_owned();
        b.attach(shared.clone());
        store.create(&mut b).expect("create");

        a.detach(&shared.id);
        let expected = a.revision;
        store.save(&mut a, expected).expect("save");
        let orphans = store
            .detached_attachments(std::slice::from_ref(&shared))
            .expect("orphans");
        assert!(orphans.is_empty(), "draft b still holds it");

        store.delete(b.id).expect("delete");
        let orphans = store
            .detached_attachments(std::slice::from_ref(&shared))
            .expect("orphans");
        assert_eq!(orphans, vec![shared.id]);
    }

    #[test]
    fn delete_removes_the_draft_and_its_hold() {
        let store = store();
        let mut d = draft();
        store.create(&mut d).expect("create");
        store
            .set_pending(&PendingHold::start(d.id, d.account.clone(), 100))
            .expect("hold");
        assert!(store.delete(d.id).expect("delete").is_some());
        assert!(store.get(d.id).expect("get").is_none());
        assert!(store.pending_holds().expect("holds").is_empty());
        assert!(store.delete(d.id).expect("delete again").is_none());
    }

    #[test]
    fn list_orders_by_recent_touch() {
        let store = store();
        let mut older = draft();
        older.to = "one@x.test".to_owned();
        store.create(&mut older).expect("create");
        let mut newer = draft();
        newer.to = "two@x.test".to_owned();
        store.create(&mut newer).expect("create");
        newer.touch(500);
        let expected = newer.revision;
        store.save(&mut newer, expected).expect("save");

        let ids: Vec<DraftId> = store
            .list()
            .expect("list")
            .into_iter()
            .map(|d| d.id)
            .collect();
        assert_eq!(ids, vec![newer.id, older.id]);
    }

    // --------------------------------------------------------------- counters

    #[test]
    fn a_local_save_keeps_worker_facts_and_moves_the_editor_counter() {
        let store = store();
        let file = attachment("merge");
        let mut d = draft();
        store.create(&mut d).expect("create");
        // The worker finished an attachment copy and bound the remote draft
        // while this editor's snapshot was already in hand.
        store
            .update_current(d.id, |row| {
                row.attach(file.clone());
                row.remote_id = Some("r1".to_owned());
                row.remote_revision = Some("rev-1".to_owned());
                row.synced_content_revision = Some(row.content_revision);
                Ok(())
            })
            .expect("worker update");

        let stale_total = d.revision;
        let editor_token = d.editor_revision;
        let mut edit = d.clone();
        edit.body.set_source("edited while the file landed");
        edit.to = "kim@x.test, ".to_owned();
        edit.updated_at = 200;

        let saved = store.save_local(edit, editor_token).expect("local save");
        assert!(
            saved.revision > stale_total,
            "the total revision still advances"
        );
        assert_eq!(saved.editor_revision, editor_token + 1);
        assert_eq!(saved.body.source, "edited while the file landed");
        assert_eq!(
            saved.to, "kim@x.test, ",
            "a half-typed recipient field persists"
        );
        assert_eq!(
            saved.attachments,
            vec![file.clone()],
            "the copied file is not clobbered"
        );
        assert_eq!(
            saved.remote_id,
            Some("r1".to_owned()),
            "the remote binding survives"
        );
        assert_eq!(
            saved.remote_revision,
            Some("rev-1".to_owned()),
            "the provider's concurrency token survives"
        );
        assert_ne!(
            saved.synced_content_revision,
            Some(saved.content_revision),
            "the edit is not mirrored"
        );
    }

    #[test]
    fn a_local_save_with_another_editors_token_changes_nothing() {
        let store = store();
        let mut d = draft();
        store.create(&mut d).expect("create");
        let first = store
            .update_current(d.id, |row| {
                row.body.set_source("typed by another editor");
                Ok(())
            })
            .expect("first edit");
        assert_eq!(first.editor_revision, 1);

        let mut mine = d.clone();
        mine.body.set_source("typed by a stale editor");
        let err = store.save_local(mine, 0).expect_err("stale editor");
        assert!(
            matches!(
                err,
                DraftError::StaleRevision {
                    expected: 0,
                    actual: 1
                }
            ),
            "{err:?}"
        );

        let stored = store.get(d.id).expect("get").expect("present");
        assert_eq!(
            stored.body.source, "typed by another editor",
            "the newer text stands"
        );
        assert_eq!(stored.revision, first.revision, "no row was written");
    }

    #[test]
    fn a_local_save_never_moves_a_counter_for_a_snapshot_that_changed_nothing() {
        let store = store();
        let mut d = draft();
        store.create(&mut d).expect("create");
        let stored = store
            .update_current(d.id, |row| {
                row.body.set_source("real text");
                Ok(())
            })
            .expect("edit");
        assert_eq!(stored.editor_revision, 1);
        assert_eq!(stored.content_revision, 1);

        // A snapshot that only moves `updated_at` forward writes the row, but it
        // is not an edit and not new content.
        let mut idle = stored.clone();
        idle.updated_at = 900;
        let again = store
            .save_local(idle, stored.editor_revision)
            .expect("save");
        assert_eq!(again.editor_revision, 1);
        assert_eq!(again.content_revision, 1);
    }

    #[test]
    fn a_body_mode_change_is_an_edit_but_not_new_content() {
        let store = store();
        let mut d = draft();
        store.create(&mut d).expect("create");
        let edited = store
            .update_current(d.id, |row| {
                row.body.set_source("hello");
                Ok(())
            })
            .expect("edit");
        let previewed = store
            .update_current(d.id, |row| {
                row.body.set_mode(crate::outgoing::BodyMode::Preview);
                Ok(())
            })
            .expect("preview");
        assert_eq!(previewed.editor_revision, edited.editor_revision + 1);
        assert_eq!(
            previewed.content_revision, edited.content_revision,
            "the payload is the same"
        );
    }

    #[test]
    fn a_shared_clone_sees_what_the_other_lane_committed() {
        let local_lane = store();
        let worker_lane = local_lane.clone();
        let mut d = draft();
        local_lane.create(&mut d).expect("create");

        let mut edit = d.clone();
        edit.body.set_source("durable before the provider answers");
        local_lane
            .save_local(edit, d.editor_revision)
            .expect("local save");

        let seen = worker_lane.get(d.id).expect("get").expect("present");
        assert_eq!(seen.body.source, "durable before the provider answers");
    }

    #[test]
    fn attachment_completion_lands_on_the_body_typed_while_it_was_copying() {
        let store = store();
        let file = attachment("concurrent");
        let mut d = draft();
        store.create(&mut d).expect("create");
        let typed = store
            .save_local(
                {
                    let mut edit = d.clone();
                    edit.body.set_source("typed during the copy");
                    edit
                },
                d.editor_revision,
            )
            .expect("local save");

        let landed = store
            .update_current(d.id, |row| {
                row.attach(file.clone());
                Ok(())
            })
            .expect("file completion");

        assert_eq!(
            landed.body.source, "typed during the copy",
            "the newer body survives"
        );
        assert_eq!(landed.attachments, vec![file]);
        assert_eq!(
            landed.content_revision,
            typed.content_revision + 1,
            "a new file is new content"
        );
        assert_eq!(
            landed.editor_revision, typed.editor_revision,
            "a file is not an editor change"
        );
    }

    #[test]
    fn a_mirror_claim_only_holds_for_the_content_that_was_actually_uploaded() {
        let store = store();
        let mut d = draft();
        store.create(&mut d).expect("create");
        let uploaded = store
            .update_current(d.id, |row| {
                row.body.set_source("what went up");
                row.note_mirrored();
                Ok(())
            })
            .expect("upload");
        assert_eq!(
            uploaded.synced_content_revision,
            Some(uploaded.content_revision)
        );

        let edited = store
            .save_local(
                {
                    let mut edit = uploaded.clone();
                    edit.body.set_source("typed after the upload");
                    edit
                },
                uploaded.editor_revision,
            )
            .expect("local save");
        assert_ne!(edited.content_revision, uploaded.content_revision);
        assert_eq!(
            edited.synced_content_revision,
            Some(uploaded.content_revision),
            "the old snapshot is still what the provider has"
        );
    }

    #[test]
    fn a_binding_that_lands_after_more_typing_marks_the_snapshot_it_uploaded() {
        let store = store();
        let mut d = draft();
        store.create(&mut d).expect("create");
        let uploaded = store
            .update_current(d.id, |row| {
                row.body.set_source("what the provider is holding");
                Ok(())
            })
            .expect("edit");
        // The provider call is out; the user keeps typing and the local save lands.
        let typed = store
            .save_local(
                {
                    let mut edit = uploaded.clone();
                    edit.body.set_source("what went up, plus more typing");
                    edit
                },
                uploaded.editor_revision,
            )
            .expect("local save");
        assert!(typed.content_revision > uploaded.content_revision);

        let bound = store
            .update_current(d.id, |row| {
                row.remote_id = Some("r1".to_owned());
                row.synced_content_revision = Some(uploaded.content_revision);
                Ok(())
            })
            .expect("binding completion");
        assert_eq!(bound.remote_id, Some("r1".to_owned()));
        assert_eq!(
            bound.body.source, typed.body.source,
            "the newer typing survives"
        );
        assert_eq!(
            bound.synced_content_revision,
            Some(uploaded.content_revision),
            "the provider holds the snapshot it was sent, not the newer typing"
        );
        assert_ne!(
            bound.synced_content_revision,
            Some(bound.content_revision),
            "the row is honestly unmirrored"
        );
    }

    #[test]
    fn a_mirror_claim_beyond_the_rows_content_is_refused() {
        let store = store();
        let mut d = draft();
        store.create(&mut d).expect("create");
        let err = store
            .update_current(d.id, |row| {
                row.synced_content_revision = Some(row.content_revision + 5);
                Ok(())
            })
            .expect("still written");
        assert_eq!(
            err.synced_content_revision, None,
            "content the row never had cannot be mirrored"
        );
    }

    #[test]
    fn moving_account_drops_the_binding_and_the_mirror_marker() {
        let store = store();
        let mut d = draft();
        store.create(&mut d).expect("create");
        let bound = store
            .update_current(d.id, |row| {
                row.remote_id = Some("r1".to_owned());
                row.remote_revision = Some("rev-1".to_owned());
                row.thread_remote_id = Some("t1".to_owned());
                row.note_mirrored();
                Ok(())
            })
            .expect("bind");

        let mut moved = bound.clone();
        moved.account = "other@x.test".into();
        moved.updated_at = 300;
        let saved = store
            .save_local(moved, bound.editor_revision)
            .expect("local save");
        assert_eq!(saved.account, "other@x.test");
        assert_eq!(
            saved.remote_id, None,
            "the other account never saw this draft"
        );
        assert_eq!(saved.remote_revision, None);
        assert_eq!(saved.thread_remote_id, None);
        assert_eq!(saved.synced_content_revision, None);
    }

    #[test]
    fn a_rewritten_subject_stops_claiming_the_thread_but_a_fetched_context_is_kept() {
        let store = store();
        let mut d = draft();
        d.prefilled_subject = Some("Re: invoice".to_owned());
        d.in_reply_to = Some("orig@x.test".to_owned());
        d.references = vec!["orig@x.test".to_owned()];
        store.create(&mut d).expect("create");

        // The editor has not seen the fetched context yet and its snapshot is
        // empty of it: the stored context must survive.
        let mut unaware = d.clone();
        unaware.body.set_source("answering");
        let kept = store
            .save_local(unaware, d.editor_revision)
            .expect("local save");
        assert_eq!(kept.in_reply_to, Some("orig@x.test".to_owned()));
        assert_eq!(kept.references, vec!["orig@x.test".to_owned()]);

        let mut renamed = kept.clone();
        renamed.subject = "New subject".to_owned();
        let rewritten = store
            .save_local(renamed, kept.editor_revision)
            .expect("local save");
        assert_eq!(rewritten.subject, "New subject");
        assert_eq!(
            rewritten.in_reply_to, None,
            "a rewritten subject is not that reply"
        );
        assert!(rewritten.references.is_empty());
        assert_eq!(rewritten.thread_remote_id, None);
    }

    #[test]
    fn a_local_save_refuses_a_held_or_in_flight_send_without_writing() {
        let store = store();
        let file = attachment("protected");
        let mut d = draft();
        store.create(&mut d).expect("create");
        store
            .update_current(d.id, |row| {
                row.attach(file.clone());
                row.body.set_source("original body");
                Ok(())
            })
            .expect("setup");
        let before = store.get(d.id).expect("get").expect("present");

        let mut flying = before.clone();
        flying.state = DraftState::Held;
        store.save(&mut flying, before.revision).expect("hold");
        let held = flying.clone();

        let mut edit = held.clone();
        edit.body.set_source("typed during the hold");
        let err = store
            .save_local(edit, held.editor_revision)
            .expect_err("protected");
        assert!(
            matches!(err, DraftError::NotEditable(DraftState::Held)),
            "{err:?}"
        );

        let after = store.get(d.id).expect("get").expect("present");
        assert_eq!(after.body.source, "original body");
        assert_eq!(after.revision, held.revision, "no row was written");
        assert_eq!(after.attachments, vec![file], "the data is not clobbered");
    }

    #[test]
    fn a_conflict_survives_an_edit_but_a_failure_is_cleared_by_one() {
        let store = store();
        for (start, expect) in [
            (DraftState::Conflict, DraftState::Conflict),
            (
                DraftState::Failed {
                    error: "no".to_owned(),
                },
                DraftState::Editing,
            ),
        ] {
            let mut d = draft();
            store.create(&mut d).expect("create");
            let created = d.revision;
            store.save(&mut d, created).expect("save");
            let after_create = d.revision;
            let mut conflict = d.clone();
            conflict.state = start.clone();
            store.save(&mut conflict, after_create).expect("state");
            let marked = conflict.clone();

            let mut edit = marked.clone();
            edit.body.set_source("user typed again");
            let saved = store
                .save_local(edit, marked.editor_revision)
                .expect("local save");
            assert_eq!(saved.state, expect, "{start:?}");
        }
    }

    #[test]
    fn a_no_op_update_writes_nothing() {
        let store = store();
        let mut d = draft();
        store.create(&mut d).expect("create");
        let untouched = store.update_current(d.id, |_| Ok(())).expect("no-op");
        assert_eq!(untouched.revision, d.revision);
    }

    #[test]
    fn a_row_that_cannot_be_written_is_an_error_not_an_empty_draft() {
        // What a dropped JSON body would leave behind: the row exists, but there is
        // nothing in it. Reading it must fail loudly instead of handing the caller a
        // blank draft that looks like the user's own.
        let store = store();
        let mut d = draft();
        store.create(&mut d).expect("create");
        {
            let conn = store.lock().expect("lock");
            conn.execute("UPDATE drafts SET json = '' WHERE id = ?1", [d.id.0])
                .expect("blank the row");
        }
        let err = store.get(d.id).expect_err("a blank row is corrupt");
        assert!(matches!(err, DraftError::Corrupt(_)), "{err:?}");
    }
}
