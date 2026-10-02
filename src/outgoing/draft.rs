//! The durable draft record: recipients, body, threading, managed
//! attachments, provider sync state and the send state it is in.

use std::collections::HashSet;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::clock::Timestamp;
use crate::model::{AccountId, Message};
use crate::provider::RemoteId;
use crate::reading::{Recipient, parse_recipients};

use super::attachment::{AttachmentId, ManagedAttachment};
use super::body::DraftBody;
use super::store::DraftError;

/// Local draft identity. `0` means "not persisted yet".
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
pub struct DraftId(pub i64);

impl fmt::Display for DraftId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DraftKind {
    New,
    Reply,
    ReplyAll,
    Forward,
}

impl DraftKind {
    pub fn label(self) -> &'static str {
        match self {
            DraftKind::New => "New message",
            DraftKind::Reply => "Reply",
            DraftKind::ReplyAll => "Reply all",
            DraftKind::Forward => "Forward",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DraftState {
    /// Editable, possibly with unsaved changes in the UI.
    Editing,
    /// Waiting out the send hold; undo is still available.
    Held,
    /// A send request is in flight (persisted before it was issued).
    Sending,
    Sent {
        remote_id: RemoteId,
    },
    /// The request may or may not have been delivered: reconcile before retry.
    Uncertain,
    Failed {
        error: String,
    },
    /// The provider's draft moved on underneath us; the local copy is kept.
    Conflict,
}

impl DraftState {
    pub fn is_editable(&self) -> bool {
        matches!(
            self,
            DraftState::Editing
                | DraftState::Held
                | DraftState::Failed { .. }
                | DraftState::Conflict
        )
    }

    /// True while a send could still be in flight, so a restart must not treat
    /// the draft as an ordinary retryable one.
    pub fn is_in_flight(&self) -> bool {
        matches!(self, DraftState::Sending | DraftState::Uncertain)
    }
}

/// Threading metadata of the message being answered, as the provider reports
/// it (RFC Message-IDs, not remote ids).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReplyContext {
    pub message_id: String,
    pub references: Vec<String>,
    pub subject: String,
    pub thread: Option<RemoteId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DraftRecord {
    pub id: DraftId,
    pub account: AccountId,
    pub kind: DraftKind,
    /// Comma-separated recipient fields, like `Message::to`.
    pub to: String,
    pub cc: String,
    pub bcc: String,
    pub subject: String,
    pub body: DraftBody,
    /// RFC `Message-ID` of the message this one answers, if any.
    #[serde(default)]
    pub in_reply_to: Option<String>,
    /// RFC `References` chain, oldest first.
    #[serde(default)]
    pub references: Vec<String>,
    #[serde(default)]
    pub thread_remote_id: Option<RemoteId>,
    /// The provider's draft, once one exists.
    #[serde(default)]
    pub remote_id: Option<RemoteId>,
    /// The provider's concurrency token for that draft.
    #[serde(default)]
    pub remote_revision: Option<String>,
    /// RFC `Message-ID` of this draft's send attempt, generated once and kept
    /// across saves, restarts and retries so a lost response can be reconciled.
    pub message_id: String,
    /// The subject this draft was prefilled with, when it came from a message.
    /// A user edit of it means the draft no longer answers that thread.
    #[serde(default)]
    pub prefilled_subject: Option<String>,
    /// The message this draft came from, so the UI can file the reply under it
    /// once the send is confirmed. Purely local: the provider only ever sees
    /// the RFC ids above.
    #[serde(default)]
    pub source_message: Option<crate::model::MessageId>,
    /// The managed identities an original import produced, when this draft was
    /// built from a message's raw content. A local baseline for "is this still
    /// exactly what was imported": a name alone cannot tell a replaced file
    /// from the original one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imported_attachment_ids: Option<Vec<AttachmentId>>,
    /// Whether a send request was ever issued for this attempt.
    #[serde(default)]
    pub attempted: bool,
    #[serde(default)]
    pub attachments: Vec<ManagedAttachment>,
    /// Bumped on every persisted change; a save with a stale value is refused.
    pub revision: u64,
    /// Bumped whenever the UI-editable fields change, so a local save can be
    /// matched against the editor it came from without being refused by
    /// unrelated worker writes.
    #[serde(default)]
    pub editor_revision: u64,
    /// Bumped whenever the payload a remote draft mirrors changes. `0` on a
    /// fresh draft; equal to `synced_content_revision` when the provider holds
    /// exactly this content.
    #[serde(default)]
    pub content_revision: u64,
    /// The `content_revision` of the exact snapshot last uploaded successfully,
    /// `None` while nothing of this draft has been mirrored (or after it moved
    /// account, which drops the binding).
    #[serde(default)]
    pub synced_content_revision: Option<u64>,
    /// Set by [`DraftRecord::note_mirrored`] while a write is being assembled,
    /// and never persisted: the store resolves it to the content revision the
    /// finished record actually has.
    #[serde(default, skip)]
    pub(crate) mirrored_now: bool,
    pub updated_at: Timestamp,
    pub state: DraftState,
}

impl DraftRecord {
    /// A blank new message. `now` only stamps the record.
    pub fn new_email(account: impl Into<AccountId>, now: Timestamp) -> Self {
        let account = account.into();
        Self {
            id: DraftId(0),
            message_id: new_message_id(&account, now),
            account,
            kind: DraftKind::New,
            to: String::new(),
            cc: String::new(),
            bcc: String::new(),
            subject: String::new(),
            body: DraftBody::markdown(""),
            prefilled_subject: None,
            source_message: None,
            imported_attachment_ids: None,
            in_reply_to: None,
            references: Vec::new(),
            thread_remote_id: None,
            remote_id: None,
            remote_revision: None,
            attempted: false,
            attachments: Vec::new(),
            revision: 0,
            editor_revision: 0,
            content_revision: 0,
            synced_content_revision: None,
            mirrored_now: false,
            updated_at: now,
            state: DraftState::Editing,
        }
    }

    /// A draft prefilled from a message. The body is a reply quote (or the
    /// forwarded block); attachments of a forward are fetched and imported by
    /// the caller, since only the provider has their bytes.
    pub fn prefilled(kind: DraftKind, msg: &Message, own_email: &str, now: Timestamp) -> Self {
        use crate::draft::{DraftKind as LegacyKind, draft};
        let legacy = match kind {
            DraftKind::New => return Self::new_email(msg.account.clone(), now),
            DraftKind::Reply => LegacyKind::Reply,
            DraftKind::ReplyAll => LegacyKind::ReplyAll,
            DraftKind::Forward => LegacyKind::Forward,
        };
        let prefilled = draft(legacy, msg, own_email);
        let mut record = Self::new_email(msg.account.clone(), now);
        record.kind = kind;
        record.to = prefilled.to;
        record.cc = prefilled.cc;
        record.subject = prefilled.subject;
        record.body = DraftBody::markdown(prefilled.body);
        record.prefilled_subject = Some(record.subject.clone());
        record.source_message = Some(msg.id);
        record
    }

    /// Record the threading of the original this draft answers.
    pub fn set_threading(&mut self, ctx: &ReplyContext) -> &mut Self {
        if !ctx.message_id.trim().is_empty() {
            self.in_reply_to = Some(ctx.message_id.trim().to_owned());
        }
        self.references = ctx.references.clone();
        if self.kind == DraftKind::New {
            self.kind = DraftKind::Reply;
        }
        if self.subject.trim().is_empty() && !ctx.subject.trim().is_empty() {
            self.subject = ctx.subject.clone();
        }
        if self.thread_remote_id.is_none() {
            self.thread_remote_id = ctx.thread.clone();
        }
        if self.prefilled_subject.is_none() && !ctx.subject.trim().is_empty() {
            self.prefilled_subject = Some(ctx.subject.clone());
        }
        self
    }

    /// Record an edited subject. A reply whose subject the user changed is no
    /// longer a reply as far as threading goes, so the reply headers and the
    /// thread hint are dropped. Returns whether they were dropped.
    pub fn set_subject(&mut self, subject: &str) -> bool {
        let changed = self
            .prefilled_subject
            .as_deref()
            .is_some_and(|original| original.trim() != subject.trim());
        self.subject = subject.to_owned();
        if changed {
            self.clear_threading();
            return true;
        }
        false
    }

    /// Drop the reply threading, for a reply the user turned into a forward:
    /// the new message must not claim to answer the original.
    pub fn clear_threading(&mut self) -> &mut Self {
        self.in_reply_to = None;
        self.references.clear();
        self.thread_remote_id = None;
        self
    }

    /// Every recipient of this draft, in To, Cc, Bcc order.
    pub fn recipients(&self) -> Vec<Recipient> {
        [&self.to, &self.cc, &self.bcc]
            .into_iter()
            .flat_map(|field| parse_recipients(field))
            .collect()
    }

    /// Reject a draft that cannot be sent, and de-duplicate its recipients
    /// case-insensitively so nobody is mailed twice.
    pub fn validate(&self) -> Result<Vec<Recipient>, String> {
        let unique = self.validate_recipient_fields()?;
        if unique.is_empty() {
            return Err("No recipients".to_owned());
        }
        Ok(unique)
    }

    /// Check what is typed without demanding a recipient yet: a draft the user
    /// has not addressed still has to save, sync and render, but whatever is
    /// there must be a real address. Returns the recipients, de-duplicated.
    pub fn validate_recipient_fields(&self) -> Result<Vec<Recipient>, String> {
        let mut seen = HashSet::new();
        let mut unique = Vec::new();
        for (field, value) in [("To", &self.to), ("Cc", &self.cc), ("Bcc", &self.bcc)] {
            for recipient in parse_recipients(value) {
                if !valid_email(&recipient.email) {
                    return Err(format!(
                        "{field}: {:?} is not an email address",
                        recipient.email
                    ));
                }
                if seen.insert(recipient.email.to_ascii_lowercase()) {
                    unique.push(recipient);
                }
            }
        }
        Ok(unique)
    }

    /// Refuse to touch a provider draft that moved on: the caller keeps the
    /// local copy and shows the conflict instead of overwriting.
    pub fn ensure_remote_current(&self, remote_revision: &str) -> Result<(), DraftError> {
        match &self.remote_revision {
            Some(current) if current != remote_revision => Err(DraftError::RemoteConflict {
                local: current.clone(),
                remote: remote_revision.to_owned(),
            }),
            _ => Ok(()),
        }
    }

    /// Add a managed copy. Returns false when the draft already has it.
    pub fn attach(&mut self, attachment: ManagedAttachment) -> bool {
        if self.attachments.iter().any(|a| a.id == attachment.id) {
            return false;
        }
        self.attachments.push(attachment);
        true
    }

    pub fn detach(&mut self, id: &AttachmentId) -> bool {
        let before = self.attachments.len();
        self.attachments.retain(|a| &a.id != id);
        self.attachments.len() != before
    }

    /// Record the parts the current import produced, so a later raw send can
    /// tell whether the draft still carries exactly those bytes.
    pub fn note_imported_attachments(&mut self) -> &mut Self {
        let mut ids: Vec<AttachmentId> = self.attachments.iter().map(|a| a.id.clone()).collect();
        ids.sort();
        self.imported_attachment_ids = Some(ids);
        self
    }

    /// True only when the draft still carries exactly the parts the original
    /// import produced. The identities carry the content hash, name, declared
    /// media type and inline id, so replacing a file with different bytes under
    /// the same name — or changing only its declared type — reads as touched.
    pub fn imported_parts_untouched(&self) -> bool {
        let Some(baseline) = &self.imported_attachment_ids else {
            return false;
        };
        if self.attachments.len() != baseline.len() {
            return false;
        }
        if self.attachments.iter().map(|a| &a.id).eq(baseline.iter()) {
            return true;
        }
        let mut have: Vec<&AttachmentId> = self.attachments.iter().map(|a| &a.id).collect();
        have.sort_unstable();
        have.into_iter().eq(baseline.iter())
    }

    /// Whether the fields a UI editor owns differ from `previous`. Body mode is
    /// included: which side of the source/preview toggle shows is an edit, even
    /// though it changes no content.
    pub fn editor_changed_since(&self, previous: &Self) -> bool {
        self.account != previous.account
            || self.kind != previous.kind
            || self.to != previous.to
            || self.cc != previous.cc
            || self.bcc != previous.bcc
            || self.subject != previous.subject
            || self.body != previous.body
    }

    /// Whether the payload a remote draft mirrors differs from `previous`: the
    /// editable text, the attachments and the RFC reply headers. A body *mode*
    /// change is a view, not content, so it does not count here.
    pub fn content_changed_since(&self, previous: &Self) -> bool {
        self.account != previous.account
            || self.kind != previous.kind
            || self.to != previous.to
            || self.cc != previous.cc
            || self.bcc != previous.bcc
            || self.subject != previous.subject
            || self.body.source != previous.body.source
            || self.body.format != previous.body.format
            || self.body.original != previous.body.original
            || self.body.approved != previous.body.approved
            || self.attachments != previous.attachments
            || self.in_reply_to != previous.in_reply_to
            || self.references != previous.references
    }

    /// Recompute the counters for a write that turns `previous` into `self`.
    /// The incoming values are never trusted: only an actual difference moves a
    /// counter, so an unrelated write cannot make a save look like an edit.
    pub fn note_change_since(&mut self, previous: &Self) {
        self.editor_revision = if self.editor_changed_since(previous) {
            previous.editor_revision + 1
        } else {
            previous.editor_revision
        };
        self.content_revision = if self.content_changed_since(previous) {
            previous.content_revision + 1
        } else {
            previous.content_revision
        };
    }
    /// Claim that the provider now holds exactly this content. The number is
    /// resolved when the write lands, so the caller does not have to predict
    /// the content revision its own edit produces.
    pub fn note_mirrored(&mut self) -> &mut Self {
        self.mirrored_now = true;
        self
    }

    pub fn touch(&mut self, now: Timestamp) {
        self.updated_at = now;
    }

    /// A new send attempt reuses the same Message-ID, so reconciliation after a
    /// crash finds the message the first attempt would have sent.
    pub fn note_send_attempt(&mut self) {
        self.attempted = true;
    }
}

/// A fresh RFC Message-ID (without angle brackets) for a draft of `account`.
pub fn new_message_id(account: &str, now: Timestamp) -> String {
    use rand::Rng as _;
    let domain = account.rsplit_once('@').map_or("localhost", |(_, d)| d);
    let entropy: u64 = rand::thread_rng().r#gen();
    format!("{now:x}.{entropy:016x}@{domain}")
}

/// Split a recipient field into the addresses typed in it.
pub fn split_addresses(input: &str) -> Vec<String> {
    parse_recipients(input)
        .into_iter()
        .map(|r| match r.name {
            Some(name) if !name.trim().is_empty() => format!("{name} <{}>", r.email),
            _ => r.email,
        })
        .collect()
}

/// Shared address policy for sending and recipient chip feedback.
pub fn valid_email(email: &str) -> bool {
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    !local.trim().is_empty()
        && !domain.trim().is_empty()
        && !email.chars().any(char::is_whitespace)
        && domain.contains('.')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Message;

    fn message() -> Message {
        Message {
            id: 7,
            thread_id: 3,
            from_name: "Dana".to_owned(),
            from_email: "dana@x.test".to_owned(),
            to: "me@x.test, sam@x.test".to_owned(),
            cc: "lee@x.test".to_owned(),
            bcc: String::new(),
            subject: "Lunch".to_owned(),
            body: "body".to_owned(),
            received: "2026-01-01T00:00:00Z".to_owned(),
            state: crate::model::TriageState::default(),
            account: "personal".to_owned(),
            outgoing: false,
            snooze: None,
            html: None,
            attachments: Vec::new(),
            read: true,
            partial: false,
        }
    }

    #[test]
    fn new_email_has_a_stable_message_id() {
        let draft = DraftRecord::new_email("me@x.test", 1_700_000_000);
        assert_eq!(draft.kind, DraftKind::New);
        assert_eq!(draft.state, DraftState::Editing);
        assert!(
            draft.message_id.ends_with("@x.test"),
            "{}",
            draft.message_id
        );
        let mut same = draft.clone();
        same.touch(1_800_000_000);
        assert_eq!(
            same.message_id, draft.message_id,
            "saving must not regenerate it"
        );
    }

    #[test]
    fn reply_prefill_points_at_the_original_thread() {
        let mut draft = DraftRecord::prefilled(DraftKind::Reply, &message(), "me@x.test", 10);
        assert!(draft.to.contains("dana@x.test"), "{}", draft.to);
        assert!(!draft.to.contains("me@x.test"), "{}", draft.to);
        assert_eq!(draft.subject, "Re: Lunch");
        // The reader composes the quote under the caret; the prefill leaves the
        // body empty so the caret starts on a clean line.
        assert!(draft.body.source.is_empty(), "{}", draft.body.source);

        draft.set_threading(&ReplyContext {
            message_id: "orig@x.test".to_owned(),
            references: vec!["older@x.test".to_owned()],
            subject: "Lunch".to_owned(),
            thread: Some("t1".to_owned()),
        });
        assert_eq!(draft.in_reply_to.as_deref(), Some("orig@x.test"));
        assert_eq!(draft.references, vec!["older@x.test".to_owned()]);
        assert_eq!(draft.thread_remote_id.as_deref(), Some("t1"));
    }

    #[test]
    fn reply_all_drops_the_own_address() {
        let draft = DraftRecord::prefilled(DraftKind::ReplyAll, &message(), "me@x.test", 10);
        assert!(draft.to.contains("dana@x.test"), "{}", draft.to);
        assert!(draft.to.contains("sam@x.test"), "{}", draft.to);
        assert!(!draft.to.contains("me@x.test"), "{}", draft.to);
        assert_eq!(draft.cc, "lee@x.test");
    }

    #[test]
    fn forward_prefills_the_block_and_leaves_attachments_to_the_provider() {
        let draft = DraftRecord::prefilled(DraftKind::Forward, &message(), "me@x.test", 10);
        assert!(draft.body.source.contains("Lunch"), "{}", draft.body.source);
        assert!(draft.attachments.is_empty());
    }

    #[test]
    fn validation_needs_a_recipient_and_a_sane_address() {
        let mut draft = DraftRecord::new_email("me@x.test", 1);
        assert!(draft.validate().is_err());

        draft.to = "sam@x.test, not-an-address".to_owned();
        let err = draft.validate().expect_err("invalid address");
        assert!(err.contains("To"), "{err}");

        draft.to = "Sam <sam@x.test>, sam@x.test, lee@x.test".to_owned();
        draft.cc = "SAM@x.test".to_owned();
        let recipients = draft.validate().expect("valid");
        let emails: Vec<&str> = recipients.iter().map(|r| r.email.as_str()).collect();
        assert_eq!(emails, vec!["sam@x.test", "lee@x.test"]);
    }

    #[test]
    fn bcc_alone_is_a_valid_recipient() {
        let mut draft = DraftRecord::new_email("me@x.test", 1);
        draft.bcc = "hidden@x.test".to_owned();
        assert_eq!(draft.validate().expect("valid").len(), 1);
    }

    #[test]
    fn split_addresses_round_trips_names() {
        let parts = split_addresses("\"Doe, Jane\" <jane@x.test>, plain@x.test");
        assert_eq!(parts, vec!["Doe, Jane <jane@x.test>", "plain@x.test"]);
    }

    #[test]
    fn remote_conflict_is_refused_without_touching_local_state() {
        let mut draft = DraftRecord::new_email("me@x.test", 1);
        draft.remote_id = Some("d1".to_owned());
        draft.remote_revision = Some("rev-1".to_owned());
        assert!(draft.ensure_remote_current("rev-1").is_ok());
        let err = draft.ensure_remote_current("rev-2").expect_err("conflict");
        match err {
            DraftError::RemoteConflict { local, remote } => {
                assert_eq!(local, "rev-1");
                assert_eq!(remote, "rev-2");
            }
            other => panic!("expected a remote conflict, got {other}"),
        }
    }

    #[test]
    fn editing_the_subject_of_a_reply_drops_its_threading() {
        let mut draft = DraftRecord::prefilled(DraftKind::Reply, &message(), "me@x.test", 10);
        draft.set_threading(&ReplyContext {
            message_id: "orig@x.test".to_owned(),
            references: vec!["older@x.test".to_owned()],
            subject: "Lunch".to_owned(),
            thread: Some("t1".to_owned()),
        });
        assert!(
            !draft.set_subject("Re: Lunch"),
            "an untouched subject keeps the thread"
        );
        assert_eq!(draft.in_reply_to.as_deref(), Some("orig@x.test"));

        assert!(
            draft.set_subject("Fwd: Lunch"),
            "a new subject is not a reply any more"
        );
        assert_eq!(draft.in_reply_to, None);
        assert_eq!(draft.thread_remote_id, None);
        assert_eq!(draft.subject, "Fwd: Lunch");
    }

    #[test]
    fn turning_a_reply_into_a_forward_drops_the_threading() {
        let mut draft = DraftRecord::prefilled(DraftKind::Reply, &message(), "me@x.test", 10);
        draft.set_threading(&ReplyContext {
            message_id: "orig@x.test".to_owned(),
            references: vec!["older@x.test".to_owned()],
            subject: "Lunch".to_owned(),
            thread: Some("t1".to_owned()),
        });
        draft.clear_threading();
        assert_eq!(draft.in_reply_to, None);
        assert!(draft.references.is_empty());
        assert_eq!(draft.thread_remote_id, None);
    }

    #[test]
    fn attachments_are_added_once_and_removed_by_id() {
        let mut draft = DraftRecord::new_email("me@x.test", 1);
        let attachment = managed("a.txt", b"a");
        assert!(draft.attach(attachment.clone()));
        assert!(!draft.attach(attachment.clone()));
        assert_eq!(draft.attachments.len(), 1);
        assert!(draft.detach(&attachment.id));
        assert!(!draft.detach(&attachment.id));
    }

    fn managed(name: &str, bytes: &[u8]) -> ManagedAttachment {
        let dir = std::env::temp_dir().join(format!("mail-classifier-draft-{name}"));
        let store = super::super::attachment::AttachmentStore::open(&dir).expect("store");
        store
            .import_bytes(
                name,
                bytes,
                super::super::attachment::AttachmentSource::Generated,
                None,
            )
            .expect("import")
    }

    #[test]
    fn body_and_state_helpers_behave() {
        let mut draft = DraftRecord::new_email("me@x.test", 1);
        assert!(draft.state.is_editable());
        assert!(!draft.state.is_in_flight());
        draft.state = DraftState::Sending;
        assert!(!draft.state.is_editable());
        assert!(draft.state.is_in_flight());
        draft.state = DraftState::Sent {
            remote_id: "m1".to_owned(),
        };
        assert!(!draft.state.is_in_flight());
        draft.state = DraftState::Editing;
        draft.body.set_source("hi");
        assert_eq!(draft.body.source, "hi");
    }

    #[test]
    fn a_draft_knows_which_message_it_came_from() {
        let original = message();
        let reply = DraftRecord::prefilled(DraftKind::Reply, &original, "me@x.test", 10);
        assert_eq!(
            reply.source_message,
            Some(original.id),
            "the reply knows its original"
        );
        let forward = DraftRecord::prefilled(DraftKind::Forward, &original, "me@x.test", 10);
        assert_eq!(
            forward.source_message,
            Some(original.id),
            "the UI can still offer it"
        );
        assert_eq!(DraftRecord::new_email("me@x.test", 10).source_message, None);
    }

    #[test]
    fn an_imported_baseline_notices_a_swapped_file() {
        let mut draft = DraftRecord::new_email("me@x.test", 1);
        assert!(!draft.imported_parts_untouched(), "nothing recorded yet");

        let original = managed("report.txt", b"original bytes");
        draft.attach(original.clone());
        draft.note_imported_attachments();
        assert!(
            draft.imported_parts_untouched(),
            "the draft still holds the import"
        );

        // Same file name, different bytes: the identity carries the hash, so
        // this cannot pass as the untouched original.
        let swapped = managed("report.txt", b"different bytes");
        assert_ne!(swapped.id, original.id);
        draft.detach(&original.id);
        draft.attach(swapped);
        assert!(
            !draft.imported_parts_untouched(),
            "a replaced file is not the original"
        );

        // Adding or removing a part is a change too.
        let mut added = DraftRecord::new_email("me@x.test", 1);
        let a = managed("a.txt", b"a");
        added.attach(a.clone());
        added.note_imported_attachments();
        added.attach(managed("extra.txt", b"extra"));
        assert!(
            !added.imported_parts_untouched(),
            "an added part is a change"
        );
        added.detach(&a.id);
        assert!(
            !added.imported_parts_untouched(),
            "a removed part is a change"
        );
    }
}
