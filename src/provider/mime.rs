//! Reading a raw RFC 2822 message back into its parts.
//!
//! One decoder for everything that has to look at stored MIME: a mock backend
//! serving its own drafts, and the imported-draft path that hands
//! [`ParsedMessage::html`] and [`ParsedMessage::text`] to the composer's rich
//! import. Pure, no HTTP, and deliberately non-destructive — it decodes
//! transfer encodings and encoded header words, and never rewrites, sanitises
//! or converts markup, so a body the app cannot represent survives the read.
//!
//! Wraps [`mailparse`]; this module is the only place that dependency is used
//! for reading.

use mailparse::{DispositionType, MailHeaderMap, MailParseError, ParsedMail, parse_mail};

/// A MIME message read back out of raw bytes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParsedMessage {
    /// Header values with RFC 2047 encoded words decoded and continuations
    /// folded, names lowercased, in document order.
    pub headers: Vec<(String, String)>,
    /// The text/plain alternative.
    pub text: String,
    /// The text/html alternative, verbatim, for a caller that must preserve
    /// styling it cannot represent.
    pub html: Option<String>,
    /// Every file part, with its name, type and decoded bytes.
    pub files: Vec<ParsedFile>,
}

/// One file part of a parsed message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedFile {
    pub name: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
    /// MIME `Content-ID` without angle brackets, for inline parts the body
    /// refers to with `cid:`.
    pub content_id: Option<String>,
}

/// Reads a raw RFC 2822 message: multipart structure, transfer encodings,
/// encoded header words and attachment payloads.
///
/// A part that cannot be decoded is an error, not an empty string: an imported
/// body that half-decodes is worse than one that refuses, because the caller
/// cannot tell the difference between "this message had no image" and "this
/// message's image was unreadable".
pub fn parse_message(raw: &[u8]) -> Result<ParsedMessage, MailParseError> {
    if raw.is_empty() {
        return Err(MailParseError::Generic("message bytes are empty"));
    }
    let message = parse_mail(raw)?;
    let mut out = ParsedMessage {
        headers: headers_of(&message),
        ..ParsedMessage::default()
    };
    // Depth first, the message itself first, so a single-part message reads
    // the same way a multipart alternative does.
    for part in message.parts() {
        // Only leaves carry content. A multipart container's own body is the
        // raw source of its children, so decoding one would put boundary text
        // where the message text belongs.
        if part.ctype.mimetype.starts_with("multipart/") {
            continue;
        }
        let disposition = part.get_content_disposition();
        let content_id = part
            .get_headers()
            .get_first_value("Content-ID")
            .map(|value| bare_message_id(&value))
            .filter(|id| !id.is_empty());
        let name = disposition
            .params
            .get("filename")
            .cloned()
            .filter(|n| !n.is_empty())
            .or_else(|| {
                part.ctype
                    .params
                    .get("name")
                    .cloned()
                    .filter(|n| !n.is_empty())
            })
            .or_else(|| {
                // An inline part the body refers to with `cid:` has no file
                // name and still has to be carried: dropping it would break the
                // image in an imported body. It gets a derived display name.
                content_id
                    .as_ref()
                    .map(|_| derived_name(&part.ctype.mimetype))
            });
        if let Some(name) = name.or_else(|| {
            // An unnamed attachment is still an attachment; call it by its type
            // rather than dropping it silently.
            (disposition.disposition == DispositionType::Attachment)
                .then(|| derived_name(&part.ctype.mimetype))
        }) {
            out.files.push(ParsedFile {
                name,
                content_type: part.ctype.mimetype.clone(),
                bytes: part.get_body_raw()?,
                content_id,
            });
            continue;
        }
        // Only a text part can be the body. Anything else without a name or a
        // content id is a binary the caller has no way to address, and it is
        // not text.
        match part.ctype.mimetype.as_str() {
            "text/html" => {
                if out.html.is_none() {
                    out.html = Some(part.get_body()?);
                }
            }
            // An unlabelled body is text; anything else binary is not.
            "text/plain" | "" if out.text.is_empty() => {
                out.text = part.get_body()?;
            }
            _ => {}
        }
    }
    Ok(out)
}

/// A stable display name for a part that has none: `image/png` becomes
/// `image.png`. Deterministic, so the same raw message always names the same
/// part the same way. A part that declared no type at all is still carried,
/// so it gets a name rather than an empty one that could not be stored.
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

/// The headers of a parsed message, decoded, in document order.
fn headers_of(message: &ParsedMail<'_>) -> Vec<(String, String)> {
    message
        .get_headers()
        .into_iter()
        .map(|header| (header.get_key().to_ascii_lowercase(), header.get_value()))
        .collect()
}

/// The value of one header of a parsed message.
pub fn header(parsed: &ParsedMessage, name: &str) -> Option<String> {
    parsed
        .headers
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.clone())
}

/// The value of one header with the angle brackets of a message id stripped.
pub fn bare_message_id(value: &str) -> String {
    let value = value.trim();
    // Only the surrounding pair is forgiven. A value with brackets of its own
    // is left as it is, so whatever validates an id downstream can reject it
    // instead of this quietly rewriting it into something the body no longer
    // refers to.
    value
        .strip_prefix('<')
        .and_then(|rest| rest.strip_suffix('>'))
        .unwrap_or(value)
        .to_owned()
}

/// The files a parsed message carries, as compose attachments.
pub fn attachments(parsed: &ParsedMessage) -> Vec<super::compose::RemoteAttachment> {
    parsed
        .files
        .iter()
        .enumerate()
        .map(|(i, file)| super::compose::RemoteAttachment {
            id: Some(format!("part-{i}")),
            content_id: file.content_id.clone(),
            name: file.name.clone(),
            size: file.bytes.len() as u64,
            content_type: file.content_type.clone(),
            inline_data: Some(file.bytes.clone()),
        })
        .collect()
}
