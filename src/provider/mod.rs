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
    pub body: String,
    pub html: Option<String>,
    pub received: Timestamp,
    pub attachments: Vec<Attachment>,
    pub state: RemoteState,
    pub outgoing: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Changes {
    pub changed: Vec<RemoteId>,
    pub removed: Vec<RemoteId>,
    pub cursor: String,
}

#[derive(Debug)]
pub enum ProviderError {
    Auth(String),
    CursorExpired,
    Network(String),
    /// The server is throttling us; retry later.
    RateLimited,
    Api { status: u16, message: String },
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Auth(m) => write!(f, "authentication failed: {m}"),
            Self::CursorExpired => write!(f, "sync cursor expired"),
            Self::Network(m) => write!(f, "network error: {m}"),
            Self::RateLimited => write!(f, "rate limit reached; pausing for a minute"),
            Self::Api { status, message } => write!(f, "server error {status}: {message}"),
        }
    }
}

impl std::error::Error for ProviderError {}

pub trait MailProvider: Send + 'static {
    fn folders(&mut self) -> Result<Vec<RemoteFolder>, ProviderError>;
    fn create_folder(&mut self, path: &str) -> Result<RemoteFolder, ProviderError>;
    /// Newest `limit` message ids plus a change cursor taken *before* listing.
    fn recent(&mut self, limit: usize) -> Result<(Vec<RemoteId>, String), ProviderError>;
    fn changes(&mut self, cursor: &str) -> Result<Changes, ProviderError>;
    /// Unknown/filtered ids (spam, drafts, chats) are simply absent from the result.
    fn fetch(&mut self, ids: &[RemoteId]) -> Result<Vec<RemoteMessage>, ProviderError>;
    fn move_message(
        &mut self,
        id: &RemoteId,
        from: &RemoteState,
        to: &RemoteState,
    ) -> Result<(), ProviderError>;
}
