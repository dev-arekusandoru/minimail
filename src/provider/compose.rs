//! Provider-neutral composing seam: remote drafts, sending and attachment bytes.
//!
//! Separate from [`MailProvider`](super::MailProvider) on purpose. Reading the
//! mailbox is something every backend can do; keeping drafts in sync, sending
//! without duplicating mail, and handing back attachment bytes are Gmail-shaped
//! operations that a weaker backend must be allowed to say "not supported"
//! about instead of lying through an empty list.
//!
//! Everything here speaks remote ids only. The local draft store owns
//! durability, Markdown and MIME assembly; a provider only uploads bytes it is
//! handed, and hands back the ids and revisions the caller must remember.
//!
//! Endpoint semantics implemented by the Gmail adapter
//! (<https://developers.google.com/workspace/gmail/api/guides/drafts>):
//!
//! * a draft is a container with a stable id; replacing its content mints a new
//!   underlying message id, so the message id is the optimistic-concurrency
//!   token used by [`ComposeProvider::update_draft`];
//! * `drafts.send` deletes the draft and creates a new message labelled
//!   `SENT`, returning it — so a send is only "sent" once that message comes
//!   back;
//! * an interrupted send is not knowable from the failed request, so it is
//!   reported as [`SendStatus::Uncertain`] and reconciled with
//!   [`ComposeProvider::find_sent`] by RFC 5322 `Message-ID`. Never retried
//!   blind.

use crate::clock::Timestamp;
use crate::model::AccountId;

use super::{ProviderError, RemoteId};

/// Opaque optimistic-concurrency token for a remote draft. Whatever the
/// provider can compare cheaply and that changes when the draft is replaced
/// (for Gmail: the underlying message id).
pub type RemoteRevision = String;

/// A draft as the provider currently holds it, with the headers a draft list
/// needs. `message`/`thread` are ids the caller may store; `revision` must be
/// echoed back on update so a remotely changed draft is never silently
/// overwritten.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteDraft {
    pub id: RemoteId,
    /// Underlying message id; replaced whenever the draft content is replaced,
    /// which is what makes it a usable conflict token.
    pub message: RemoteId,
    pub thread: RemoteId,
    /// Opaque token to echo back on update; [`ComposeError::Conflict`] when it
    /// no longer matches.
    pub revision: RemoteRevision,
    /// RFC 5322 `Message-ID` of the stored message, without angle brackets.
    ///
    /// This is what a lost send response is reconciled by, so a caller that
    /// falls back to the provider's own bytes must reuse *this* id rather than
    /// mint a new one: sending stored MIME unchanged sends the id inside it, and
    /// a `find_sent` lookup under a different id can never match.
    /// `None` when the resource carried no headers — a create or update
    /// response, whose body was not read.
    pub message_id: Option<String>,
    pub to: String,
    pub cc: String,
    pub bcc: String,
    pub subject: String,
    pub body: MessageBody,
    /// Server snippet; a stand-in when the body was not fetched.
    pub snippet: String,
    /// Whether this draft carries files. Only meaningful when
    /// [`Self::attachments_known`] is true: a listing carries no payload, and
    /// "we did not look" is not "there are none".
    pub has_attachments: bool,
    /// True when the payload was fetched and `attachments` is complete.
    pub attachments_known: bool,
    pub updated: Timestamp,
    /// Files carried by the draft, with inline bytes when the provider has
    /// them. Populated by [`ComposeProvider::get_draft`].
    pub attachments: Vec<RemoteAttachment>,
}

/// One page of [`ComposeProvider::list_drafts`], newest first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DraftPage {
    pub drafts: Vec<RemoteDraft>,
    pub next: Option<String>,
}
/// RFC 5322 threading headers of a remote message, needed to build a
/// well-formed reply: `In-Reply-To` is [`Threading::message_id`] and
/// `References` is the chain plus that id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Threading {
    /// `Message-ID` value, without angle brackets.
    pub message_id: String,
    /// `References` values, oldest first, without angle brackets.
    pub references: Vec<String>,
    pub subject: String,
}

/// Parsed body of a draft or message: the text the caller can edit and the
/// HTML it must preserve verbatim until the user approves conversion.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MessageBody {
    pub text: String,
    pub html: Option<String>,
}

