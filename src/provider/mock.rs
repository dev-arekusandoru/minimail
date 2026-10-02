//! In-memory [`ComposeProvider`] for mock accounts and tests.
//!
//! Deterministic and honest: drafts get stable ids, replacing a draft mints a
//! new underlying message id exactly as Gmail does, sends are recorded under the
//! RFC 5322 `Message-ID` the caller wrote into the MIME, and faults can be
//! injected so a lost send response is reproducible instead of hypothetical.
//! Nothing here pretends a send succeeded when it did not.

use std::collections::BTreeMap;

use crate::clock::Timestamp;

use super::compose::{
    AttachmentBytes, ComposeError, ComposeProvider, DraftPage, MessageBody, OutgoingMail,
    RemoteAttachment, RemoteDraft, SendStatus, SentMail, Threading,
};
use super::mime::{ParsedMessage, attachments as files_of, header, parse_message};
use super::{ProviderError, RemoteId};

/// A fault to inject into the next calls of a given kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MockFault {
    /// The request never left the machine: nothing was applied, and the same
    /// request can be tried again later.
    Offline,
    /// The server rejected the request before acting.
    Rejected,
    /// The server failed, and may have acted before failing.
    ServerError,
    /// The request left and the answer was lost. On a send the mail is
    /// delivered — that is the whole point of the fault.
    LostResponse,
}

/// One draft as the mock holds it.
#[derive(Clone, Debug)]
struct Entry {
    message: String,
    thread: String,
    rfc822: Vec<u8>,
    attachments: Vec<RemoteAttachment>,
    updated: Timestamp,
}

/// Everything a test wants to observe about what the mock did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MockLog {
    pub listed: usize,
    pub created: Vec<RemoteId>,
    pub updated: Vec<RemoteId>,
    pub deleted: Vec<RemoteId>,
    pub sent: Vec<RemoteId>,
    pub attachment_reads: Vec<(RemoteId, RemoteId)>,
}

pub struct MockCompose {
    drafts: BTreeMap<RemoteId, Entry>,
    /// Draft ids in creation order, so a listing is newest-first the way a
    /// server orders by time rather than by id.
    order: Vec<RemoteId>,
    sent: Vec<SentMail>,
    threads: BTreeMap<RemoteId, String>,
    log: MockLog,
    next: usize,
    now: Timestamp,
    fault: Option<MockFault>,
}

/// Decodes stored MIME, turning a parse failure into a compose error. A draft
/// whose parts cannot be decoded has no honest file listing, so the caller is
/// told rather than handed a short one.
fn decode(raw: &[u8]) -> Result<ParsedMessage, ComposeError> {
    parse_message(raw).map_err(|e| {
        ComposeError::Provider(ProviderError::Api {
            status: 422,
            message: format!("cannot read stored MIME: {e}"),
        })
    })
}

impl MockCompose {
    pub fn new(now: Timestamp) -> Self {
        Self {
            drafts: BTreeMap::new(),
            order: Vec::new(),
            sent: Vec::new(),
            threads: BTreeMap::new(),
            log: MockLog::default(),
            next: 0,
            now,
            fault: None,
        }
    }

    /// Arms a fault for subsequent calls; `None` clears it.
    pub fn set_fault(&mut self, fault: Option<MockFault>) {
        self.fault = fault;
    }

    pub fn log(&self) -> &MockLog {
        &self.log
    }

    /// The draft ids currently held, oldest first.
    pub fn draft_ids(&self) -> Vec<RemoteId> {
        self.order.clone()
    }

    /// Registers a message's RFC 5322 headers so [`ComposeProvider::threading`]
    /// can answer for it, standing in for a mailbox the mock does not read.
    pub fn add_message(&mut self, message: &RemoteId, raw: &[u8], thread: &RemoteId) {
        self.threads.insert(message.clone(), thread.clone());
        self.threads.insert(
            format!("raw:{message}"),
            String::from_utf8_lossy(raw).into_owned(),
        );
    }

    /// Attaches bytes to a draft, as if the user had added a file the composer
    /// did not embed.
    pub fn attach(&mut self, draft: &RemoteId, name: &str, content_type: &str, bytes: &[u8]) {
        let Some(entry) = self.drafts.get_mut(draft) else {
            return;
        };
        entry.attachments.push(RemoteAttachment {
            id: Some(format!("att-{}", entry.attachments.len())),
            content_id: None,
            name: name.to_owned(),
            size: bytes.len() as u64,
            content_type: content_type.to_owned(),
            inline_data: Some(bytes.to_vec()),
        });
    }

    fn id(&mut self) -> String {
        self.next += 1;
        format!("draft-{}", self.next)
    }

