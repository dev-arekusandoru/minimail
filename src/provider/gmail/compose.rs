//! Gmail implementation of [`ComposeProvider`].
//!
//! Endpoints and semantics per
//! <https://developers.google.com/workspace/gmail/api/guides/drafts>,
//! <https://developers.google.com/workspace/gmail/api/reference/rest/v1/users.drafts>
//! and <https://developers.google.com/workspace/gmail/api/guides/threads>:
//!
//! | operation        | call                                             |
//! |------------------|--------------------------------------------------|
//! | list             | `GET  /users/me/drafts`                          |
//! | get              | `GET  /users/me/drafts/{id}?format=full`         |
//! | raw              | `GET  /users/me/drafts/{id}?format=raw`          |
//! | create           | `POST /users/me/drafts`                          |
//! | update           | `PUT  /users/me/drafts/{id}`                     |
//! | delete           | `DELETE /users/me/drafts/{id}`                   |
//! | send             | `POST /users/me/drafts/send` (draft id in body)  |
//! | find sent        | `GET  /users/me/messages?q=rfc822msgid:…`        |
//! | list attachments | `GET  /users/me/messages/{id}?format=full`       |
//! | attachment bytes | `GET  /users/me/messages/{id}/attachments/{aid}` |
//! | threading        | `GET  /users/me/messages/{id}?format=metadata`   |
//!
//! `drafts.send` has no path parameter: the draft to send is named by `id` in
//! the request body.
//!
//! Threading follows the threads guide, which requires all three of a requested
//! `threadId` in `drafts.message`, RFC 2822 `References`/`In-Reply-To`, and a
//! matching `Subject`. This adapter supplies the first, from
//! `OutgoingMail.thread`, and only for a message that really declares
//! `In-Reply-To`. That guard stops a fresh message being forced into an
//! unrelated conversation; it cannot see a subject the user edited, so the
//! caller must clear the hint when the subject changes — a mismatched subject
//! is exactly what makes Gmail decline to thread.
//!
//! Concurrency: neither `drafts.update` nor `drafts.send` documents an
//! `If-Match` or any revision precondition, so a conflict cannot be detected
//! atomically. What the API does guarantee is that a draft's underlying message
//! id is *replaced* every time its content is replaced, including a
//! replacement made in the Gmail web client. So every update and every send
//! re-reads the draft first and compares that message id against the revision
//! the caller last saw; a mismatch returns [`ComposeError::Conflict`] with the
//! remote snapshot and writes nothing. That matters most when sending a draft
//! whose content the caller never supplied: sending the remote copy blind
//! would put mail the user never saw in front of people. An edit landing
//! between the read and the write is still possible and cannot be closed with
//! this API — every check here is read-then-write, not compare-and-swap. The
//! window is one round trip on a draft only the user is editing.
//!
//! Sends are never retried. `send_draft` issues exactly one `drafts.send`; only
//! a throttled request or a rejected access token is re-issued, and neither
//! means the server acted. A transport failure or 5xx afterwards is
//! [`ComposeError::Uncertain`], reconciled by RFC 5322 `Message-ID` with
//! [`ComposeProvider::find_sent`]. That ambiguity is specific to a *send*: an
//! interrupted autosave is [`ComposeError::AutosaveUncertain`] instead, because
//! nothing left the user's mailbox and saying "may have sent" would be a lie.

use super::super::RemoteId;
use serde_json::Value;

use super::super::compose::{
    AttachmentBytes, ComposeError, ComposeProvider, DraftPage, MessageBody, OutgoingMail,
    RemoteAttachment, RemoteDraft, SendStatus, SentMail, Threading,
};
use super::{BASE, GmailProvider, METADATA_HEADERS, ProviderError, RequestBody, drafts, url_with};

