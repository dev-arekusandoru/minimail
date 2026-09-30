//! Schema creation and forward migrations.
//!
//! Each migration is one numbered SQL statement list; `PRAGMA user_version`
//! records how many have been applied, so opening an existing file only runs
//! what is missing.

use rusqlite::Connection;

use super::error::{Error, Result};

/// Applied in order; index + 1 is the `user_version` that migration sets.
const MIGRATIONS: &[&str] = &[MIGRATION_1];

/// Provider-shaped contacts: people, child tables for their addresses, and
/// groups for labels.
const MIGRATION_1: &str = r#"
CREATE TABLE contacts (
    id                INTEGER PRIMARY KEY,
    prefix            TEXT,
    given_name        TEXT,
    middle_name       TEXT,
    family_name       TEXT,
    suffix            TEXT,
    nickname          TEXT,
    display_name      TEXT NOT NULL DEFAULT '',
    organization      TEXT,
    department        TEXT,
    job_title         TEXT,
    birthday          TEXT,
    notes             TEXT,
    photo             TEXT,
    favorite          INTEGER NOT NULL DEFAULT 0,
    source            TEXT NOT NULL DEFAULT 'manual'
                      CHECK (source IN ('manual', 'screener', 'seed', 'import')),
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL,
    last_contacted_at INTEGER
);

-- Addresses are stored lowercased and unique across the whole table, so one
-- address belongs to exactly one contact.
CREATE TABLE contact_emails (
    id         INTEGER PRIMARY KEY,
    contact_id INTEGER NOT NULL REFERENCES contacts(id) ON DELETE CASCADE,
    address    TEXT NOT NULL UNIQUE,
    label      TEXT NOT NULL DEFAULT '',
    is_primary INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE contact_phones (
    id         INTEGER PRIMARY KEY,
    contact_id INTEGER NOT NULL REFERENCES contacts(id) ON DELETE CASCADE,
    number     TEXT NOT NULL,
    label      TEXT NOT NULL DEFAULT '',
    is_primary INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE contact_addresses (
    id          INTEGER PRIMARY KEY,
    contact_id  INTEGER NOT NULL REFERENCES contacts(id) ON DELETE CASCADE,
    street      TEXT NOT NULL DEFAULT '',
    city        TEXT NOT NULL DEFAULT '',
    region      TEXT NOT NULL DEFAULT '',
    postal_code TEXT NOT NULL DEFAULT '',
    country     TEXT NOT NULL DEFAULT '',
    label       TEXT NOT NULL DEFAULT ''
);

CREATE TABLE contact_urls (
    id         INTEGER PRIMARY KEY,
    contact_id INTEGER NOT NULL REFERENCES contacts(id) ON DELETE CASCADE,
    url        TEXT NOT NULL,
    label      TEXT NOT NULL DEFAULT ''
);

CREATE TABLE contact_groups (
    id   INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE COLLATE NOCASE
);

CREATE TABLE contact_group_members (
    contact_id INTEGER NOT NULL REFERENCES contacts(id) ON DELETE CASCADE,
    group_id   INTEGER NOT NULL REFERENCES contact_groups(id) ON DELETE CASCADE,
    PRIMARY KEY (contact_id, group_id)
) WITHOUT ROWID;

CREATE TABLE contact_fields (
    id         INTEGER PRIMARY KEY,
    contact_id INTEGER NOT NULL REFERENCES contacts(id) ON DELETE CASCADE,
    key        TEXT NOT NULL,
    value      TEXT NOT NULL
);

-- contact_emails.address is already UNIQUE (and so indexed); search also hits
-- display names and every child lookup goes through contact_id.
CREATE INDEX idx_contacts_display_name ON contacts(display_name);
CREATE INDEX idx_contact_emails_contact ON contact_emails(contact_id);
CREATE INDEX idx_contact_phones_contact ON contact_phones(contact_id);
CREATE INDEX idx_contact_addresses_contact ON contact_addresses(contact_id);
CREATE INDEX idx_contact_urls_contact ON contact_urls(contact_id);
CREATE INDEX idx_contact_fields_contact ON contact_fields(contact_id);
CREATE INDEX idx_group_members_group ON contact_group_members(group_id);

-- Store-level bookkeeping, e.g. which seed revision has been applied.
CREATE TABLE store_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#;

/// Turn on the pragmas the store relies on and apply missing migrations.
pub(crate) fn prepare(conn: &Connection) -> Result<()> {
    // A database file may be mid-recovery from a crash; WAL lets readers run
    // while a write is in flight, and foreign keys give us cascade deletes.
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let version: usize = version.max(0) as usize;
    if version > MIGRATIONS.len() {
        return Err(Error::Invalid(format!(
            "database was written by a newer version (schema {version}, this build knows {})",
            MIGRATIONS.len()
        )));
    }
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(version) {
        let target = (i + 1) as i64;
        // Migrations and the version bump share one transaction, so a failure
        // leaves the file at the previous version.
        conn.execute_batch("BEGIN")?;
        match (|| -> Result<()> {
            conn.execute_batch(sql)?;
            conn.pragma_update(None, "user_version", target)?;
            Ok(())
        })() {
            Ok(()) => conn.execute_batch("COMMIT")?,
            Err(e) => {
                conn.execute_batch("ROLLBACK")?;
                return Err(e);
            }
        }
    }
    Ok(())
}
