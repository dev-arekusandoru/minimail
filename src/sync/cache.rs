//! On-disk mirror of synced mail: accounts, folders, threads and messages.
//!
//! The `json` column holds a serialized [`crate::model::Message`] including
//! its local state and snooze string, so a restart restores pending triage and
//! snoozes; `remote_state` is the serialized [`RemoteState`] the provider last
//! confirmed, which is what pending triage is diffed against.

use crate::clock::{DAY, Timestamp};
use crate::provider::Scope;

/// How far back one scope's backfill reaches, and how far it has got.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Backfill {
    /// Fixed cutoff chosen when the account was first synced.
    pub since: Timestamp,
    /// Next page token of the `Since(since)` pass; `None` starts at the first page.
    pub recent_page: Option<String>,
    pub recent_done: bool,
    /// Next page token of the `Before(since)` pass, for load-more.
    pub older_page: Option<String>,
    pub older_done: bool,
}

impl Backfill {
    pub fn new(since: Timestamp) -> Self {
        Self { since, recent_page: None, recent_done: false, older_page: None, older_done: false }
    }
}

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};

use crate::model::{Account, Folder, FolderId, Message, MessageId, ProviderKind};
use crate::provider::{RemoteId, RemoteState};

/// `$MAIL_CLASSIFIER_MAIL_DB`, else
/// `~/Library/Application Support/mail-classifier/mail.db`.
pub fn default_cache_path() -> PathBuf {
    if let Ok(path) = std::env::var("MAIL_CLASSIFIER_MAIL_DB")
        && !path.trim().is_empty()
    {
        return PathBuf::from(path);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    home.join("Library/Application Support/mail-classifier/mail.db")
}

pub struct Cache {
    conn: Connection,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS accounts(
    id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    name TEXT NOT NULL,
    email TEXT NOT NULL,
    color TEXT NOT NULL,
    cursor TEXT
);
CREATE TABLE IF NOT EXISTS folders(
    id INTEGER PRIMARY KEY,
    account TEXT NOT NULL,
    remote_id TEXT,
    name TEXT NOT NULL,
    parent INTEGER
);
CREATE TABLE IF NOT EXISTS threads(
    id INTEGER PRIMARY KEY,
    account TEXT NOT NULL,
    remote_id TEXT NOT NULL,
    UNIQUE(account, remote_id)
);
CREATE TABLE IF NOT EXISTS messages(
    id INTEGER PRIMARY KEY,
    account TEXT NOT NULL,
    remote_id TEXT NOT NULL,
    json TEXT NOT NULL,
    remote_state TEXT NOT NULL,
    UNIQUE(account, remote_id)
);
CREATE TABLE IF NOT EXISTS backfill(
    account TEXT NOT NULL,
    scope TEXT NOT NULL,
    since INTEGER NOT NULL,
    recent_page TEXT,
    recent_done INTEGER NOT NULL,
    older_page TEXT,
    older_done INTEGER NOT NULL,
    PRIMARY KEY(account, scope)
);
";

impl Cache {
    /// Open (creating if needed) the cache at `path`, creating its directory.
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        }
        Self::connect(path)
    }

    /// A throwaway cache for tests.
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        Self::connect(Path::new(":memory:"))
    }

    fn connect(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(SCHEMA)?;
        // Caches created before read sync lack the column; their rows start unknown (NULL).
        let has_read: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('messages') WHERE name='remote_read'",
            [],
            |row| row.get::<_, i64>(0).map(|n| n > 0),
        )?;
        if !has_read {
            conn.execute_batch("ALTER TABLE messages ADD COLUMN remote_read INTEGER")?;
        }
        Ok(Self { conn })
    }

    /// Everything needed to rebuild a [`crate::model::Mailbox`].
    pub fn load(&self) -> rusqlite::Result<(Vec<Account>, Vec<Folder>, Vec<Message>)> {
        let mut stmt = self.conn.prepare(
            "SELECT id, provider, name, email, color FROM accounts ORDER BY rowid",
        )?;
        let accounts = stmt
            .query_map([], |row| {
                let provider: String = row.get(1)?;
                Ok(Account {
                    id: row.get(0)?,
                    name: row.get(2)?,
                    email: row.get(3)?,
                    color: row.get(4)?,
                    provider: serde_json::from_str(&provider).unwrap_or(ProviderKind::Mock),
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        let mut stmt = self
            .conn
            .prepare("SELECT id, account, name, parent FROM folders ORDER BY id")?;
        let folders = stmt
            .query_map([], |row| {
                Ok(Folder {
                    id: row.get::<_, i64>(0)? as FolderId,
                    account: row.get(1)?,
                    name: row.get(2)?,
                    parent: row.get::<_, Option<i64>>(3)?.map(|p| p as FolderId),
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        let mut stmt = self
            .conn
            .prepare("SELECT json FROM messages ORDER BY id")?;
        let messages = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .filter_map(|json| json.ok())
            .filter_map(|json| serde_json::from_str::<Message>(&json).ok())
            .collect();

        Ok((accounts, folders, messages))
    }

    pub fn upsert_account(&self, account: &Account) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO accounts(id, provider, name, email, color, cursor)
             VALUES(?1, ?2, ?3, ?4, ?5, NULL)
             ON CONFLICT(id) DO UPDATE SET
                provider=excluded.provider, name=excluded.name,
                email=excluded.email, color=excluded.color",
            params![
                account.id,
                serde_json::to_string(&account.provider).unwrap_or_else(|_| "\"Mock\"".into()),
                account.name,
                account.email,
                account.color,
            ],
        )?;
        Ok(())
    }

    pub fn set_cursor(&self, account: &str, cursor: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE accounts SET cursor=?2 WHERE id=?1",
            params![account, cursor],
        )?;
        Ok(())
    }

    pub fn cursor(&self, account: &str) -> rusqlite::Result<Option<String>> {
        let cursor = self
            .conn
            .query_row("SELECT cursor FROM accounts WHERE id=?1", [account], |row| {
                row.get::<_, Option<String>>(0)
            })
            .optional()?;
        Ok(cursor.flatten())
    }

    pub fn upsert_folder(&self, folder: &Folder, remote_id: Option<&str>) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO folders(id, account, remote_id, name, parent)
             VALUES(?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET
                account=excluded.account,
                remote_id=COALESCE(excluded.remote_id, folders.remote_id),
                name=excluded.name, parent=excluded.parent",
            params![
                folder.id as i64,
                folder.account,
                remote_id,
                folder.name,
                folder.parent.map(|p| p as i64),
            ],
        )?;
        Ok(())
    }

    pub fn folder_remote_id(&self, id: FolderId) -> rusqlite::Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT remote_id FROM folders WHERE id=?1", [id as i64], |row| {
                row.get::<_, Option<String>>(0)
            })
            .optional()?
            .flatten())
    }

    pub fn folder_by_remote(
        &self,
        account: &str,
        remote_id: &str,
    ) -> rusqlite::Result<Option<FolderId>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id FROM folders WHERE account=?1 AND remote_id=?2",
                params![account, remote_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .map(|id| id as FolderId))
    }

    /// The local thread id for a remote thread, allocating one via `alloc` the
    /// first time it is seen.
    pub fn thread_id(
        &self,
        account: &str,
        remote_thread: &str,
        alloc: impl FnOnce() -> u32,
    ) -> rusqlite::Result<u32> {
        let found = self
            .conn
            .query_row(
                "SELECT id FROM threads WHERE account=?1 AND remote_id=?2",
                params![account, remote_thread],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        if let Some(id) = found {
            return Ok(id as u32);
        }
        let id = alloc() as i64;
        self.conn.execute(
            "INSERT INTO threads(id, account, remote_id) VALUES(?1, ?2, ?3)",
            params![id, account, remote_thread],
        )?;
        Ok(id as u32)
    }

    pub fn message_by_remote(
        &self,
        account: &str,
        remote_id: &str,
    ) -> rusqlite::Result<Option<MessageId>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id FROM messages WHERE account=?1 AND remote_id=?2",
                params![account, remote_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .map(|id| id as MessageId))
    }

    /// Remote ids of every cached message of `account`.
    pub fn remote_ids(&self, account: &str) -> rusqlite::Result<std::collections::HashSet<String>> {
        let mut stmt = self.conn.prepare("SELECT remote_id FROM messages WHERE account=?1")?;
        let ids = stmt.query_map([account], |row| row.get(0))?;
        ids.collect()
    }

    pub fn upsert_message(
        &self,
        message: &Message,
        remote_id: &str,
        state: &RemoteState,
        remote_read: bool,
    ) -> rusqlite::Result<()> {
        let json = serde_json::to_string(message)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        self.conn.execute(
            "INSERT INTO messages(id, account, remote_id, json, remote_state, remote_read)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
                account=excluded.account, remote_id=excluded.remote_id,
                json=excluded.json, remote_state=excluded.remote_state,
                remote_read=excluded.remote_read",
            params![
                message.id as i64,
                message.account,
                remote_id,
                json,
                serde_json::to_string(state)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?,
                remote_read,
            ],
        )?;
        Ok(())
    }

    /// Account, remote id and last confirmed remote state of a cached message.
    pub fn remote_state(&self, id: MessageId) -> rusqlite::Result<Option<(String, RemoteId, RemoteState)>> {
        let row = self
            .conn
            .query_row(
                "SELECT account, remote_id, remote_state FROM messages WHERE id=?1",
                [id as i64],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;
        Ok(row.and_then(|(account, remote, state)| {
            Some((account, remote, serde_json::from_str(&state).ok()?))
        }))
    }

    pub fn set_remote_state(&self, id: MessageId, state: &RemoteState) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE messages SET remote_state=?2 WHERE id=?1",
            params![
                id as i64,
                serde_json::to_string(state)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?
            ],
        )?;
        Ok(())
    }

    /// Rewrite the stored message without touching the confirmed remote state.
    pub fn save_local(&self, message: &Message) -> rusqlite::Result<()> {
        let json = serde_json::to_string(message)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        self.conn.execute(
            "UPDATE messages SET json=?2 WHERE id=?1",
            params![message.id as i64, json],
        )?;
        Ok(())
    }

    /// Read state the server last confirmed; `None` for rows cached before read sync existed.
    pub fn remote_read(&self, id: MessageId) -> rusqlite::Result<Option<bool>> {
        Ok(self
            .conn
            .query_row("SELECT remote_read FROM messages WHERE id=?1", [id as i64], |row| {
                row.get::<_, Option<bool>>(0)
            })
            .optional()?
            .flatten())
    }

    pub fn set_remote_read(&self, id: MessageId, read: bool) -> rusqlite::Result<()> {
        self.conn
            .execute("UPDATE messages SET remote_read=?2 WHERE id=?1", params![id as i64, read])?;
        Ok(())
    }

    /// Up to `limit` cached messages of `account` whose server read state is unknown.
    pub fn unknown_read(&self, account: &str, limit: usize) -> rusqlite::Result<Vec<RemoteId>> {
        let mut stmt = self.conn.prepare(
            "SELECT remote_id FROM messages WHERE account=?1 AND remote_read IS NULL ORDER BY id LIMIT ?2",
        )?;
        let ids = stmt.query_map(params![account, limit as i64], |row| row.get(0))?;
        ids.collect()
    }

    /// Every cached message, by local id.
    pub fn cached_messages(&self) -> rusqlite::Result<Vec<Message>> {
        let mut stmt = self.conn.prepare("SELECT json FROM messages ORDER BY id")?;
        Ok(stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .filter_map(|json| json.ok())
            .filter_map(|json| serde_json::from_str::<Message>(&json).ok())
            .collect())
    }

    /// Cached folders of `account` with the remote label they map to, if any.
    pub fn folder_list(&self, account: &str) -> rusqlite::Result<Vec<CachedFolder>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, parent, remote_id FROM folders WHERE account=?1 ORDER BY id",
        )?;
        let rows = stmt.query_map([account], |row| {
            Ok((
                row.get::<_, i64>(0)? as FolderId,
                row.get::<_, String>(1)?,
                row.get::<_, Option<i64>>(2)?.map(|p| p as FolderId),
                row.get::<_, Option<String>>(3)?,
            ))
        })?;
        rows.collect()
    }

    /// Ids of every cached account.
    pub fn account_ids(&self) -> rusqlite::Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT id FROM accounts ORDER BY rowid")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        rows.collect()
    }

    pub fn has_account(&self, account: &str) -> rusqlite::Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM accounts WHERE id=?1",
            [account],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Backfill progress of one scope. An unseen scope starts at the first page
    /// with a zero cutoff, which the caller replaces before the first listing.
    pub fn backfill(&self, account: &str, scope: &Scope) -> rusqlite::Result<Backfill> {
        let row = self.conn.query_row(
            "SELECT since, recent_page, recent_done, older_page, older_done
             FROM backfill WHERE account=?1 AND scope=?2",
            params![account, scope_json(scope)],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, bool>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, bool>(4)?,
                ))
            },
        );
        let Ok((since, recent_page, recent_done, older_page, older_done)) = row else {
            return Ok(Backfill::new(0));
        };
        Ok(Backfill { since, recent_page, recent_done, older_page, older_done })
    }

    /// The cutoff every scope of `account` is measured against: the one its first
    /// backfill chose, else `now - WINDOW_DAYS` days.
    pub fn window_since(&self, account: &str, now: Timestamp) -> rusqlite::Result<Timestamp> {
        let since: Option<i64> = self.conn.query_row(
            "SELECT MIN(since) FROM backfill WHERE account=?1",
            [account],
            |row| row.get(0),
        )?;
        Ok(since.unwrap_or(now - crate::sync::WINDOW_DAYS * DAY))
    }

    pub fn save_backfill(
        &self,
        account: &str,
        scope: &Scope,
        state: &Backfill,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO backfill(account, scope, since, recent_page, recent_done, older_page, older_done)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(account, scope) DO UPDATE SET
                since=excluded.since, recent_page=excluded.recent_page,
                recent_done=excluded.recent_done, older_page=excluded.older_page,
                older_done=excluded.older_done",
            params![
                account,
                scope_json(scope),
                state.since,
                state.recent_page,
                state.recent_done,
                state.older_page,
                state.older_done,
            ],
        )?;
        Ok(())
    }

    /// Forget every scope's recent progress, so the next backfill re-lists the window.
    pub fn reset_recent(&self, account: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE backfill SET recent_page=NULL, recent_done=0 WHERE account=?1",
            [account],
        )?;
        Ok(())
    }

    /// Whether any cached row of `account` still has an unknown server read state
    /// (rows written before read sync existed).
    pub fn has_unknown_read(&self, account: &str) -> rusqlite::Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM messages WHERE account=?1 AND remote_read IS NULL",
            [account],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }


    pub fn delete_message(&self, id: MessageId) -> rusqlite::Result<()> {
        self.conn
            .execute("DELETE FROM messages WHERE id=?1", [id as i64])?;
        Ok(())
    }

    /// Drops the account and everything cached for it.
    pub fn delete_account(&self, account: &str) -> rusqlite::Result<()> {
        for table in ["messages", "folders", "threads", "backfill"] {
            self.conn
                .execute(&format!("DELETE FROM {table} WHERE account=?1"), [account])?;
        }
        self.conn.execute("DELETE FROM accounts WHERE id=?1", [account])?;
        Ok(())
    }

    /// Max cached message id plus one; 1 when empty.
    pub fn next_message_id(&self) -> rusqlite::Result<MessageId> {
        self.next_id("SELECT MAX(id) FROM messages")
    }

    /// Max cached folder id plus one; 1 when empty.
    pub fn next_folder_id(&self) -> rusqlite::Result<FolderId> {
        self.next_id("SELECT MAX(id) FROM folders")
    }

    /// Max cached thread id plus one; 1 when empty.
    pub fn next_thread_id(&self) -> rusqlite::Result<u32> {
        self.next_id("SELECT MAX(id) FROM threads")
    }

    fn next_id(&self, sql: &str) -> rusqlite::Result<u32> {
        let max = self
            .conn
            .query_row(sql, [], |row| row.get::<_, Option<i64>>(0))
            .optional()?
            .flatten();
        Ok(max.map_or(1, |m| (m + 1) as u32))
    }
}

/// A cached folder row: its local id, name, parent and remote label.
pub type CachedFolder = (FolderId, String, Option<FolderId>, Option<String>);

/// [`Scope`] as the JSON stored in the `backfill` table's key.
fn scope_json(scope: &Scope) -> String {
    serde_json::to_string(scope).unwrap_or_default()
}