const DRAFT_PAGE: usize = 500;
/// `drafts.list`.
const COST_DRAFT_LIST: u32 = 5;
/// `drafts.create`.
const COST_DRAFT_CREATE: u32 = 10;
/// `drafts.update`.
const COST_DRAFT_UPDATE: u32 = 15;
/// `drafts.delete`.
const COST_DRAFT_DELETE: u32 = 10;
/// `drafts.send`.
const COST_DRAFT_SEND: u32 = 100;
/// `drafts.get`.
const COST_DRAFT_GET: u32 = 20;
/// `messages.get`.
const COST_MESSAGE_GET: u32 = 20;
/// `messages.list`.
const COST_MESSAGE_LIST: u32 = 5;
/// `messages.attachments.get`.
const COST_ATTACHMENT_GET: u32 = 20;

/// The RFC 5322 headers a reply needs, and nothing else. `In-Reply-To` is
/// among them because a message that omitted `References` still names its
/// parent there, and a reply built without it starts a new conversation.
const THREADING_HEADERS: [&str; 4] = ["Message-ID", "References", "In-Reply-To", "Subject"];

/// True when an error proves the server never acted on the request. A 429 or a
/// 401 was rejected before any work; anything else may have been applied.
fn definitely_unapplied(e: &ProviderError) -> bool {
    match e {
        ProviderError::Auth(_) | ProviderError::RateLimited | ProviderError::CursorExpired => true,
        ProviderError::Api { status, .. } => *status < 500,
        ProviderError::Network(_) => false,
    }
}

/// Turns a failed *send* into the honest verdict: either it never happened, or
/// it may have reached people and must be reconciled by `Message-ID` rather
/// than repeated.
fn send_error(action: &str, e: ProviderError) -> ComposeError {
    if definitely_unapplied(&e) {
        ComposeError::Provider(e)
    } else {
        ComposeError::Uncertain {
            detail: format!("{action}: {e}"),
        }
    }
}

/// Which part of a send attempt failed. A missing draft means different things
/// at different moments, and the difference decides whether the caller may try
/// again or has to reconcile first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendPhase {
    /// The read that checks the draft is still the one the caller saw.
    Preflight,
    /// The `drafts.send` request itself.
    Dispatch,
}

/// Classifies a failed send attempt.
///
/// A 404 here is never [`ComposeError::NotFound`]. It means the draft is gone,
/// and during a send the likeliest reason is that it was just sent from
/// another client — this very draft, to these very recipients. Reporting a
/// missing draft would let the caller clear its attempt and send again, which
/// is how one compose becomes two copies in people's inboxes. It is
/// [`ComposeError::Uncertain`] instead, so the caller reconciles by RFC 5322
/// `Message-ID` with [`ComposeProvider::find_sent`] before deciding anything.
///
/// This applies to sends only: a 404 from `get_draft` or `delete_draft` is a
/// plain missing draft, because no mail can be involved.
pub fn classify_send_failure(phase: SendPhase, error: ProviderError) -> ComposeError {
    let missing = matches!(&error, ProviderError::Api { status: 404, .. });
    if missing {
        let detail = match phase {
            SendPhase::Preflight => {
                format!(
                    "reading the draft before sending: {error}; it may already have been sent elsewhere"
                )
            }
            SendPhase::Dispatch => {
                format!("sending: {error}; the draft may already have been sent elsewhere")
            }
        };
        return ComposeError::Uncertain { detail };
    }
    send_error("sending", error)
}

/// Turns a failed draft save into the honest verdict. Nothing was mailed, so
/// this never claims a delivery might have happened: the caller re-lists
/// instead of writing again, in case a draft now exists it has not seen.
fn save_error(action: &str, e: ProviderError) -> ComposeError {
    if definitely_unapplied(&e) {
        ComposeError::Provider(e)
    } else {
        ComposeError::AutosaveUncertain {
            detail: format!("{action}: {e}"),
        }
    }
}

