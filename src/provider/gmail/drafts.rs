//! Pure conversions between Gmail JSON and the provider-neutral compose types.
//! No HTTP: everything here is testable without a network.

use base64::alphabet;
use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig};
use base64::engine::{DecodePaddingMode, Engine as _};
use serde_json::{Value, json};

use super::super::compose::{
    DraftPage, MessageBody, RemoteAttachment, RemoteDraft, SendStatus, SentMail, Threading,
};
use crate::clock::Timestamp;
use crate::provider::RemoteId;

/// Gmail encodes message payloads as base64url, with or without padding.
pub fn decode_engine() -> GeneralPurpose {
    GeneralPurpose::new(
        &alphabet::URL_SAFE,
        GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
    )
}

/// base64url without padding, as `messages.raw` requires.
pub fn encode_engine() -> GeneralPurpose {
    GeneralPurpose::new(
        &alphabet::URL_SAFE,
        GeneralPurposeConfig::new().with_encode_padding(false),
    )
}

/// Decodes a base64url payload into raw bytes; `None` when it is not decodable.
pub fn decode_bytes(data: &str) -> Option<Vec<u8>> {
    decode_engine().decode(data.as_bytes()).ok()
}

/// True when the MIME declares `In-Reply-To`, the marker of a reply rather
/// than a fresh message.
pub fn is_reply(raw: &[u8]) -> bool {
    raw_headers(raw)
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("in-reply-to"))
}

/// Unfolds the header block of a raw RFC 2822 message into `(name, value)`
/// pairs, stopping at the first blank line.
fn raw_headers(raw: &[u8]) -> Vec<(String, String)> {
    let text = String::from_utf8_lossy(raw);
    let head = text.split("\r\n\r\n").next().unwrap_or_default();
    let mut out: Vec<(String, String)> = Vec::new();
    for line in head.lines() {
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some(last) = out.last_mut() {
                last.1.push(' ');
                last.1.push_str(line.trim());
            }
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            out.push((key.trim().to_owned(), value.trim().to_owned()));
        }
    }
    out
}

/// The `{"message": {"raw": …}}` body of `drafts.create` and `drafts.update`.
///
/// Gmail threads a message only when all three conditions from
/// <https://developers.google.com/workspace/gmail/api/guides/threads> hold: the
/// requested `threadId` is supplied, `References`/`In-Reply-To` are set per
/// RFC 2822, and the `Subject` matches. Only the first is this function's
/// business. The `In-Reply-To` check below is a structural guard against
/// forcing a brand new message into someone else's conversation; it is *not* a
/// substitute for the caller clearing the hint when the user actually edits
/// the subject, because a changed `Subject` is what breaks the third condition
/// and nothing here can see that.
pub fn raw_request(raw: &[u8], thread: Option<&str>) -> Value {
    let mut message = json!({"raw": encode_engine().encode(raw)});
    if let (Some(thread), true) = (thread, is_reply(raw)) {
        message["threadId"] = json!(thread);
    }
    json!({"message": message})
}

/// The body of `drafts.send`: a `Draft` naming the draft to send, with the
/// replacement content when the caller supplies it. The endpoint is
/// `POST /users/me/drafts/send`; the draft to send is selected by the `id` in
/// this body, not by the path.
pub fn send_request(draft: &str, mail: Option<(&[u8], Option<&str>)>) -> Value {
    match mail {
        None => json!({"id": draft}),
        Some((raw, thread)) => json!({
            "id": draft,
            "message": raw_request(raw, thread).get("message").cloned().unwrap_or(Value::Null),
        }),
    }
}

fn str_field<'a>(json: &'a Value, key: &str) -> Option<&'a str> {
    json.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

fn header<'a>(headers: &'a [Value], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|h| {
            h.get("name")
                .and_then(Value::as_str)
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
        })
        .and_then(|h| h.get("value"))
        .and_then(Value::as_str)
}

static NO_HEADERS: std::sync::LazyLock<Vec<Value>> = std::sync::LazyLock::new(Vec::new);

fn headers_of(message: &Value) -> &[Value] {
    message
        .get("payload")
        .and_then(|p| p.get("headers"))
        .and_then(Value::as_array)
        .map_or(NO_HEADERS.as_slice(), Vec::as_slice)
}

