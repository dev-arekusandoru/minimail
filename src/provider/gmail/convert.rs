//! Pure conversion between Gmail JSON and provider-agnostic types. No HTTP.

use std::collections::HashSet;

use base64::alphabet;
use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig};
use base64::engine::{DecodePaddingMode, Engine as _};
use serde_json::Value;

use super::super::{Body, Changes, RemoteFlags, RemoteMessage, RemoteState, Scope, Window};
use crate::clock::Timestamp;
use crate::model::Attachment;

const LABEL_INBOX: &str = "INBOX";
const LABEL_TRASH: &str = "TRASH";
const LABEL_SENT: &str = "SENT";
const LABEL_SPAM: &str = "SPAM";
const LABEL_DRAFT: &str = "DRAFT";
const LABEL_CHAT: &str = "CHAT";
const LABEL_UNREAD: &str = "UNREAD";

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

/// Gmail signals throttling as 429, or as 403 with a rate/quota reason
/// (e.g. "Quota exceeded for quota metric … per minute per user").
pub fn is_rate_limited(status: u16, error_body: &Value) -> bool {
    if status == 429 {
        return true;
    }
    let error = &error_body["error"];
    status == 403
        && (error["status"] == "RESOURCE_EXHAUSTED"
            || error["message"].as_str().is_some_and(|m| m.starts_with("Quota exceeded"))
            || error["errors"].as_array().into_iter().flatten().any(|e| {
                matches!(
                    e["reason"].as_str(),
                    Some("rateLimitExceeded" | "userRateLimitExceeded" | "quotaExceeded")
                )
            }))
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

fn labels_of(json: &Value) -> Vec<String> {
    json.get("labelIds")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect())
        .unwrap_or_default()
}

/// Spam, drafts and chats never enter the mailbox.
fn is_filtered(labels: &[String]) -> bool {
    labels.iter().any(|l| l == LABEL_SPAM || l == LABEL_DRAFT || l == LABEL_CHAT)
}

/// State and unread flag from a message's labels.
pub fn flags(labels: &[String], user_folders: &HashSet<String>) -> RemoteFlags {
    RemoteFlags {
        state: state_from_labels(labels, user_folders),
        unread: labels.iter().any(|l| l == LABEL_UNREAD),
    }
}

/// Decodes the HTML entities Gmail puts in snippets.
pub fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let decoded = rest.find(';').filter(|&end| end <= 10).and_then(|end| {
            let entity = &rest[1..end];
            let ch = match entity {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                _ => entity
                    .strip_prefix("#x")
                    .or_else(|| entity.strip_prefix("#X"))
                    .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                    .or_else(|| entity.strip_prefix('#').and_then(|d| d.parse().ok()))
                    .and_then(char::from_u32),
            }?;
            Some((ch, end))
        });
        match decoded {
            Some((ch, end)) => {
                out.push(ch);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Converts a Gmail message resource (`format=metadata` or `full`) to headers,
/// snippet and flags. `None` means the message is filtered out (spam, draft,
/// chat) or carries no id.
pub fn message(json: &Value, user_folders: &HashSet<String>) -> Option<RemoteMessage> {
    let id = json.get("id").and_then(Value::as_str)?;
    let labels = labels_of(json);
    if is_filtered(&labels) {
        return None;
    }
    let empty: Vec<Value> = Vec::new();
    let headers: &[Value] = json
        .get("payload")
        .and_then(|p| p.get("headers"))
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    let (from_name, from_email) = match header(headers, "From") {
        Some(v) => split_from(v),
        None => (String::new(), String::new()),
    };
    let received: Timestamp = json
        .get("internalDate")
        .and_then(Value::as_str)
        .and_then(|ms| ms.parse::<i64>().ok())
        .map(|ms| ms.div_euclid(1000))
        .unwrap_or(0);
    let RemoteFlags { state, unread } = flags(&labels, user_folders);
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
        snippet: decode_entities(json.get("snippet").and_then(Value::as_str).unwrap_or_default()),
        received,
        state,
        outgoing: labels.iter().any(|l| l == LABEL_SENT),
        unread,
    })
}

/// Body, html and attachments of a `format=full` message resource.
pub fn body(json: &Value) -> Body {
    let mut out = Body::default();
    walk(
        json.get("payload").unwrap_or(&Value::Null),
        &mut out.body,
        &mut out.html,
        &mut out.attachments,
    );
    out
}

/// `messages.list` parameters for a scope and window: the `q` search string,
/// a label filter, and whether trash must be included.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListQuery {
    pub q: String,
    pub label: Option<String>,
    pub include_trash: bool,
}

pub fn list_query(scope: &Scope, window: Window) -> ListQuery {
    let mut q = String::from("-in:spam -in:drafts -in:chats");
    let mut label = None;
    let mut include_trash = false;
    match scope {
        Scope::Inbox => q.push_str(" in:inbox"),
        Scope::Archive => q.push_str(" -in:inbox -in:trash has:nouserlabels"),
        Scope::Trash => {
            q.push_str(" in:trash");
            include_trash = true;
        }
        Scope::Folder(id) => label = Some(id.clone()),
    }
    match window {
        Window::Since(t) => q.push_str(&format!(" after:{}", t - 1)),
        Window::Before(t) => q.push_str(&format!(" before:{t}")),
    }
    ListQuery { q, label, include_trash }
}

/// Merges one `history.list` page into `out`. Label changes carry the
/// message's full current label set, so they become flags without a fetch.
pub fn merge_history(json: &Value, user_folders: &HashSet<String>, out: &mut Changes) {
    if let Some(cursor) = json.get("historyId").and_then(Value::as_str) {
        out.cursor = cursor.to_owned();
    }
    for record in json.get("history").and_then(Value::as_array).into_iter().flatten() {
        let entries = |key: &str| {
            record
                .get(key)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|e| e.get("message"))
                .collect::<Vec<_>>()
        };
        for m in entries("messagesAdded") {
            let Some(id) = m.get("id").and_then(Value::as_str) else { continue };
            if is_filtered(&labels_of(m)) || out.removed.iter().any(|r| r == id) {
                continue;
            }
            if !out.added.iter().any(|a| a == id) {
                out.added.push(id.to_owned());
            }
        }
        for m in entries("labelsAdded").into_iter().chain(entries("labelsRemoved")) {
            let Some(id) = m.get("id").and_then(Value::as_str) else { continue };
            if out.removed.iter().any(|r| r == id) {
                continue;
            }
            let labels = labels_of(m);
            out.updated.retain(|(u, _)| u != id);
            if is_filtered(&labels) {
                // Moved to spam: gone from every view.
                out.added.retain(|a| a != id);
                out.removed.push(id.to_owned());
            } else {
                out.updated.push((id.to_owned(), flags(&labels, user_folders)));
            }
        }
        for m in entries("messagesDeleted") {
            let Some(id) = m.get("id").and_then(Value::as_str) else { continue };
            out.added.retain(|a| a != id);
            out.updated.retain(|(u, _)| u != id);
            if !out.removed.iter().any(|r| r == id) {
                out.removed.push(id.to_owned());
            }
        }
    }
}