/// The most a create or update response can tell us on its own: enough to
/// address the draft and to refuse a later stale write, with the body and
/// headers left to be fetched.
fn identity_draft(id: &str, message: &str, thread: &str) -> RemoteDraft {
    RemoteDraft {
        id: id.to_owned(),
        message: message.to_owned(),
        thread: thread.to_owned(),
        revision: message.to_owned(),
        // The response carried ids, not headers, so this draft cannot say what
        // `Message-ID` its stored bytes would have. The caller must not send
        // those bytes unchanged until it has read the draft.
        message_id: None,
        to: String::new(),
        cc: String::new(),
        bcc: String::new(),
        subject: String::new(),
        body: MessageBody::default(),
        snippet: String::new(),
        has_attachments: false,
        attachments_known: false,
        updated: 0,
        attachments: Vec::new(),
    }
}

impl GmailProvider {
    /// Fetches one draft resource.
    fn draft_resource(&mut self, id: &str, format: &str) -> Result<Value, ComposeError> {
        self.call_once(
            COST_DRAFT_GET,
            "GET",
            &format!("{BASE}/drafts/{id}?format={format}"),
            None,
        )
        .map_err(|e| match e {
            ProviderError::Api { status: 404, .. } => ComposeError::NotFound(id.to_owned()),
            other => ComposeError::Provider(other),
        })
    }

    /// Message metadata carrying exactly `headers`, so a reply can be threaded
    /// without downloading a body.
    fn message_metadata(&mut self, id: &str, headers: &[&str]) -> Result<Value, ComposeError> {
        let query: String = headers
            .iter()
            .map(|h| format!("&metadataHeaders={h}"))
            .collect();
        self.call_once(
            COST_MESSAGE_GET,
            "GET",
            &format!("{BASE}/messages/{id}?format=metadata{query}"),
            None,
        )
        .map_err(|e| match e {
            ProviderError::Api { status: 404, .. } => ComposeError::NotFound(id.to_owned()),
            other => ComposeError::Provider(other),
        })
    }
}

impl ComposeProvider for GmailProvider {
    fn list_drafts(&mut self, page: Option<&str>, max: usize) -> Result<DraftPage, ComposeError> {
        let mut params = vec![("maxResults", max.clamp(1, DRAFT_PAGE).to_string())];
        if let Some(token) = page {
            params.push(("pageToken", token.to_owned()));
        }
        let url = url_with(&format!("{BASE}/drafts"), &params)?;
        let json = self
            .call_once(COST_DRAFT_LIST, "GET", &url, None)
            .map_err(ComposeError::Provider)?;
        let listed = drafts::draft_page(&json);
        let mut out = DraftPage {
            drafts: Vec::with_capacity(listed.drafts.len()),
            next: listed.next,
        };
        for entry in listed.drafts {
            // A listed draft resource carries only ids, so the headers a draft
            // list shows come from its message.
            let message = match self.message_metadata(&entry.message, &METADATA_HEADERS) {
                Ok(m) => m,
                // A draft deleted between the list and the fetch is not in this
                // page any more.
                Err(ComposeError::NotFound(_)) => continue,
                Err(e) => return Err(e),
            };
            let Some(mut full) = drafts::draft(&serde_json::json!({
                "id": entry.id,
                "message": message,
            })) else {
                continue;
            };
            // The list is the authority on which drafts exist; the message is
            // the authority on their headers.
            full.id = entry.id;
            full.message = entry.message;
            out.drafts.push(full);
        }
        Ok(out)
    }

    fn get_draft(&mut self, id: &RemoteId) -> Result<RemoteDraft, ComposeError> {
        let json = self.draft_resource(id, "full")?;
        drafts::draft(&json).ok_or_else(|| ComposeError::NotFound(id.clone()))
    }

    fn draft_raw(&mut self, id: &RemoteId) -> Result<Vec<u8>, ComposeError> {
        let json = self.draft_resource(id, "raw")?;
        drafts::draft_raw(&json).ok_or_else(|| ComposeError::NotFound(id.clone()))
    }