    fn view(&self, id: &RemoteId, entry: &Entry) -> RemoteDraft {
        let parsed = parse_message(&entry.rfc822).expect("mock drafts hold valid MIME");
        let mut attachments = entry.attachments.clone();
        // Files the MIME itself carries are reported even when nobody recorded
        // them, so a draft that arrived with attachments shows them.
        attachments.extend(files_of(&parsed));
        RemoteDraft {
            id: id.clone(),
            message: entry.message.clone(),
            thread: entry.thread.clone(),
            revision: entry.message.clone(),
            message_id: header(&parsed, "message-id").map(|v| super::mime::bare_message_id(&v)),
            to: header(&parsed, "to").unwrap_or_default(),
            cc: header(&parsed, "cc").unwrap_or_default(),
            bcc: header(&parsed, "bcc").unwrap_or_default(),
            subject: header(&parsed, "subject").unwrap_or_default(),
            snippet: parsed.text.chars().take(120).collect(),
            body: MessageBody {
                text: parsed.text.clone(),
                html: parsed.html.clone(),
            },
            has_attachments: !attachments.is_empty(),
            // The mock always holds the stored MIME, so its answer is complete.
            attachments_known: true,
            updated: entry.updated,
            attachments,
        }
    }

    /// The fault verdict for a call, if one is armed.
    fn faulted(&self, action: &str) -> Result<(), ComposeError> {
        match &self.fault {
            None => Ok(()),
            // Offline proves nothing left the machine.
            Some(MockFault::Offline) => Err(ComposeError::Offline),
            // A rejected request never reached the write.
            Some(MockFault::Rejected) => Err(ComposeError::Provider(ProviderError::Api {
                status: 400,
                message: "invalid recipient".into(),
            })),
            // A server that failed may have acted before failing.
            Some(MockFault::ServerError) => Err(ComposeError::Provider(ProviderError::Api {
                status: 503,
                message: "backend error".into(),
            })),
            // The request went out; the answer did not come back.
            Some(MockFault::LostResponse) => Err(ComposeError::Uncertain {
                detail: format!("{action}: response lost"),
            }),
        }
    }

    fn store(&mut self, mail: &OutgoingMail) -> Entry {
        self.next += 1;
        let message = format!("m-{}", self.next);
        let thread = mail.thread.clone().unwrap_or_else(|| message.clone());
        Entry {
            message,
            thread,
            rfc822: mail.rfc822.clone(),
            attachments: Vec::new(),
            updated: self.now,
        }
    }
}

impl ComposeProvider for MockCompose {
    fn list_drafts(&mut self, page: Option<&str>, max: usize) -> Result<DraftPage, ComposeError> {
        self.faulted("listing drafts")?;
        self.log.listed += 1;
        let start = match page {
            Some(token) => token.parse::<usize>().map_err(|_| {
                ComposeError::Provider(ProviderError::Api {
                    status: 400,
                    message: format!("bad page token {token}"),
                })
            })?,
            None => 0,
        };
        let window: Vec<RemoteId> = self
            .order
            .iter()
            .rev()
            .skip(start)
            .take(max.max(1))
            .cloned()
            .collect();
        let drafts = window
            .iter()
            .filter_map(|id| self.drafts.get(id).map(|e| self.view(id, e)))
            .collect();
        let next =
            (start + window.len() < self.order.len()).then(|| (start + window.len()).to_string());
        Ok(DraftPage { drafts, next })
    }

    fn get_draft(&mut self, id: &RemoteId) -> Result<RemoteDraft, ComposeError> {
        self.faulted("getting the draft")?;
        let entry = self
            .drafts
            .get(id)
            .ok_or_else(|| ComposeError::NotFound(id.clone()))?;
        Ok(self.view(id, entry))
    }

    fn draft_raw(&mut self, id: &RemoteId) -> Result<Vec<u8>, ComposeError> {
        self.faulted("reading the draft")?;
        self.drafts
            .get(id)
            .map(|e| e.rfc822.clone())
            .ok_or_else(|| ComposeError::NotFound(id.clone()))
    }

