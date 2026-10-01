//! Pure conversion between Gmail JSON and provider-agnostic types. No HTTP.

use std::collections::HashSet;

use base64::alphabet;
use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig};
use base64::engine::{DecodePaddingMode, Engine as _};
use serde_json::Value;

use super::super::{RemoteMessage, RemoteState};
use crate::clock::Timestamp;
use crate::model::Attachment;

const LABEL_INBOX: &str = "INBOX";
const LABEL_TRASH: &str = "TRASH";
const LABEL_SENT: &str = "SENT";
const LABEL_SPAM: &str = "SPAM";
const LABEL_DRAFT: &str = "DRAFT";
const LABEL_CHAT: &str = "CHAT";

/// Labels to add and remove for a triage move. Archiving only strips `INBOX`,
/// so other user labels are preserved.
pub fn label_ops(from: &RemoteState, to: &RemoteState) -> (Vec<String>, Vec<String>) {
    let mut add = Vec::new();
    let mut remove = Vec::new();
    match to {
        RemoteState::Inbox => add.push(LABEL_INBOX.to_owned()),
        RemoteState::Folder(id) => add.push(id.clone()),
        RemoteState::Archived | RemoteState::Trash => {}
    }
    if !matches!(to, RemoteState::Inbox) {
        remove.push(LABEL_INBOX.to_owned());
    }
    if let RemoteState::Folder(from_id) = from
        && !matches!(to, RemoteState::Folder(to_id) if to_id == from_id)
    {
        remove.push(from_id.clone());
    }
    (add, remove)
}

/// Message ids from a `messages.list` page: `{"messages": [{"id": …, "threadId": …}, …]}`.
pub fn list_ids(json: &Value) -> Vec<String> {
    json.get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|m| m.get("id").and_then(Value::as_str).map(str::to_owned))
        .collect()
}

/// Maps a message's label list to a triage state.
pub fn state_from_labels(labels: &[String], user_folders: &HashSet<String>) -> RemoteState {
    if labels.iter().any(|l| l == LABEL_TRASH) {
        return RemoteState::Trash;
    }
    if labels.iter().any(|l| l == LABEL_INBOX) {
        return RemoteState::Inbox;
    }
    for label in labels {
        if user_folders.contains(label) {
            return RemoteState::Folder(label.clone());
        }
    }
    RemoteState::Archived
}

/// Gmail encodes message bodies as base64url with optional padding.
fn decode_engine() -> GeneralPurpose {
    GeneralPurpose::new(
        &alphabet::URL_SAFE,
        GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
    )
}

fn decode_text(data: &Value) -> String {
    data.as_str()
        .and_then(|raw| decode_engine().decode(raw.as_bytes()).ok())
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default()
}

/// Splits `Name <a@b>`, `"Name" <a@b>`, or a bare `a@b` into `(name, email)`.
fn split_from(value: &str) -> (String, String) {
    let value = value.trim();
    if let Some(start) = value.rfind('<')
        && let Some(rel_end) = value[start..].find('>')
    {
        let email = value[start + 1..start + rel_end].trim().to_owned();
        let mut name = value[..start].trim().to_owned();
        if name.len() >= 2 && name.starts_with('"') && name.ends_with('"') {
            name = name[1..name.len() - 1].to_owned();
        }
        return (name, email);
    }
    (value.to_owned(), value.to_owned())
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

/// Walks a MIME payload depth-first, collecting the first text/plain and
/// text/html parts plus every part carrying a filename.
fn walk(part: &Value, body: &mut String, html: &mut Option<String>, files: &mut Vec<Attachment>) {
    let Some(obj) = part.as_object() else { return };
    if let Some(parts) = obj.get("parts").and_then(Value::as_array)
        && !parts.is_empty()
    {
        for child in parts {
            walk(child, body, html, files);
        }
        return;
    }
    let Some(part_body) = obj.get("body").and_then(Value::as_object) else {
        return;
    };
    if let Some(name) = obj
        .get("filename")
        .and_then(Value::as_str)
        .filter(|f| !f.is_empty())
    {
        let size = part_body.get("size").and_then(Value::as_u64).unwrap_or(0);
        files.push(Attachment {
            name: name.to_owned(),
            size,
        });
        return;
    }
    let Some(data) = part_body.get("data").and_then(Value::as_str) else {
        return;
    };
    let mime = obj.get("mimeType").and_then(Value::as_str).unwrap_or("");
    let decoded = decode_text(&Value::String(data.to_owned()));
    match mime {
        "text/plain" if body.is_empty() => *body = decoded,
        "text/html" if html.is_none() => *html = Some(decoded),
        // Gmail omits mimeType on some single-part bodies.
        "" if body.is_empty() && html.is_none() => *body = decoded,
        _ => {}
    }
}

/// Converts one Gmail message resource. `None` means the message is filtered
/// out (spam, draft, chat) or carries no id.
pub fn message(json: &Value, user_folders: &HashSet<String>) -> Option<RemoteMessage> {
    let id = json.get("id").and_then(Value::as_str)?;
    let labels: Vec<String> = json
        .get("labelIds")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    if labels
        .iter()
        .any(|l| l == LABEL_SPAM || l == LABEL_DRAFT || l == LABEL_CHAT)
    {
        return None;
    }
    let payload = json.get("payload").unwrap_or(&Value::Null);
    let empty: Vec<Value> = Vec::new();
    let headers: &[Value] = payload
        .get("headers")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    let (from_name, from_email) = match header(headers, "From") {
        Some(v) => split_from(v),
        None => (String::new(), String::new()),
    };
    let mut body = String::new();
    let mut html = None;
    let mut attachments = Vec::new();
    walk(payload, &mut body, &mut html, &mut attachments);
    let received: Timestamp = json
        .get("internalDate")
        .and_then(Value::as_str)
        .and_then(|ms| ms.parse::<i64>().ok())
        .map(|ms| ms.div_euclid(1000))
        .unwrap_or(0);
    Some(RemoteMessage {
        id: id.to_owned(),
        thread: json
            .get("threadId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        from_name,
        from_email,
        to: header(headers, "To").unwrap_or_default().to_owned(),
        cc: header(headers, "Cc").unwrap_or_default().to_owned(),
        bcc: header(headers, "Bcc").unwrap_or_default().to_owned(),
        subject: header(headers, "Subject").unwrap_or_default().to_owned(),
        body,
        html,
        received,
        attachments,
        state: state_from_labels(&labels, user_folders),
        outgoing: labels.iter().any(|l| l == LABEL_SENT),
    })
}