//! The composer domain: Markdown source and preview, rich HTML import and
//! clipboard conversion, durable local drafts, managed attachments, the send
//! hold and the MIME message that goes out.
//!
//! Pure and blocking, with `now: Timestamp` injected everywhere: no GPUI, no
//! clock, no provider. `store` is owned by the background worker; the UI reads
//! snapshots of these records.

pub mod attachment;
pub mod body;
pub mod draft;
pub mod hold;
pub mod html;
pub mod markdown;
pub mod mime;
pub mod schedule;
pub mod store;

pub use attachment::{
    AttachmentId, AttachmentSource, AttachmentStore, ManagedAttachment, declared_content_type,
    keep_hashes, valid_content_id,
};
pub use body::{
    BodyFormat, BodyMode, Clipboard, ConversionApproval, ConversionWarning, DraftBody,
    HtmlConversion, ImportedOriginal, PasteResult, RenderedBody,
};
pub use draft::{DEFAULT_REMIND_DAYS, DraftId, DraftKind, DraftRecord, DraftState, ReplyContext, SendIntent, split_addresses};
pub use hold::{
    HOLD_SECONDS, HoldAction, HoldProgress, HoldRegistry, HoldTransition, PendingHold,
    Reconciliation, SendOutcome, SentLookup, hold_progress, hold_remaining, reconcile_uncertain,
};
pub use html::{html_text, html_to_markdown};
pub use mime::{
    OutgoingMessage, build_mime, encode_header_value, format_address_list, rfc2822_date,
};
pub use schedule::{BadTime, parse_send_later};
pub use store::{DraftError, DraftStore, Recovery};