    fn create_draft(&mut self, mail: &OutgoingMail) -> Result<RemoteDraft, ComposeError> {
        // Only faults that prove nothing was applied are raised before the
        // draft exists; a lost answer happens after it does.
        match self.fault {
            Some(MockFault::Offline) => return Err(ComposeError::Offline),
            Some(MockFault::Rejected) => {
                return Err(ComposeError::Provider(ProviderError::Api {
                    status: 400,
                    message: "invalid recipient".into(),
                }));
            }
            _ => {}
        }
        decode(&mail.rfc822)?;
        let id = self.id();
        let entry = self.store(mail);
        self.drafts.insert(id.clone(), entry);
        self.order.push(id.clone());
        self.log.created.push(id.clone());
        if self.fault == Some(MockFault::LostResponse) {
            // The draft exists; the caller just never learned its id, which is
            // why the seam tells it to re-list rather than to create again.
            // Nothing was mailed, so this is not a delivery ambiguity.
            return Err(ComposeError::AutosaveUncertain {
                detail: "creating the draft: response lost".into(),
            });
        }
        if self.fault == Some(MockFault::ServerError) {
            return Err(ComposeError::Provider(ProviderError::Api {
                status: 503,
                message: "backend error".into(),
            }));
        }
        let entry = &self.drafts[&id];
        Ok(self.view(&id, entry))
    }

    fn update_draft(
        &mut self,
        draft: &RemoteDraft,
        mail: &OutgoingMail,
    ) -> Result<RemoteDraft, ComposeError> {
        decode(&mail.rfc822)?;
        // Only faults that prove nothing was applied are raised before the
        // write; a lost answer happens after it.
        match self.fault {
            Some(MockFault::Offline) => return Err(ComposeError::Offline),
            Some(MockFault::Rejected) => {
                return Err(ComposeError::Provider(ProviderError::Api {
                    status: 400,
                    message: "invalid recipient".into(),
                }));
            }
            _ => {}
        }
        let Some(current) = self.drafts.get(&draft.id) else {
            return Err(ComposeError::NotFound(draft.id.clone()));
        };
        if current.message != draft.message {
            return Err(ComposeError::Conflict {
                draft: draft.id.clone(),
                expected: draft.revision.clone(),
                actual: current.message.clone(),
            });
        }
        // Replacing the content mints a new message id, exactly as Gmail does:
        // that is what makes a later update by a stale caller a conflict.
        self.next += 1;
        let thread = mail
            .thread
            .clone()
            .unwrap_or_else(|| current.thread.clone());
        let entry = self.drafts.get_mut(&draft.id).expect("checked above");
        entry.message = format!("m-{}", self.next);
        entry.thread = thread;
        entry.rfc822 = mail.rfc822.clone();
        entry.updated = self.now;
        self.log.updated.push(draft.id.clone());
        if matches!(
            self.fault,
            Some(MockFault::LostResponse) | Some(MockFault::ServerError)
        ) {
            // The replacement happened; the caller does not know that. No mail
            // was sent, so this is not a delivery ambiguity.
            return Err(ComposeError::AutosaveUncertain {
                detail: "replacing the draft: response lost".into(),
            });
        }
        let entry = self.drafts.get(&draft.id).expect("just written");
        Ok(self.view(&draft.id, entry))
    }

    fn delete_draft(&mut self, id: &RemoteId) -> Result<(), ComposeError> {
        match self.fault {
            Some(MockFault::Offline) => return Err(ComposeError::Offline),
            Some(MockFault::Rejected) => {
                return Err(ComposeError::Provider(ProviderError::Api {
                    status: 400,
                    message: "invalid recipient".into(),
                }));
            }
            _ => {}
        }
        self.drafts.remove(id);
        self.order.retain(|d| d != id);
        self.log.deleted.push(id.clone());
        if matches!(
            self.fault,
            Some(MockFault::LostResponse) | Some(MockFault::ServerError)
        ) {
            // The draft is gone but the caller cannot tell; re-listing settles
            // it. No mail was sent either way.
            return Err(ComposeError::AutosaveUncertain {
                detail: "deleting the draft: response lost".into(),
            });
        }
        Ok(())
    }