/// A file carried by a remote message, with everything needed to fetch or
/// reuse its bytes. `inline_data` is set when the provider already returned
/// the content inline (small parts); otherwise `id` identifies the provider's
/// attachment resource.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteAttachment {
    /// Identifies this part within its message. Two parts of one message may
    /// share a name, so this — not the name — is what addresses the bytes.
    pub id: Option<RemoteId>,
    /// The part's MIME `Content-ID` without angle brackets. Inline images the
    /// body refers to with `cid:` are addressed by this and nothing else, so a
    /// rebuilt body can point at the original part instead of dropping the
    /// image or inlining a second copy of it.
    pub content_id: Option<String>,
    pub name: String,
    pub size: u64,
    pub content_type: String,
    pub inline_data: Option<Vec<u8>>,
}

/// Bytes of one attachment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttachmentBytes {
    pub bytes: Vec<u8>,
    pub size: u64,
    pub content_type: String,
}

/// A message ready to upload. The MIME is assembled by the caller so exactly
/// the HTML shown in preview is what goes out; the threading fields are the
/// RFC 5322 values that were written into those headers, kept here because the
/// provider needs them to reconcile a send whose response was lost.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutgoingMail {
    /// Complete RFC 2822 message, CRLF line endings.
    pub rfc822: Vec<u8>,
    /// RFC 5322 `Message-ID` value without angle brackets.
    pub message_id: String,
    /// `In-Reply-To` value without angle brackets, for a reply.
    pub in_reply_to: Option<String>,
    /// `References` values without angle brackets, oldest first.
    pub references: Vec<String>,
    /// Thread this message should join, for backends that thread explicitly.
    /// Clear it when the user edits the subject: a backend that threads by
    /// subject will not join the thread anyway, and one told to would misfile
    /// the message. A provider may also ignore it for a message that is not a
    /// reply.
    pub thread: Option<RemoteId>,
    /// Account the mail is sent from, for multi-account backends.
    pub account: AccountId,
}

/// The message a confirmed send produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SentMail {
    pub id: RemoteId,
    pub thread: RemoteId,
    /// RFC 5322 `Message-ID` of the sent message, without angle brackets.
    pub message_id: String,
    pub sent_at: Timestamp,
}

/// Outcome of a send attempt.
///
/// `Uncertain` is not an error to retry: the request may already have been
/// applied, so the caller must reconcile with
/// [`ComposeProvider::find_sent`] before deciding anything.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SendStatus {
    Sent(SentMail),
    Uncertain { detail: String },
}

#[derive(Debug)]
pub enum ComposeError {
    /// The provider rejected the request, or it failed in a way that proves
    /// nothing was applied (auth, throttling, validation).
    Provider(ProviderError),
    /// The draft moved on underneath us: `expected` is the revision the caller
    /// last saw, `actual` the one the provider holds now. The caller's copy is
    /// kept; nothing was written.
    Conflict {
        draft: RemoteId,
        expected: RemoteRevision,
        actual: RemoteRevision,
    },
    /// The request may or may not have been applied. Reconcile, do not resend.
    Uncertain { detail: String },
    /// No such draft or message on the server.
    NotFound(RemoteId),
    /// The request provably never left the machine, so nothing may have been
    /// applied. Safe to try again later, and nothing to reconcile.
    Offline,
    /// An interrupted draft save or delete. The remote draft may exist in a
    /// state the caller has not seen — a duplicate, or a replacement this
    /// client no longer knows about — but no mail was sent, so this is never
    /// reconciled with [`ComposeProvider::find_sent`]. Re-list instead of
    /// writing again.
    AutosaveUncertain { detail: String },
}

impl From<ProviderError> for ComposeError {
    fn from(e: ProviderError) -> Self {
        Self::Provider(e)
    }
}

impl std::fmt::Display for ComposeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Provider(e) => e.fmt(f),
            Self::Conflict {
                draft,
                expected,
                actual,
            } => write!(
                f,
                "draft {draft} changed on the server (had {expected}, now {actual})"
            ),
            Self::Uncertain { detail } => {
                write!(f, "the server may or may not have accepted it: {detail}")
            }
            Self::NotFound(id) => write!(f, "no such draft: {id}"),
            Self::Offline => write!(f, "offline"),
            Self::AutosaveUncertain { detail } => {
                write!(f, "the draft save may not have landed: {detail}")
            }
        }
    }
}