/// `internalDate` is epoch milliseconds; the seam speaks whole seconds.
fn seconds(json: &Value) -> Timestamp {
    str_field(json, "internalDate")
        .and_then(|ms| ms.parse::<i64>().ok())
        .map(|ms| ms.div_euclid(1000))
        .unwrap_or(0)
}

/// Splits `<id>` or `id` into the bare value.
fn bare_id(value: &str) -> String {
    value
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .to_owned()
}

/// Splits a `References` header into bare ids, oldest first.
pub fn references(value: &str) -> Vec<String> {
    value
        .split_whitespace()
        .map(bare_id)
        .filter(|id| !id.is_empty())
        .collect()
}

/// A part of a message payload, flattened out of the MIME tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part {
    pub part_id: String,
    pub filename: String,
    pub content_type: String,
    pub size: u64,
    /// Attachment resource id, when the part is not small enough to inline.
    pub attachment_id: Option<String>,
    /// Bytes carried inline in `body.data`.
    pub data: Option<Vec<u8>>,
    /// MIME `Content-ID` of the part, without angle brackets.
    pub content_id: Option<String>,
}

fn walk_parts(part: &Value, path: &str, out: &mut Vec<Part>) {
    if let Some(children) = part.get("parts").and_then(Value::as_array)
        && !children.is_empty()
    {
        for (i, child) in children.iter().enumerate() {
            walk_parts(child, &format!("{path}.{i}"), out);
        }
        return;
    }
    let body = part.get("body");
    let filename = part
        .get("filename")
        .and_then(Value::as_str)
        .filter(|f| !f.is_empty())
        .unwrap_or_default()
        .to_owned();
    let attachment_id = body
        .and_then(|b| b.get("attachmentId"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let data = body
        .and_then(|b| b.get("data"))
        .and_then(Value::as_str)
        .and_then(decode_bytes);
    // An inline image the body refers to with `cid:` is a part like any other,
    // and its content id is the only thing that addresses it.
    let content_id = part
        .get("headers")
        .and_then(Value::as_array)
        .and_then(|headers| header(headers, "Content-ID"))
        .map(bare_id)
        .filter(|id| !id.is_empty());
    let size = body
        .and_then(|b| b.get("size"))
        .and_then(Value::as_u64)
        .unwrap_or_else(|| data.as_ref().map_or(0, |d| d.len() as u64))
        .max(data.as_ref().map_or(0, |d| d.len() as u64));
    out.push(Part {
        part_id: path.to_owned(),
        filename,
        content_type: part
            .get("mimeType")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        size,
        attachment_id,
        data,
        content_id,
    });
}

/// Every leaf of a message's payload, in document order.
pub fn parts(message: &Value) -> Vec<Part> {
    message
        .get("payload")
        .map(payload_parts)
        .unwrap_or_default()
}

/// Every leaf of a payload, in document order. Takes the payload itself, not
/// the message that carries it.
fn payload_parts(payload: &Value) -> Vec<Part> {
    let mut out = Vec::new();
    walk_parts(payload, "", &mut out);
    out
}

/// A stable display name for a part that has none: `image/png` becomes
/// `image.png`. Deterministic, so the same raw message always names the same
/// part the same way. A part that declared no type at all is still an
/// attachment and still needs a name, so it gets the generic one rather than
/// an empty file name that would be indistinguishable from a part the walk
/// never reached.
fn derived_name(mimetype: &str) -> String {
    let Some((main, sub)) = mimetype.split_once('/') else {
        return "attachment".to_owned();
    };
    let sub = sub.split('+').next().unwrap_or("bin");
    if main.is_empty() || sub.is_empty() {
        return "attachment".to_owned();
    }
    format!("{main}.{sub}")
}

fn files(parts: &[Part]) -> Vec<RemoteAttachment> {
    parts
        .iter()
        .filter(|p| !p.filename.is_empty() || p.content_id.is_some())
        .map(|p| RemoteAttachment {
            id: p.attachment_id.clone(),
            content_id: p.content_id.clone(),
            name: if p.filename.is_empty() {
                derived_name(&p.content_type)
            } else {
                p.filename.clone()
            },
            size: p.size,
            content_type: media_type(&p.content_type).to_owned(),
            inline_data: p.data.clone(),
        })
        .collect()
}

/// The bare `type/subtype` of a part: the declared media type with any
/// parameters (`charset`, `name`) dropped, or an empty string when the part
/// declared none. Parameters are not the media type and a client reading a
/// header must not be handed one as if it were, but an absent declaration is
/// reported as absent rather than invented here — the caller's file name is
/// the better answer, and the attachment store is where it is asked.
pub fn media_type(declared: &str) -> &str {
    declared.split(';').next().unwrap_or(declared).trim()
}

fn body_of(parts: &[Part]) -> MessageBody {
    let mut out = MessageBody::default();
    for part in parts {
        // A part with a file name is a file the user can detach, not the
        // message text — even when it happens to be `text/plain`.
        if !part.filename.is_empty() || part.content_id.is_some() {
            continue;
        }
        let Some(text) = part
            .data
            .as_deref()
            .map(|b| String::from_utf8_lossy(b).into_owned())
        else {
            continue;
        };
        match media_type(&part.content_type) {
            "text/plain" if out.text.is_empty() => out.text = text,
            "text/html" if out.html.is_none() => out.html = Some(text),
            // Gmail omits mimeType on some single-part bodies.
            "" if out.text.is_empty() && out.html.is_none() => out.text = text,
            _ => {}
        }
    }
    out
}

/// The message behind a draft, with its body already parsed.
fn message_of(json: &Value) -> &Value {
    json.get("message").unwrap_or(json)
}

/// A `users.drafts` resource, either from `list` (headers absent, since a
/// listed draft carries only ids) or from `get`.
pub fn draft(json: &Value) -> Option<RemoteDraft> {
    let id = str_field(json, "id")?;
    let message = message_of(json);
    let headers = headers_of(message);
    let parts = message
        .get("payload")
        .map(payload_parts)
        .unwrap_or_default();
    let attachments = files(&parts);
    let updated = seconds(message);
    let snippet = str_field(message, "snippet").unwrap_or_default().to_owned();
    // A listed draft carries no payload at all: fall back to the snippet so the
    // draft list still shows something, and report its files as unknown rather
    // than claiming it has none.
    let loaded = message.get("payload").is_some();
    let body = if loaded {
        body_of(&parts)
    } else {
        MessageBody {
            text: snippet.clone(),
            html: None,
        }
    };
    Some(RemoteDraft {
        id: id.to_owned(),
        message: str_field(message, "id").unwrap_or_default().to_owned(),
        thread: str_field(message, "threadId")
            .unwrap_or_default()
            .to_owned(),
        revision: str_field(message, "id").unwrap_or_default().to_owned(),
        // The id the message itself carries, which is what a send of these very
        // bytes would be reconciled by.
        message_id: header(headers, "Message-Id")
            .or_else(|| header(headers, "Message-ID"))
            .map(bare_id)
            .filter(|id| !id.is_empty()),
        to: header(headers, "To").unwrap_or_default().to_owned(),
        cc: header(headers, "Cc").unwrap_or_default().to_owned(),
        bcc: header(headers, "Bcc").unwrap_or_default().to_owned(),
        subject: header(headers, "Subject").unwrap_or_default().to_owned(),
        body,
        snippet,
        // Only a payload that was actually walked can answer this. A size
        // estimate says how big the message is, not whether it carries files.
        has_attachments: !attachments.is_empty(),
        attachments_known: loaded,
        updated,
        attachments,
    })
}

/// Identity of a draft from a create or update response, which carries ids but
/// no payload. Enough to address the draft and to refuse a later stale write.
pub fn draft_identity(json: &Value) -> Option<(RemoteId, RemoteId, RemoteId)> {
    let id = str_field(json, "id")?.to_owned();
    let message = message_of(json);
    Some((
        id,
        str_field(message, "id").unwrap_or_default().to_owned(),
        str_field(message, "threadId")
            .unwrap_or_default()
            .to_owned(),
    ))
}

/// One `users.drafts.list` page.
pub fn draft_page(json: &Value) -> DraftPage {
    DraftPage {
        drafts: json
            .get("drafts")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(draft)
            .collect(),
        next: str_field(json, "nextPageToken").map(str::to_owned),
    }
}

/// `drafts.get?format=raw` returns the stored message as base64url.
pub fn draft_raw(json: &Value) -> Option<Vec<u8>> {
    str_field(message_of(json), "raw").and_then(decode_bytes)
}

/// The message `drafts.send` returns, carrying the new `SENT` message id.
pub fn sent(json: &Value) -> Option<SentMail> {
    let headers = headers_of(json);
    Some(SentMail {
        id: str_field(json, "id")?.to_owned(),
        thread: str_field(json, "threadId").unwrap_or_default().to_owned(),
        message_id: header(headers, "Message-Id")
            .or_else(|| header(headers, "Message-ID"))
            .map(bare_id)
            .unwrap_or_default(),
        sent_at: seconds(json),
    })
}

/// A send either came back as a sent message or left us unable to tell.
pub fn send_status(json: &Value) -> SendStatus {
    sent(json).map_or(
        SendStatus::Uncertain {
            detail: "send response carried no message".into(),
        },
        SendStatus::Sent,
    )
}

/// `rfc822msgid:` finds a message by its exact RFC 5322 `Message-ID`, which is
/// what makes a lost send response reconcilable.
pub fn sent_query(message_id: &str) -> String {
    format!("rfc822msgid:{}", message_id.trim_matches(['<', '>']))
}

/// The matching message from a `messages.list` result, verified against its own
/// `Message-ID` header: the search operator is documented but a match that does
/// not carry the id is not our send and must not be reported as one.
pub fn reconcile_sent(json: &Value, message_id: &str) -> Option<SentMail> {
    let id = str_field(json, "id")?;
    let headers = headers_of(json);
    let found = header(headers, "Message-Id")
        .or_else(|| header(headers, "Message-ID"))
        .map(bare_id)
        .unwrap_or_default();
    if found != bare_id(message_id) {
        return None;
    }
    Some(SentMail {
        id: id.to_owned(),
        thread: str_field(json, "threadId").unwrap_or_default().to_owned(),
        message_id: found,
        sent_at: seconds(json),
    })
}

/// RFC 5322 threading headers of a message fetched with
/// `metadataHeaders=Message-ID&metadataHeaders=References&metadataHeaders=In-Reply-To&metadataHeaders=Subject`.
pub fn threading(json: &Value) -> Threading {
    let headers = headers_of(json);
    let message_id = header(headers, "Message-Id")
        .or_else(|| header(headers, "Message-ID"))
        .map(bare_id);
    Threading {
        references: reply_chain(
            message_id.as_deref().unwrap_or_default(),
            header(headers, "References"),
            header(headers, "In-Reply-To"),
        ),
        message_id: message_id.unwrap_or_default(),
        subject: header(headers, "Subject").unwrap_or_default().to_owned(),
    }
}

/// The RFC 5322 chain a reply to a message must carry: the ancestors, then
/// the message itself. `References` is the chain proper, but a client that
/// omitted it still named its direct parent in `In-Reply-To`, and dropping
/// that would start a new conversation instead of continuing this one — so
/// the parent is used when there is no chain to carry.
pub fn reply_chain(
    message_id: &str,
    chain: Option<&str>,
    in_reply_to: Option<&str>,
) -> Vec<String> {
    let mut chain = chain.map(references).unwrap_or_default();
    if chain.is_empty()
        && let Some(parent) = in_reply_to.map(bare_id).filter(|id| !id.is_empty())
    {
        chain.push(parent);
    }
    // A reply's References is the chain *plus* its own id; keep that shape so
    // a reply to a reply threads the whole conversation.
    if !message_id.is_empty() && chain.last() != Some(&message_id.to_owned()) {
        chain.push(message_id.to_owned());
    }
    chain
}

/// Files carried by a message, with bytes where the listing had them.
pub fn attachments(json: &Value) -> Vec<RemoteAttachment> {
    files(&parts(json))
}

/// `messages.attachments.get` returns the bytes base64url-encoded in `data`.
pub fn attachment_data(json: &Value) -> Option<Vec<u8>> {
    str_field(json, "data").and_then(decode_bytes)
}