    fn create_draft(&mut self, mail: &OutgoingMail) -> Result<RemoteDraft, ComposeError> {
        let json = self
            .execute_once(
                COST_DRAFT_CREATE,
                "POST",
                &format!("{BASE}/drafts"),
                RequestBody::Json(drafts::raw_request(&mail.rfc822, mail.thread.as_deref())),
            )
            .map_err(|e| save_error("creating the draft", e))?
            .json;
        // A create response carries the ids but no content. Keeping them is what
        // stops a failed follow-up read from inviting a second create, which
        // would leave the user with two drafts.
        let identity = drafts::draft_identity(&json).filter(|(_, message, _)| !message.is_empty());
        let Some((id, message, thread)) = identity else {
            // The draft may exist, but without its message id the next write
            // cannot present a revision and would be refused forever. Re-list.
            return Err(ComposeError::AutosaveUncertain {
                detail: "draft creation returned no usable identity".into(),
            });
        };
        // The read is for the body and headers only; failing it loses nothing
        // the caller cannot fetch again by id.
        Ok(self
            .get_draft(&id)
            .unwrap_or_else(|_| identity_draft(&id, &message, &thread)))
    }

    fn update_draft(
        &mut self,
        draft: &RemoteDraft,
        mail: &OutgoingMail,
    ) -> Result<RemoteDraft, ComposeError> {
        // Read-before-write: the draft id is stable, the message id is not.
        let current = self.draft_resource(&draft.id, "full")?;
        let actual = current
            .get("message")
            .and_then(|m| m.get("id"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if actual != draft.message {
            return Err(ComposeError::Conflict {
                draft: draft.id.clone(),
                expected: draft.revision.clone(),
                actual,
            });
        }
        let json = self
            .execute_once(
                COST_DRAFT_UPDATE,
                "PUT",
                &format!("{BASE}/drafts/{}", draft.id),
                RequestBody::Json(drafts::raw_request(&mail.rfc822, mail.thread.as_deref())),
            )
            .map_err(|e| save_error("replacing the draft", e))?
            .json;
        // The replacement mints a new underlying message id, and that id is the
        // revision the next write has to present. Without it the draft exists
        // but cannot be written again until it is re-read, so say so instead of
        // handing back a revision that would never match.
        let identity = drafts::draft_identity(&json).filter(|(_, message, _)| !message.is_empty());
        let Some((id, message, thread)) = identity else {
            return Err(ComposeError::AutosaveUncertain {
                detail: "draft replacement returned no usable identity".into(),
            });
        };
        // Same reasoning as a create: the replacement already happened and its
        // new revision is known from the response, so a failed read must not be
        // reported as a failed update.
        Ok(self
            .get_draft(&id)
            .unwrap_or_else(|_| identity_draft(&id, &message, &thread)))
    }

    fn delete_draft(&mut self, id: &RemoteId) -> Result<(), ComposeError> {
        match self.execute_once(
            COST_DRAFT_DELETE,
            "DELETE",
            &format!("{BASE}/drafts/{id}"),
            RequestBody::Delete,
        ) {
            Ok(_) => Ok(()),
            // Already gone is the state the caller asked for.
            Err(ProviderError::Api { status: 404, .. }) => Ok(()),
            Err(e) => Err(save_error("deleting the draft", e)),
        }
    }

    fn send_draft(
        &mut self,
        draft: &RemoteDraft,
        mail: Option<&OutgoingMail>,
    ) -> Result<SendStatus, ComposeError> {
        // Read-before-write, for the same reason an update is: `mail = None`
        // sends whatever the server holds, so a draft edited elsewhere since
        // the caller last saw it must be a conflict, never mail the user has
        // not read. The check is one round trip and not atomic; the residual
        // window cannot be closed with this API.
        // A 404 here is routed through the same classifier as the send
        // itself: the draft may have been sent from another client in the
        // moments between the caller's snapshot and this call.
        // Read directly rather than through `draft_resource`, because this
        // read's failures are classified differently: during a send a missing
        // draft is a delivery question, not a missing object.
        let current = self
            .call_once(
                COST_DRAFT_GET,
                "GET",
                &format!("{BASE}/drafts/{}?format=metadata", draft.id),
                None,
            )
            .map_err(|e| classify_send_failure(SendPhase::Preflight, e))?;
        let actual = current
            .get("message")
            .and_then(|m| m.get("id"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if actual != draft.message {
            return Err(ComposeError::Conflict {
                draft: draft.id.clone(),
                expected: draft.revision.clone(),
                actual,
            });
        }
        // `drafts.send` takes no path parameter: the draft is named in the body.
        let body = drafts::send_request(
            &draft.id,
            mail.map(|m| (m.rfc822.as_slice(), m.thread.as_deref())),
        );
        // Never repeated: one send call, whatever comes back.
        let raw = self
            .execute_once(
                COST_DRAFT_SEND,
                "POST",
                &format!("{BASE}/drafts/send"),
                RequestBody::Json(body),
            )
            .map_err(|e| classify_send_failure(SendPhase::Dispatch, e))?;
        Ok(drafts::send_status(&raw.json))
    }

    fn find_sent(&mut self, message_id: &str) -> Result<Option<SentMail>, ComposeError> {
        if message_id.is_empty() {
            return Ok(None);
        }
        let url = url_with(
            &format!("{BASE}/messages"),
            &[
                ("q", drafts::sent_query(message_id)),
                ("labelIds", "SENT".to_owned()),
                ("maxResults", "10".to_owned()),
            ],
        )?;
        let list = self
            .call_once(COST_MESSAGE_LIST, "GET", &url, None)
            .map_err(ComposeError::Provider)?;
        for id in super::convert::list_ids(&list) {
            // The search operator is a convenience; the message's own
            // `Message-ID` header is the proof. A hit without it is some other
            // mail and must not be reported as this send.
            let json = self.message_metadata(&id, &THREADING_HEADERS)?;
            if let Some(found) = drafts::reconcile_sent(&json, message_id) {
                return Ok(Some(found));
            }
        }
        Ok(None)
    }

    fn list_attachments(
        &mut self,
        message: &RemoteId,
    ) -> Result<Vec<RemoteAttachment>, ComposeError> {
        // `format=metadata` has no payload to walk, so the full message is the
        // only way to see which files a message carries at all.
        let full = self
            .call_once(
                COST_MESSAGE_GET,
                "GET",
                &format!("{BASE}/messages/{message}?format=full"),
                None,
            )
            .map_err(|e| match e {
                ProviderError::Api { status: 404, .. } => ComposeError::NotFound(message.clone()),
                other => ComposeError::Provider(other),
            })?;
        Ok(drafts::attachments(&full))
    }

    fn attachment_bytes(
        &mut self,
        message: &RemoteId,
        attachment: &RemoteAttachment,
    ) -> Result<AttachmentBytes, ComposeError> {
        if let Some(bytes) = &attachment.inline_data {
            return Ok(AttachmentBytes {
                // The bytes that came back are the length, not the size the
                // listing estimated for the part.
                size: bytes.len() as u64,
                bytes: bytes.clone(),
                // The fetch response carries no type at all, so what the
                // listing declared is the authority on this part — including
                // when it declared none.
                content_type: drafts::media_type(&attachment.content_type).to_owned(),
            });
        }
        let Some(id) = &attachment.id else {
            return Err(ComposeError::NotFound(attachment.name.clone()));
        };
        let json = self
            .call_once(
                COST_ATTACHMENT_GET,
                "GET",
                &format!("{BASE}/messages/{message}/attachments/{id}"),
                None,
            )
            .map_err(|e| match e {
                ProviderError::Api { status: 404, .. } => ComposeError::NotFound(id.clone()),
                other => ComposeError::Provider(other),
            })?;
        let bytes = drafts::attachment_data(&json).ok_or_else(|| ComposeError::Uncertain {
            detail: format!("attachment {id} carried no data"),
        })?;
        Ok(AttachmentBytes {
            size: bytes.len() as u64,
            bytes,
            content_type: drafts::media_type(&attachment.content_type).to_owned(),
        })
    }

    fn threading(&mut self, message: &RemoteId) -> Result<Threading, ComposeError> {
        let json = self.message_metadata(message, &THREADING_HEADERS)?;
        Ok(drafts::threading(&json))
    }
}