    fn send_draft(
        &mut self,
        draft: &RemoteDraft,
        mail: Option<&OutgoingMail>,
    ) -> Result<SendStatus, ComposeError> {
        // Only faults that prove nothing was applied are raised before the
        // mail leaves, so those leave the draft in place.
        match self.fault {
            Some(MockFault::Offline) => return Err(ComposeError::Offline),
            Some(MockFault::Rejected) => {
                return Err(ComposeError::Provider(ProviderError::Api {
                    status: 400,
                    message: "invalid recipient".into(),
                }));
            }
            _ => {}
        }
        let Some(entry) = self.drafts.get(&draft.id) else {
            return Err(ComposeError::NotFound(draft.id.clone()));
        };
        // A send with `mail = None` puts whatever the server holds in front of
        // people, so a draft that moved on since the caller's snapshot is a
        // conflict, not something to send unseen.
        if entry.message != draft.message {
            return Err(ComposeError::Conflict {
                draft: draft.id.clone(),
                expected: draft.revision.clone(),
                actual: entry.message.clone(),
            });
        }
        let thread = entry.thread.clone();
        let raw = match mail {
            Some(mail) => mail.rfc822.clone(),
            None => entry.rfc822.clone(),
        };
        let parsed = decode(&raw)?;
        // The mail leaves before a lost or failed answer is reported:
        // reconciliation has to be able to find what the caller cannot see.
        self.next += 1;
        let sent = SentMail {
            id: format!("sent-{}", self.next),
            thread,
            message_id: header(&parsed, "message-id")
                .map(|v| super::mime::bare_message_id(&v))
                .unwrap_or_default(),
            sent_at: self.now,
        };
        self.add_message(&sent.id, &raw, &sent.thread);
        self.sent.push(sent.clone());
        self.drafts.remove(&draft.id);
        self.order.retain(|d| d != &draft.id);
        self.log.sent.push(draft.id.clone());
        match self.fault {
            // Delivered, and the caller does not know it.
            Some(MockFault::LostResponse) => Err(ComposeError::Uncertain {
                detail: "sending: response lost".into(),
            }),
            // Delivered or not: the server cannot say, so neither can we.
            Some(MockFault::ServerError) => Err(ComposeError::Provider(ProviderError::Api {
                status: 503,
                message: "backend error".into(),
            })),
            _ => Ok(SendStatus::Sent(sent)),
        }
    }

    fn find_sent(&mut self, message_id: &str) -> Result<Option<SentMail>, ComposeError> {
        self.faulted("reconciling the send")?;
        Ok(self
            .sent
            .iter()
            .find(|s| s.message_id == message_id)
            .cloned())
    }

    fn list_attachments(
        &mut self,
        message: &RemoteId,
    ) -> Result<Vec<RemoteAttachment>, ComposeError> {
        self.faulted("listing attachments")?;
        if let Some(entry) = self.drafts.values().find(|entry| &entry.message == message) {
            let mut out = entry.attachments.clone();
            out.extend(files_of(&decode(entry.rfc822.as_slice())?));
            return Ok(out);
        }
        if let Some(raw) = self.threads.get(&format!("raw:{message}")) {
            return Ok(files_of(&decode(raw.as_bytes())?));
        }
        Err(ComposeError::NotFound(message.clone()))
    }

    fn attachment_bytes(
        &mut self,
        message: &RemoteId,
        attachment: &RemoteAttachment,
    ) -> Result<AttachmentBytes, ComposeError> {
        self.faulted("reading the attachment")?;
        if let Some(bytes) = &attachment.inline_data {
            if let Some(id) = &attachment.id {
                self.log
                    .attachment_reads
                    .push((message.clone(), id.clone()));
            }
            return Ok(AttachmentBytes {
                size: bytes.len() as u64,
                bytes: bytes.clone(),
                content_type: attachment.content_type.clone(),
            });
        }
        let id = attachment
            .id
            .as_ref()
            .ok_or_else(|| ComposeError::NotFound(attachment.name.clone()))?;
        let parts =
            if let Some(entry) = self.drafts.values().find(|entry| &entry.message == message) {
                let mut parts = entry.attachments.clone();
                parts.extend(files_of(&decode(entry.rfc822.as_slice())?));
                parts
            } else if let Some(raw) = self.threads.get(&format!("raw:{message}")) {
                files_of(&decode(raw.as_bytes())?)
            } else {
                return Err(ComposeError::NotFound(message.clone()));
            };
        let part = parts
            .into_iter()
            .find(|part| part.id.as_ref() == Some(id))
            .ok_or_else(|| ComposeError::NotFound(id.clone()))?;
        let bytes = part
            .inline_data
            .ok_or_else(|| ComposeError::NotFound(id.clone()))?;
        self.log
            .attachment_reads
            .push((message.clone(), id.clone()));
        Ok(AttachmentBytes {
            size: bytes.len() as u64,
            bytes,
            content_type: part.content_type,
        })
    }

    fn threading(&mut self, message: &RemoteId) -> Result<Threading, ComposeError> {
        self.faulted("reading threading headers")?;
        let raw = self
            .threads
            .get(&format!("raw:{message}"))
            .ok_or_else(|| ComposeError::NotFound(message.clone()))?;
        let parsed = decode(raw.as_bytes())?;
        let message_id = header(&parsed, "message-id")
            .map(|v| super::mime::bare_message_id(&v))
            .unwrap_or_default();
        Ok(Threading {
            references: super::gmail::drafts::reply_chain(
                &message_id,
                header(&parsed, "references").as_deref(),
                header(&parsed, "in-reply-to").as_deref(),
            ),
            message_id,
            subject: header(&parsed, "subject").unwrap_or_default(),
        })
    }
}