impl std::error::Error for ComposeError {}

impl ComposeError {
    /// True when nothing may have been applied and the same request can be
    /// tried again later.
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Offline => true,
            Self::Provider(e) => {
                matches!(e, ProviderError::RateLimited | ProviderError::CursorExpired)
            }
            _ => false,
        }
    }

    /// True when the provider cannot say whether a write landed, whatever the
    /// write was. Use [`Self::needs_send_reconciliation`] to ask the narrower
    /// question that matters after a send.
    pub fn is_uncertain(&self) -> bool {
        self.needs_send_reconciliation() || matches!(self, Self::AutosaveUncertain { .. })
    }

    /// True when a *send* may or may not have reached recipients, and must be
    /// settled with [`ComposeProvider::find_sent`] before anything is retried.
    /// An interrupted draft save is uncertain but never this, so a failed
    /// autosave can never be shown to the user as "may have sent".
    pub fn needs_send_reconciliation(&self) -> bool {
        match self {
            Self::Uncertain { .. } => true,
            Self::Provider(ProviderError::Network(_)) => true,
            Self::Provider(ProviderError::Api { status, .. }) => *status >= 500,
            _ => false,
        }
    }
}

pub trait ComposeProvider: Send + 'static {
    /// One page (at most `max`) of remote drafts, newest first.
    fn list_drafts(&mut self, page: Option<&str>, max: usize) -> Result<DraftPage, ComposeError>;
    /// Headers and revision of one draft. [`ComposeError::NotFound`] when the
    /// draft is gone (it may have been sent or deleted elsewhere).
    fn get_draft(&mut self, id: &RemoteId) -> Result<RemoteDraft, ComposeError>;
    /// The draft's stored RFC 2822 message, verbatim, so rich formatting the
    /// caller cannot represent survives a round trip.
    fn draft_raw(&mut self, id: &RemoteId) -> Result<Vec<u8>, ComposeError>;
    /// Uploads `mail` as a new draft. On an interrupted request the returned
    /// error is [`ComposeError::Uncertain`]: a draft may exist that the caller
    /// has not seen, so re-listing is required before creating another.
    fn create_draft(&mut self, mail: &OutgoingMail) -> Result<RemoteDraft, ComposeError>;
    /// Replaces the content of `draft`, but only if the provider still holds
    /// `draft.revision`; otherwise [`ComposeError::Conflict`] and no write.
    /// `mail` is threaded onto `mail.thread`/`mail.references` rather than onto
    /// the replaced draft's thread, so a reply keeps its original parent.
    fn update_draft(
        &mut self,
        draft: &RemoteDraft,
        mail: &OutgoingMail,
    ) -> Result<RemoteDraft, ComposeError>;
    /// Deletes a draft. Already gone counts as deleted.
    fn delete_draft(&mut self, id: &RemoteId) -> Result<(), ComposeError>;
    /// Sends the draft as it stands, replacing its content with `mail` first
    /// when given.
    fn send_draft(
        &mut self,
        draft: &RemoteDraft,
        mail: Option<&OutgoingMail>,
    ) -> Result<SendStatus, ComposeError>;
    /// Looks for a sent message with this RFC 5322 `Message-ID`. The
    /// reconciliation step for [`SendStatus::Uncertain`].
    fn find_sent(&mut self, message_id: &str) -> Result<Option<SentMail>, ComposeError>;
    /// Files carried by exactly this message, with inline bytes where the
    /// provider has them. `message` is a message id, never a thread: a thread
    /// holds many messages, and a part of one is not a part of another.
    fn list_attachments(
        &mut self,
        message: &RemoteId,
    ) -> Result<Vec<RemoteAttachment>, ComposeError>;
    /// Bytes of one attachment of exactly `message`. Resolved by the part's
    /// `id`, or taken straight from `inline_data` when the caller already has
    /// them — two parts of one message may share a file name, so the name is
    /// never a lookup key.
    fn attachment_bytes(
        &mut self,
        message: &RemoteId,
        attachment: &RemoteAttachment,
    ) -> Result<AttachmentBytes, ComposeError>;
    /// RFC 5322 `Message-ID`/`References` of a remote message, so a reply is
    /// threaded onto the original instead of starting a new thread.
    fn threading(&mut self, message: &RemoteId) -> Result<Threading, ComposeError>;
}
