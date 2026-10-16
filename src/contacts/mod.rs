//! SQLite-backed contact store.
//!
//! [`ContactStore`] owns one connection. Plain structs go in and out; the
//! store normalizes addresses (lowercased, unique across the table), keeps
//! timestamps in unix seconds like [`crate::clock::Timestamp`], and turns
//! constraint violations into typed errors.
//!
//! ```no_run
//! use mail_classifier::contacts::{ContactStore, NewContact};
//!
//! let store = ContactStore::open_in_memory().unwrap();
//! let contact = store.create(NewContact::new("Ada")).unwrap();
//! assert!(store.get(contact.id).unwrap().is_some());
//! ```

mod children;
mod error;
#[cfg(feature = "fixtures")]
mod fixture;
mod groups;
mod judgements;
mod model;
mod read;
mod schema;
mod settings;
mod write;

use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::clock::{Clock, SystemClock, Timestamp};
use rusqlite::Connection;

pub use error::{Error, Result};
#[cfg(feature = "fixtures")]
pub use fixture::{open_seeded_in_memory, seed_if_empty};
pub use model::{
    Contact, ContactQuery, ContactSource, ContactUrl, EmailAddress, Field, Group, Label,
    NewContact, PhoneNumber, PostalAddress,
};

/// `$MAIL_CLASSIFIER_DB`, else `~/Library/Application Support/mail-classifier/contacts.db`.
pub fn default_db_path() -> PathBuf {
    if let Ok(path) = std::env::var("MAIL_CLASSIFIER_DB")
        && !path.trim().is_empty()
    {
        return PathBuf::from(path);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    home.join("Library/Application Support/mail-classifier/contacts.db")
}

/// Open the database the app uses, creating the directory. Starts empty.
pub fn open_default() -> Result<ContactStore> {
    let path = default_db_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| Error::Invalid(format!("cannot create {}: {e}", dir.display())))?;
    }
    ContactStore::open(&path)
}

/// A connection to the contacts database.
pub struct ContactStore {
    conn: Connection,
    clock: Rc<dyn Clock>,
}

impl ContactStore {
    /// Open (creating if needed) the database at `path` and migrate it.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::with_clock(path, Rc::new(SystemClock))
    }

    /// A private in-memory database, for tests and scratch work.
    pub fn open_in_memory() -> Result<Self> {
        Self::with_clock(Path::new(":memory:"), Rc::new(SystemClock))
    }

    fn with_clock(path: impl AsRef<Path>, clock: Rc<dyn Clock>) -> Result<Self> {
        let conn = Connection::open(path.as_ref())?;
        schema::prepare(&conn)?;
        Ok(Self { conn, clock })
    }

    /// Swap the time source; tests drive it with [`crate::clock::FakeClock`].
    pub fn set_clock(&mut self, clock: Rc<dyn Clock>) {
        self.clock = clock;
    }

    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }

    pub(crate) fn now(&self) -> Timestamp {
        self.clock.now()
    }

    /// `PRAGMA user_version`: how many migrations this database has applied.
    pub fn schema_version(&self) -> Result<i64> {
        Ok(self.conn.query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }

    /// `PRAGMA journal_mode`, e.g. `wal`.
    pub fn journal_mode(&self) -> Result<String> {
        Ok(self.conn.query_row("PRAGMA journal_mode", [], |r| r.get(0))?)
    }

    /// Whether cascades are enforced.
    pub fn foreign_keys_enabled(&self) -> Result<bool> {
        Ok(self.conn.query_row("PRAGMA foreign_keys", [], |r| r.get::<_, i64>(0))? == 1)
    }
}

/// Lowercase an address and reject the unusable ones.
pub fn normalize_email(email: &str) -> Result<String> {
    let address = email.trim().to_ascii_lowercase();
    let bad = || error::invalid(format!("`{email}` is not an email address"));
    let Some((local, domain)) = address.split_once('@') else {
        return Err(bad());
    };
    if local.is_empty() || domain.is_empty() || address.chars().any(char::is_whitespace) {
        return Err(bad());
    }
    Ok(address)
}
