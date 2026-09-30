//! Errors from the contact store.

use std::fmt;

/// Convenience alias for store operations.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Everything that can go wrong in [`super::ContactStore`].
#[derive(Debug)]
pub enum Error {
    /// The database rejected a statement.
    Sqlite(rusqlite::Error),
    /// An email address is already owned by another contact row.
    DuplicateEmail(String),
    /// A group name is already taken.
    DuplicateGroup(String),
    /// The referenced contact or group does not exist.
    NotFound(String),
    /// Empty or otherwise unusable input.
    Invalid(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Sqlite(e) => write!(f, "contact store: {e}"),
            Error::DuplicateEmail(a) => write!(f, "email `{a}` already belongs to a contact"),
            Error::DuplicateGroup(n) => write!(f, "group `{n}` already exists"),
            Error::NotFound(what) => write!(f, "{what} not found"),
            Error::Invalid(why) => write!(f, "invalid contact data: {why}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::Sqlite(e)
    }
}

pub(crate) fn invalid(why: impl Into<String>) -> Error {
    Error::Invalid(why.into())
}
