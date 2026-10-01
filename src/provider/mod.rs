//! Provider-agnostic mail backend interface. No GPUI, no model ids: providers
//! speak in remote ids only; the sync layer maps to local ids.

use serde::{Deserialize, Serialize};

use crate::clock::Timestamp;
use crate::model::Attachment;

pub mod gmail;
pub mod secrets;

pub type RemoteId = String;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemoteState {
    Inbox,
    Archived,
    Folder(RemoteId),
    Trash,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RemoteFolder {
    pub id: RemoteId,
    /// Path segments joined by '/'.
    pub path: String,
}

/// Server-side flags of a message, derivable from its labels alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteFlags {
    pub state: RemoteState,
    pub unread: bool,
}

/// A listable mailbox view. Archive = not inbox/trash/spam/draft and no user label.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Scope {
    Inbox,
    Archive,
    Trash,
    Folder(RemoteId),
}

/// Date bound for a listing: `Since(t)` = received at/after `t`, `Before(t)` = strictly before `t`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Window {
    Since(Timestamp),
    Before(Timestamp),
}

/// One page of a listing, newest first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Page {
    pub ids: Vec<RemoteId>,
    pub next: Option<String>,
}

/// The full content of a message, downloaded when it is opened.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Body {
    pub body: String,
    pub html: Option<String>,
    pub attachments: Vec<Attachment>,
}

/// Headers, snippet and flags of a message; the body comes from [`MailProvider::fetch_body`].
#[derive(Clone, Debug, PartialEq)]
pub struct RemoteMessage {
    pub id: RemoteId,
    pub thread: RemoteId,
    pub from_name: String,
    pub from_email: String,
    pub to: String,
    pub cc: String,
    pub bcc: String,
    pub subject: String,
    /// Server snippet (HTML entities decoded); stands in for the body until it is downloaded.
    pub snippet: String,
    pub received: Timestamp,
    pub state: RemoteState,
    pub outgoing: bool,
    pub unread: bool,
}

/// Changes since a cursor. An id appears at most once per list, and ids in
/// `removed` never appear in `added` or `updated`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    /// New to the mailbox; need [`MailProvider::fetch_headers`] if not cached.
    pub added: Vec<RemoteId>,
    /// Current flags of messages whose labels changed (last change wins).
    pub updated: Vec<(RemoteId, RemoteFlags)>,
    pub removed: Vec<RemoteId>,
    pub cursor: String,
}

#[derive(Debug)]
pub enum ProviderError {
    Auth(String),
    CursorExpired,
    Network(String),
    /// Still throttled after the provider's own retries; try again later.
    RateLimited,
    Api { status: u16, message: String },
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Auth(m) => write!(f, "authentication failed: {m}"),
            Self::CursorExpired => write!(f, "sync cursor expired"),
            Self::Network(m) => write!(f, "network error: {m}"),
            Self::RateLimited => write!(f, "server is busy; retrying shortly"),
            Self::Api { status, message } => write!(f, "server error {status}: {message}"),
        }
    }
}

impl std::error::Error for ProviderError {}

pub trait MailProvider: Send + 'static {
    fn folders(&mut self) -> Result<Vec<RemoteFolder>, ProviderError>;
    fn create_folder(&mut self, path: &str) -> Result<RemoteFolder, ProviderError>;
    /// Current change cursor. Take it *before* listing so nothing slips between.
    fn cursor(&mut self) -> Result<String, ProviderError>;
    /// One page (at most `max` ids, newest first) of `scope` within `window`.
    /// `page` is a token from a previous [`Page::next`].
    fn list(
        &mut self,
        scope: &Scope,
        window: Window,
        page: Option<&str>,
        max: usize,
    ) -> Result<Page, ProviderError>;
    /// Flags of every message received at/after `since`, across all scopes, for
    /// reconciling after an expired cursor. Built from listings only.
    fn snapshot(&mut self, since: Timestamp) -> Result<Vec<(RemoteId, RemoteFlags)>, ProviderError>;
    /// [`ProviderError::CursorExpired`] when `cursor` is too old.
    fn changes(&mut self, cursor: &str) -> Result<Changes, ProviderError>;
    /// Headers, snippet and flags, in the order of `ids`. Unknown or filtered ids
    /// (spam, drafts, chats, deleted) are absent from the result.
    fn fetch_headers(&mut self, ids: &[RemoteId]) -> Result<Vec<RemoteMessage>, ProviderError>;
    fn fetch_body(&mut self, id: &RemoteId) -> Result<Body, ProviderError>;
    fn move_message(
        &mut self,
        id: &RemoteId,
        from: &RemoteState,
        to: &RemoteState,
    ) -> Result<(), ProviderError>;
    /// Mark the message read (`true`) or unread on the server.
    fn set_read(&mut self, id: &RemoteId, read: bool) -> Result<(), ProviderError>;
}
