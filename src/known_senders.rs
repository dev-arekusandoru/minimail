//! v1 toggle for the known/unknown sender ("Screener") distinction.
//!
//! When [`KNOWN_SENDERS_ENABLED`] is `false` the app does not distinguish known
//! from unknown senders: [`crate::model::Mailbox::is_new_sender`] is always
//! `false`, allowing a sender is a no-op, and the New Senders chip, the
//! New Sender filter, the icon legend row, the palette commands and the
//! "new sender" badge/banner never appear. Flip the const to `true` to restore
//! the feature; every call site gates on it.

/// Whether the known/unknown sender distinction is live. Off in v1.
pub const KNOWN_SENDERS_ENABLED: bool = false;
