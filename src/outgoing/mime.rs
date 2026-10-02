//! RFC 5322 / MIME assembly for a composed message.
//!
//! The HTML part is exactly what the preview showed (it comes from
//! `DraftBody::render`), the plain-text part is its alternative, attachments
//! are base64 parts of the managed copies, and `Message-ID` is the draft's
//! persisted id so a lost response can be reconciled afterwards.

use std::io;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

use crate::clock::Timestamp;
use crate::model::AccountId;
use crate::reading::Recipient;

use super::attachment::ManagedAttachment;
use super::draft::{DraftRecord, split_addresses};

/// Everything needed to send one message. `message_id` is the RFC Message-ID
/// without angle brackets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutgoingMessage {
    pub account: AccountId,
    pub from: String,
    pub to: String,
    pub cc: String,
    pub bcc: String,
    pub subject: String,
    pub html: String,
    pub text: String,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub message_id: String,
    pub date: Timestamp,
}

impl OutgoingMessage {
    /// Build the message a draft describes. The recipient fields are checked
    /// but a recipient is not required: an unsaved-address draft still has to
    /// sync, preview and be recoverable. [`crate::outgoing::DraftRecord::validate`]
    /// is the send-time gate, and the worker calls it before holding or sending.
    pub fn from_draft(
        draft: &DraftRecord,
        account_email: &str,
        now: Timestamp,
    ) -> Result<Self, String> {
        draft.validate_recipient_fields()?;
        let rendered = draft.body.render();
        Ok(Self {
            account: draft.account.clone(),
            from: account_email.to_owned(),
            to: join(&draft.to),
            cc: join(&draft.cc),
            bcc: join(&draft.bcc),
            subject: draft.subject.clone(),
            html: rendered.html,
            text: rendered.text,
            in_reply_to: draft.in_reply_to.clone(),
            references: draft.references.clone(),
            message_id: draft.message_id.clone(),
            date: now,
        })
    }
}

/// Render a recipient field as header text: display names quoted when they
/// contain anything special, and never carrying a line break.
fn join(field: &str) -> String {
    split_addresses(field)
        .iter()
        .map(|one| format_address(one))
        .collect::<Vec<_>>()
        .join(", ")
}

fn format_address(typed: &str) -> String {
    let cleaned = strip_header_breaks(typed);
    let Some((name, email)) = cleaned.rsplit_once('<') else {
        return cleaned;
    };
    let email = email.trim_end_matches('>').trim();
    let name = name.trim().trim_matches('"').trim();
    if name.is_empty() {
        return email.to_owned();
    }
    // A non-ascii name becomes an encoded word, which carries no specials and
    // so needs no quoting; an ascii one is quoted when it has any.
    if name.is_ascii() {
        if name
            .chars()
            .any(|c| matches!(c, ',' | ';' | '"' | '<' | '>' | ':' | '\\'))
        {
            let escaped = name.replace('\\', "\\\\").replace('"', "\\\"");
            return format!("\"{escaped}\" <{email}>");
        }
        return format!("{name} <{email}>");
    }
    format!("{} <{email}>", encode_header_value(name))
}

/// An RFC 5322 msg-id: nothing that could end the angle brackets or the line.
fn clean_msg_id(raw: &str) -> Option<String> {
    let cleaned = strip_header_breaks(raw).trim().to_owned();
    let inner = cleaned
        .strip_prefix('<')
        .and_then(|v| v.strip_suffix('>'))
        .unwrap_or(&cleaned);
    if inner.is_empty()
        || inner.contains(['<', '>', ' ', '\t'])
        || inner.chars().any(|c| c.is_whitespace())
    {
        return None;
    }
    Some(inner.to_owned())
}

/// Header values are line-based: a CR or LF in anything a user or a provider
/// supplied is removed here, so no field can smuggle in a header of its own.
/// Folding is only ever the one this module generates.
fn strip_header_breaks(value: &str) -> String {
    value
        .chars()
        .filter(|c| *c != '\r' && *c != '\n' && !c.is_control())
        .collect()
}

/// Build the raw message. `boundary_seed` makes the boundaries deterministic
/// for a given attempt without needing a clock.
pub fn build_mime(
    message: &OutgoingMessage,
    attachments: &[ManagedAttachment],
    boundary_seed: u64,
) -> io::Result<Vec<u8>> {
    let mut out = String::with_capacity(1024 + message.html.len() + message.text.len());
    out.push_str(&header_block(message));

    // Inline parts the body points at with `cid:` live in a multipart/related
    // with the body; ordinary files sit beside it in a multipart/mixed. Clients
    // that render only a mixed's first part otherwise drop the images.
    let (inline, files): (Vec<&ManagedAttachment>, Vec<&ManagedAttachment>) =
        attachments.iter().partition(|a| a.content_id.is_some());

    let alt = boundary("alt", boundary_seed);
    let rel = boundary("rel", boundary_seed.wrapping_add(1));
    let mix = boundary("mixed", boundary_seed.wrapping_add(2));

    match (inline.is_empty(), files.is_empty()) {
        // Only the body: the message itself is the alternative.
        (true, true) => {
            out.push_str(&format!(
                "Content-Type: multipart/alternative; boundary=\"{alt}\"\r\n\r\n"
            ));
            body_part(&mut out, &alt, "text/plain", message.text.as_bytes());
            body_part(&mut out, &alt, "text/html", message.html.as_bytes());
            out.push_str(&format!("--{alt}--\r\n"));
        }
        // Only inline parts: they belong to the body that references them.
        (false, true) => {
            out.push_str(&format!(
                "Content-Type: multipart/related; boundary=\"{rel}\"\r\n\r\n"
            ));
            alternative_part(&mut out, &rel, &alt, message);
            for part in &inline {
                file_part(&mut out, &rel, part)?;
            }
            out.push_str(&format!("--{rel}--\r\n"));
        }
        // Ordinary files: the body group is the mixed's first part.
        (true, false) => {
            out.push_str(&format!(
                "Content-Type: multipart/mixed; boundary=\"{mix}\"\r\n\r\n"
            ));
            alternative_part(&mut out, &mix, &alt, message);
            for part in &files {
                file_part(&mut out, &mix, part)?;
            }
            out.push_str(&format!("--{mix}--\r\n"));
        }
        // Both: the related group travels as the mixed's first part, so a client
        // that renders only that part still sees the images.
        (false, false) => {
            out.push_str(&format!(
                "Content-Type: multipart/mixed; boundary=\"{mix}\"\r\n\r\n"
            ));
            out.push_str(&format!("--{mix}\r\n"));
            out.push_str(&format!(
                "Content-Type: multipart/related; boundary=\"{rel}\"\r\n\r\n"
            ));
            alternative_part(&mut out, &rel, &alt, message);
            for part in &inline {
                file_part(&mut out, &rel, part)?;
            }
            out.push_str(&format!("--{rel}--\r\n"));
            for part in &files {
                file_part(&mut out, &mix, part)?;
            }
            out.push_str(&format!("--{mix}--\r\n"));
        }
    }
    Ok(out.into_bytes())
}

/// The two alternatives as one complete part of `parent`: its own opening
/// delimiter on `parent`, its `multipart/alternative` header, both bodies, and
/// its closing delimiter. Every part this module writes carries its own
/// delimiter, so a nested part missing one leaves the body unreadable.
fn alternative_part(out: &mut String, parent: &str, boundary: &str, message: &OutgoingMessage) {
    out.push_str(&format!("--{parent}\r\n"));
    out.push_str(&format!(
        "Content-Type: multipart/alternative; boundary=\"{boundary}\"\r\n\r\n"
    ));
    body_part(out, boundary, "text/plain", message.text.as_bytes());
    body_part(out, boundary, "text/html", message.html.as_bytes());
    out.push_str(&format!("--{boundary}--\r\n"));
}

fn boundary(kind: &str, seed: u64) -> String {
    format!("----=_MailClassifier_{kind}_{seed:016x}")
}

/// The RFC 5322 line limit. Nothing this module writes may exceed it.
const MAX_LINE: usize = 998;

/// Write `name: value`, folding the value at whitespace so no line runs past
/// [`MAX_LINE`]. A continuation line starts with the space that was the break
/// point, so it is the RFC 5322 folding whitespace a reader unfolds, and
/// unfolding recovers the value exactly. A value with no break point longer
/// than the limit is left on one line: it is a single unbreakable token, and
/// splitting it would change it.
fn push_folded(out: &mut String, name: &str, value: &str) {
    out.push_str(name);
    out.push_str(": ");
    let mut line = name.len() + 2;
    let mut rest = value;
    loop {
        if line + rest.len() <= MAX_LINE {
            out.push_str(rest);
            out.push_str("\r\n");
            return;
        }
        let room = MAX_LINE.saturating_sub(line);
        // The last space that still fits: a break there is whitespace the
        // reader folds back out.
        let head = floor_boundary(rest, room.min(rest.len()));
        let Some(at) = rest[..head].rfind(' ').filter(|at| *at > 0) else {
            out.push_str(rest);
            out.push_str("\r\n");
            return;
        };
        out.push_str(&rest[..at]);
        // The break point's own space becomes the continuation's leading WSP;
        // it is consumed here rather than repeated from `rest`.
        out.push_str("\r\n ");
        rest = &rest[at + 1..];
        line = 1;
    }
}

/// The largest character boundary at or below `i`, so a fold can never slice a
/// character in half.
fn floor_boundary(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn header_block(message: &OutgoingMessage) -> String {
    let mut out = String::new();
    // An address field is already correctly formed by `format_address`: its
    // display names are encoded words and its list structure is real. Running
    // it through `encode_header_value` again would encode a non-ASCII
    // addr-spec as one blob, leaving no address for the recipient at all.
    let address = |out: &mut String, name: &str, value: &str| {
        let value = strip_header_breaks(value);
        if value.trim().is_empty() {
            return;
        }
        push_folded(out, name, value.trim());
    };
    // An unstructured field is the one that needs encoding.
    let text = |out: &mut String, name: &str, value: &str| {
        let value = strip_header_breaks(value);
        if value.trim().is_empty() {
            return;
        }
        // `encode_header_value` already folds a value that needed encoding.
        if value.is_ascii() {
            push_folded(out, name, value.trim());
        } else {
            out.push_str(name);
            out.push_str(": ");
            out.push_str(&encode_header_value(value.trim()));
            out.push_str("\r\n");
        }
    };
    address(&mut out, "From", &format_address(&message.from));
    address(&mut out, "To", &message.to);
    address(&mut out, "Cc", &message.cc);
    // Bcc has to travel with the message: the provider submits raw MIME.
    address(&mut out, "Bcc", &message.bcc);
    text(&mut out, "Subject", &message.subject);
    out.push_str(&format!("Date: {}\r\n", rfc2822_date(message.date)));
    if let Some(id) = clean_msg_id(&message.message_id) {
        out.push_str(&format!("Message-ID: <{id}>\r\n"));
    }
    if let Some(in_reply_to) = message.in_reply_to.as_deref().and_then(clean_msg_id) {
        out.push_str(&format!("In-Reply-To: <{in_reply_to}>\r\n"));
    }
    let references: Vec<String> = message
        .references
        .iter()
        .filter_map(|r| clean_msg_id(r))
        .map(|id| format!("<{id}>"))
        .collect();
    if !references.is_empty() {
        // A long thread is a long References, and it is a list of msg-ids:
        // folding between them is the one break that cannot change them.
        push_folded(&mut out, "References", &references.join(" "));
    }
    out.push_str("MIME-Version: 1.0\r\n");
    out
}

/// One body part, base64 so no line can run past the 998 character limit and
/// the bytes a recipient decodes are exactly the ones that were rendered.
fn body_part(out: &mut String, boundary: &str, content_type: &str, body: &[u8]) {
    out.push_str(&format!("--{boundary}\r\n"));
    out.push_str(&format!("Content-Type: {content_type}; charset=utf-8\r\n"));
    out.push_str("Content-Transfer-Encoding: base64\r\n\r\n");
    out.push_str(&base64_lines(body));
}

/// A managed copy as a part: inline under its `cid:` when the body references
/// it, an attachment otherwise.
fn file_part(out: &mut String, boundary: &str, attachment: &ManagedAttachment) -> io::Result<()> {
    out.push_str(&format!("--{boundary}\r\n"));
    let name = &attachment.file_name;
    out.push_str(&format!(
        "Content-Type: {}; {}\r\n",
        safe_content_type(&attachment.content_type),
        file_name_params(name, "name")
    ));
    match &attachment.content_id {
        Some(cid) => {
            // The id comes from parsed provider metadata, so it is checked here
            // as well: a bad one must never reach a header.
            if !super::attachment::valid_content_id(cid) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("{cid:?} is not a usable Content-ID"),
                ));
            }
            out.push_str(&format!(
                "Content-Disposition: inline; {}\r\n",
                file_name_params(name, "filename")
            ));
            out.push_str(&format!("Content-ID: <{cid}>\r\n"));
        }
        None => out.push_str(&format!(
            "Content-Disposition: attachment; {}\r\n",
            file_name_params(name, "filename")
        )),
    }
    out.push_str("Content-Transfer-Encoding: base64\r\n\r\n");
    out.push_str(&base64_lines(&attachment.read()?));
    Ok(())
}

fn base64_lines(bytes: &[u8]) -> String {
    let encoded = BASE64.encode(bytes);
    let mut out = String::with_capacity(encoded.len() + encoded.len() / 76 + 2);
    for chunk in encoded.as_bytes().chunks(76) {
        out.push_str(std::str::from_utf8(chunk).expect("base64 is ascii"));
        out.push_str("\r\n");
    }
    out
}

/// `name="a.txt"` (or `filename=`), with quotes and backslashes escaped; a
/// non-ASCII name also gets the RFC 2231 form, which is what clients read.
fn file_name_params(name: &str, key: &str) -> String {
    let name = strip_header_breaks(name);
    if name.is_ascii() {
        let escaped = name.replace('\\', "\\\\").replace('"', "\\\"");
        return format!("{key}=\"{escaped}\"");
    }
    format!(
        "{key}=\"{}\"; filename*=UTF-8''{}",
        encode_header_value(&name),
        encode_2231(&name)
    )
}

/// Percent-encode everything outside the RFC 5987 attribute character set.
fn encode_2231(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        let c = *byte as char;
        if c.is_ascii_alphanumeric() || "!#$&+-.^_`|~".contains(c) {
            out.push(c);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// A content type is a token; anything else in it is dropped rather than
/// trusted, since it lands in a header.
fn safe_content_type(content_type: &str) -> String {
    let cleaned = strip_header_breaks(content_type).trim().to_owned();
    let ok = !cleaned.is_empty()
        && cleaned.split('/').count() == 2
        && cleaned
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/!#$&^_.+-".contains(c));
    if ok {
        cleaned
    } else {
        "application/octet-stream".to_owned()
    }
}

/// RFC 2047 encoded word, or the value unchanged when it is plain ASCII.
/// Words are cut on character boundaries and stay inside the 75 character
/// limit, joined by the one kind of folding this module emits.
pub fn encode_header_value(value: &str) -> String {
    let value = strip_header_breaks(value);
    if value.is_ascii() {
        return value;
    }
    // 45 raw bytes are 60 base64 characters: 10 + 60 + 2 = 72, under the limit.
    const CHUNK: usize = 45;
    let mut words = Vec::new();
    let mut chunk = String::new();
    let mut bytes = 0usize;
    for c in value.chars() {
        if bytes + c.len_utf8() > CHUNK && !chunk.is_empty() {
            words.push(encoded_word(&chunk));
            chunk.clear();
            bytes = 0;
        }
        chunk.push(c);
        bytes += c.len_utf8();
    }
    if !chunk.is_empty() || words.is_empty() {
        words.push(encoded_word(&chunk));
    }
    words.join("\r\n ")
}

fn encoded_word(text: &str) -> String {
    format!("=?UTF-8?B?{}?=", BASE64.encode(text))
}

/// `Name <a@b>, c@d` from parsed recipients.
pub fn format_address_list(recipients: &[Recipient]) -> String {
    recipients
        .iter()
        .map(
            |r| match r.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
                Some(name) => format_address(&format!("{name} <{}>", r.email)),
                None => strip_header_breaks(&r.email),
            },
        )
        .collect::<Vec<_>>()
        .join(", ")
}

/// `Thu, 02 Oct 2026 05:13:25 +0000` for a UTC timestamp.
pub fn rfc2822_date(timestamp: Timestamp) -> String {
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let days = timestamp.div_euclid(86_400);
    let seconds = timestamp.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let weekday = (days + 4).rem_euclid(7) as usize;
    format!(
        "{}, {:02} {} {} {:02}:{:02}:{:02} +0000",
        DAYS[weekday],
        day,
        MONTHS[(month - 1) as usize],
        year,
        seconds / 3600,
        (seconds % 3600) / 60,
        seconds % 60
    )
}

/// Days since 1970-01-01 to a proleptic Gregorian date.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = (z - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outgoing::attachment::{AttachmentSource, AttachmentStore};
    use crate::outgoing::body::DraftBody;
    use crate::outgoing::draft::DraftRecord;

    fn draft() -> DraftRecord {
        let mut draft = DraftRecord::new_email("me@x.test", 100);
        draft.to = "Sam Smith <sam@x.test>, kim@x.test".to_owned();
        draft.cc = "lee@x.test".to_owned();
        draft.subject = "Lunch".to_owned();
        draft.body = DraftBody::markdown("**hi**\n\n- a");
        draft.message_id = "abc123@x.test".to_owned();
        draft
    }

    fn attachment(tag: &str) -> ManagedAttachment {
        let dir =
            std::env::temp_dir().join(format!("mail-classifier-mime-{tag}-{}", std::process::id()));
        AttachmentStore::open(&dir)
            .expect("store")
            .import_bytes(
                "notes.txt",
                tag.as_bytes(),
                AttachmentSource::Generated,
                None,
            )
            .expect("import")
    }

    fn message() -> OutgoingMessage {
        OutgoingMessage::from_draft(&draft(), "me@x.test", 1_767_225_600).expect("message")
    }

    /// Every body part of a multipart message, decoded, in order.
    fn decoded_bodies(raw: &str, boundary: &str) -> Vec<String> {
        raw.split(&format!("--{boundary}\r\n"))
            .filter_map(|part| {
                let (headers, encoded) = part.split_once("\r\n\r\n")?;
                if !headers.contains("Content-Type: text/") {
                    return None;
                }
                let payload = encoded.split("\r\n--").next().unwrap_or(encoded);
                Some(
                    String::from_utf8(
                        BASE64
                            .decode(payload.replace(['\r', '\n'], "").trim())
                            .expect("base64"),
                    )
                    .expect("utf-8 body"),
                )
            })
            .collect()
    }

    fn boundary_of(raw: &str, kind: &str) -> String {
        raw.split("\r\n")
            .find_map(|line| {
                line.strip_prefix(&format!("Content-Type: multipart/{kind}; boundary=\""))
            })
            .map(|b| b.trim_end_matches('"').to_owned())
            .unwrap_or_else(|| panic!("a {kind} boundary"))
    }

    #[test]
    fn the_sent_bodies_are_exactly_what_the_body_rendered() {
        let draft = draft();
        let message = OutgoingMessage::from_draft(&draft, "me@x.test", 1).expect("message");
        assert_eq!(
            message.html,
            crate::outgoing::markdown::render_markdown(&draft.body.source)
        );
        let raw = String::from_utf8(build_mime(&message, &[], 3).expect("mime")).expect("utf8");
        let bodies = decoded_bodies(&raw, &boundary_of(&raw, "alternative"));
        assert_eq!(bodies, vec![message.text, message.html]);
    }

    #[test]
    fn an_unaddressed_draft_still_builds_a_message() {
        let mut draft = DraftRecord::new_email("me@x.test", 1);
        draft.body = DraftBody::markdown("saving as I go");
        let message = OutgoingMessage::from_draft(&draft, "me@x.test", 1)
            .expect("a draft with no recipient yet still saves");
        assert_eq!(message.to, "");
        let raw = String::from_utf8(build_mime(&message, &[], 1).expect("mime")).expect("utf8");
        // The bodies travel base64, so they are checked as what a reader
        // decodes out of the finished message.
        let bodies = decoded_bodies(&raw, &boundary_of(&raw, "alternative"));
        assert_eq!(
            bodies,
            vec![message.text.clone(), message.html.clone()],
            "{raw}"
        );

        // Sending it is refused: that gate is the record's own validation.
        assert!(draft.validate().is_err());
        assert!(draft.to.is_empty());
    }

    #[test]
    fn a_malformed_recipient_still_refuses_to_build() {
        let mut draft = DraftRecord::new_email("me@x.test", 1);
        draft.to = "not-an-address".to_owned();
        assert!(OutgoingMessage::from_draft(&draft, "me@x.test", 1).is_err());
    }

    #[test]
    fn headers_carry_threading_and_the_persisted_message_id() {
        let mut draft = draft();
        draft.in_reply_to = Some("orig@x.test".to_owned());
        draft.references = vec!["older@x.test".to_owned(), "orig@x.test".to_owned()];
        let message = OutgoingMessage::from_draft(&draft, "me@x.test", 1_767_225_600).expect("msg");
        let raw = String::from_utf8(build_mime(&message, &[], 7).expect("mime")).expect("utf8");

        assert!(raw.starts_with("From: me@x.test\r\n"), "{raw}");
        assert!(
            raw.contains("To: Sam Smith <sam@x.test>, kim@x.test\r\n"),
            "{raw}"
        );
        assert!(raw.contains("Message-ID: <abc123@x.test>\r\n"), "{raw}");
        assert!(raw.contains("In-Reply-To: <orig@x.test>\r\n"), "{raw}");
        assert!(
            raw.contains("References: <older@x.test> <orig@x.test>\r\n"),
            "{raw}"
        );
        assert!(
            raw.contains("Date: Thu, 01 Jan 2026 00:00:00 +0000\r\n"),
            "{raw}"
        );
    }

    #[test]
    fn bcc_travels_with_the_message_and_empty_fields_are_dropped() {
        let mut draft = draft();
        draft.cc = String::new();
        draft.bcc = "secret@x.test".to_owned();
        let message = OutgoingMessage::from_draft(&draft, "me@x.test", 1).expect("msg");
        let raw = String::from_utf8(build_mime(&message, &[], 1).expect("mime")).expect("utf8");
        assert!(raw.contains("Bcc: secret@x.test\r\n"), "{raw}");
        assert!(!raw.contains("\r\nCc:"), "{raw}");
    }

    #[test]
    fn no_field_can_smuggle_in_a_header() {
        let mut draft = draft();
        draft.subject = "Lunch\r\nBcc: attacker@evil.test".to_owned();
        draft.in_reply_to = Some("orig@x.test\r\nX-Evil: 1".to_owned());
        let message = OutgoingMessage::from_draft(&draft, "me@x.test", 1).expect("msg");
        let raw = String::from_utf8(build_mime(&message, &[], 1).expect("mime")).expect("utf8");
        assert!(
            !raw.split("\r\n").any(|line| line.starts_with("X-Evil")),
            "{raw}"
        );
        assert!(
            !raw.split("\r\n").any(|line| line.starts_with("Bcc:")),
            "{raw}"
        );
        assert!(
            raw.contains("Subject: LunchBcc: attacker@evil.test\r\n"),
            "{raw}"
        );
        // A reply target that is not a msg-id is dropped, not stitched together.
        assert!(!raw.contains("In-Reply-To"), "{raw}");

        // A recipient carrying a line break never becomes a header at all.
        draft.to = "sam@x.test\r\nX-Evil: 1".to_owned();
        assert!(OutgoingMessage::from_draft(&draft, "me@x.test", 1).is_err());
    }

    #[test]
    fn display_names_are_quoted_and_encoded() {
        let mut draft = draft();
        draft.to = "\"Doe, Jane\" <jane@x.test>, Kim Café <kim@x.test>".to_owned();
        let message = OutgoingMessage::from_draft(&draft, "me@x.test", 1).expect("msg");
        let raw = String::from_utf8(build_mime(&message, &[], 1).expect("mime")).expect("utf8");
        assert!(raw.contains(r#"To: "Doe, Jane" <jane@x.test>"#), "{raw}");
        assert!(
            raw.contains("=?UTF-8?B?"),
            "non-ascii names are encoded: {raw}"
        );
    }

    #[test]
    fn a_non_ascii_addr_spec_keeps_its_address_list() {
        // A non-ASCII display name is already an encoded word by the time the
        // field is written, but a non-ASCII *address* is not: encoding the
        // whole field would turn the list into one blob and leave the
        // recipient with no address at all.
        let mut draft = draft();
        draft.to = "renee@x.test, josé@x.test".to_owned();
        draft.cc = "lee@x.test".to_owned();
        let message = OutgoingMessage::from_draft(&draft, "me@x.test", 1).expect("msg");
        let raw = String::from_utf8(build_mime(&message, &[], 1).expect("mime")).expect("utf8");
        let header = raw
            .split("\r\n")
            .find_map(|line| line.strip_prefix("To: "))
            .expect("a To header");
        assert!(
            !header.starts_with("=?"),
            "the list became one encoded word: {header}"
        );
        assert!(
            !header.contains("=?UTF-8?B?"),
            "an address must not be encoded: {header}"
        );
        // The field still parses as the addresses it names.
        let recipients = crate::reading::parse_recipients(header);
        assert_eq!(
            recipients
                .iter()
                .map(|r| r.email.as_str())
                .collect::<Vec<_>>(),
            vec!["renee@x.test", "josé@x.test"],
            "{header}"
        );
        assert!(
            raw.contains("\r\nCc: lee@x.test\r\n"),
            "Cc was consumed too: {raw}"
        );
    }

    #[test]
    fn a_non_ascii_display_name_is_still_encoded() {
        let mut draft = draft();
        draft.to = "José <jose@x.test>".to_owned();
        let message = OutgoingMessage::from_draft(&draft, "me@x.test", 1).expect("msg");
        let raw = String::from_utf8(build_mime(&message, &[], 1).expect("mime")).expect("utf8");
        assert!(raw.contains("=?UTF-8?B?"), "the name is not encoded: {raw}");
        assert!(raw.contains("jose@x.test>"), "the address is gone: {raw}");
    }

    #[test]
    fn an_ascii_address_field_is_left_alone() {
        let message = message();
        let raw = String::from_utf8(build_mime(&message, &[], 1).expect("mime")).expect("utf8");
        assert!(
            raw.contains("To: Sam Smith <sam@x.test>, kim@x.test\r\n"),
            "{raw}"
        );
    }

    #[test]
    fn a_from_address_is_a_real_address() {
        let draft = draft();
        let message = OutgoingMessage::from_draft(&draft, "José <me@x.test>", 1).expect("msg");
        let raw = String::from_utf8(build_mime(&message, &[], 1).expect("mime")).expect("utf8");
        let header = raw
            .split("\r\n")
            .find_map(|line| line.strip_prefix("From: "))
            .expect("a From header");
        assert!(
            header.contains("me@x.test>"),
            "the account address is gone: {header}"
        );
        assert!(
            header.contains("=?UTF-8?B?"),
            "the name is not encoded: {header}"
        );
    }

    #[test]
    fn a_long_thread_folds_its_references_instead_of_overrunning_the_line_limit() {
        let mut draft = draft();
        draft.references = (1..=40)
            .map(|i| format!("msg{i:04}abcdef0123456789@thread.x.test"))
            .collect();
        let message = OutgoingMessage::from_draft(&draft, "me@x.test", 1).expect("msg");
        let raw = String::from_utf8(build_mime(&message, &[], 4).expect("mime")).expect("utf8");

        assert!(
            raw.split("\r\n").all(|line| line.len() <= MAX_LINE),
            "a line ran past the limit"
        );
        // Folding is only ever the whitespace this module inserts, so
        // unfolding the field — its own line plus every continuation — has to
        // give back exactly the msg-ids, in order.
        let mut lines = raw
            .split("\r\n")
            .skip_while(|l| !l.starts_with("References: "));
        let first = lines
            .next()
            .and_then(|l| l.strip_prefix("References: "))
            .expect("a References field");
        // A continuation is a line whose first character is the folding
        // whitespace; anything else is the next header or the body.
        let unfolded =
            lines
                .take_while(|l| l.starts_with(' '))
                .fold(first.to_owned(), |mut acc, l| {
                    acc.push(' ');
                    acc.push_str(l.trim_start());
                    acc
                });
        assert!(
            raw.split("\r\n")
                .filter(|l| l.contains("@thread.x.test"))
                .count()
                > 1,
            "the references were not folded at all"
        );
        let expected: Vec<String> = draft
            .references
            .iter()
            .map(|id| format!("<{id}>"))
            .collect();
        assert_eq!(unfolded, expected.join(" "));
    }

    #[test]
    fn a_value_with_no_break_point_is_not_split() {
        // There is no whitespace to fold at, and inserting one would change
        // the value, so the token is left whole.
        let mut draft = draft();
        draft.subject = "a".repeat(1200);
        let message = OutgoingMessage::from_draft(&draft, "me@x.test", 1).expect("msg");
        let raw = String::from_utf8(build_mime(&message, &[], 4).expect("mime")).expect("utf8");
        assert!(raw.contains(&"a".repeat(1200)), "the subject was broken up");
    }

    #[test]
    fn a_short_header_is_written_on_one_line() {
        let mut draft = draft();
        draft.references = vec!["older@x.test".to_owned(), "orig@x.test".to_owned()];
        let message = OutgoingMessage::from_draft(&draft, "me@x.test", 1).expect("msg");
        let raw = String::from_utf8(build_mime(&message, &[], 4).expect("mime")).expect("utf8");
        assert!(
            raw.contains("References: <older@x.test> <orig@x.test>\r\n"),
            "{raw}"
        );
    }

    #[test]
    fn attachments_become_base64_parts_inside_mixed() {
        let message = message();
        let attachment = attachment("hello");
        let raw = String::from_utf8(
            build_mime(&message, std::slice::from_ref(&attachment), 5).expect("mime"),
        )
        .expect("utf8");
        assert!(raw.contains("Content-Type: multipart/mixed"), "{raw}");
        assert!(raw.contains("Content-Disposition: attachment; "), "{raw}");
        assert!(raw.contains("name=\"notes.txt\""), "{raw}");
        let outer = boundary_of(&raw, "mixed");
        let part = raw
            .split(&format!("--{outer}\r\n"))
            .find(|p| p.contains("Content-Disposition"))
            .expect("attachment part");
        let (_, encoded) = part.split_once("\r\n\r\n").expect("part headers");
        let payload = encoded.split("\r\n--").next().unwrap_or(encoded);
        let bytes = BASE64
            .decode(payload.replace(['\r', '\n'], "").trim())
            .expect("base64");
        assert_eq!(bytes, attachment.read().expect("stored bytes"));
    }

    #[test]
    fn every_line_of_the_message_fits_a_mail_line() {
        let dir =
            std::env::temp_dir().join(format!("mail-classifier-mime-big-{}", std::process::id()));
        let store = AttachmentStore::open(&dir).expect("store");
        let attachment = store
            .import_bytes(
                "big.bin",
                &vec![7u8; 4096],
                AttachmentSource::Generated,
                None,
            )
            .expect("import");
        let mut draft = draft();
        draft.body = DraftBody::markdown("a fairly long line of text. ".repeat(40));
        let message = OutgoingMessage::from_draft(&draft, "me@x.test", 1).expect("msg");
        let raw =
            String::from_utf8(build_mime(&message, &[attachment], 9).expect("mime")).expect("utf8");
        for line in raw.split("\r\n") {
            assert!(line.len() <= 998, "line of {} characters", line.len());
        }
    }

    #[test]
    fn attachment_names_are_escaped_for_the_quoted_form() {
        assert_eq!(file_name_params("plain.txt", "name"), r#"name="plain.txt""#);
        assert_eq!(
            file_name_params(r#"we"ird\odd.txt"#, "filename"),
            r#"filename="we\"ird\\odd.txt""#
        );
        assert!(!file_name_params("a\r\nb.txt", "name").contains('\r'));
    }

    #[test]
    fn non_ascii_attachment_names_carry_the_rfc2231_form() {
        let dir =
            std::env::temp_dir().join(format!("mail-classifier-mime-utf8-{}", std::process::id()));
        let store = AttachmentStore::open(&dir).expect("store");
        let attachment = store
            .import_bytes(
                "rapport-été.pdf",
                b"%PDF",
                AttachmentSource::Generated,
                None,
            )
            .expect("import");
        let raw = String::from_utf8(build_mime(&message(), &[attachment], 2).expect("mime"))
            .expect("utf8");
        assert!(
            raw.contains("filename*=UTF-8''rapport-%C3%A9t%C3%A9.pdf"),
            "{raw}"
        );
    }

    #[test]
    fn a_content_type_that_is_not_a_token_is_replaced() {
        assert_eq!(safe_content_type("text/plain"), "text/plain");
        assert_eq!(safe_content_type("evil"), "application/octet-stream");
        assert_eq!(
            safe_content_type("text/plain; x"),
            "application/octet-stream"
        );
        assert_eq!(safe_content_type("image/png"), "image/png");
        assert_eq!(
            safe_content_type("text/plain\r\nX-Evil: 1"),
            "application/octet-stream"
        );
    }

    #[test]
    fn encoded_words_split_on_character_boundaries() {
        let subject = "Café ".repeat(60) + "☕";
        let encoded = encode_header_value(&subject);
        let mut decoded = Vec::new();
        for word in encoded.split("\r\n ") {
            assert!(
                word.len() <= 75,
                "encoded word of {} characters",
                word.len()
            );
            let payload = word
                .strip_prefix("=?UTF-8?B?")
                .and_then(|w| w.strip_suffix("?="))
                .unwrap_or_else(|| panic!("an encoded word: {word}"));
            let bytes = BASE64.decode(payload).expect("base64");
            assert!(
                String::from_utf8(bytes.clone()).is_ok(),
                "a word split a character"
            );
            decoded.extend(bytes);
        }
        assert_eq!(String::from_utf8(decoded).expect("utf-8"), subject);
    }

    #[test]
    fn ascii_header_values_stay_readable() {
        assert_eq!(encode_header_value("Lunch?"), "Lunch?");
    }

    #[test]
    fn rfc2822_dates_are_right() {
        assert_eq!(rfc2822_date(0), "Thu, 01 Jan 1970 00:00:00 +0000");
        assert_eq!(
            rfc2822_date(1_767_225_600),
            "Thu, 01 Jan 2026 00:00:00 +0000"
        );
        assert_eq!(
            rfc2822_date(1_709_164_800),
            "Thu, 29 Feb 2024 00:00:00 +0000"
        );
    }

    #[test]
    fn address_lists_are_formatted_from_recipients() {
        let recipients = crate::reading::parse_recipients("Sam Smith <sam@x.test>, kim@x.test");
        assert_eq!(
            format_address_list(&recipients),
            "Sam Smith <sam@x.test>, kim@x.test"
        );
    }

    /// Where each interesting part sits, as a list of its ancestor mime types,
    /// read back through a real MIME parser.
    fn part_paths(raw: &[u8]) -> Vec<(Vec<String>, String, Option<String>)> {
        use mailparse::{MailHeaderMap, ParsedMail, parse_mail};
        fn walk(
            part: &ParsedMail<'_>,
            ancestors: &[String],
            out: &mut Vec<(Vec<String>, String, Option<String>)>,
        ) {
            let path: Vec<String> = ancestors
                .iter()
                .map(|a| a.replace("multipart/", ""))
                .collect();
            let mime = part.ctype.mimetype.to_string();
            let id = part
                .get_headers()
                .get_first_value("content-id")
                .map(|value| value.trim_matches(['<', '>']).to_owned());
            if part.subparts.is_empty() {
                out.push((path, mime, id));
                return;
            }
            for child in &part.subparts {
                let mut next = ancestors.to_vec();
                next.push(mime.clone());
                walk(child, &next, out);
            }
        }
        let mail = parse_mail(raw).expect("the message parses");
        let mut out = Vec::new();
        walk(&mail, &[], &mut out);
        out
    }

    fn cid_part(tag: &str) -> ManagedAttachment {
        let dir =
            std::env::temp_dir().join(format!("mail-classifier-mime-{tag}-{}", std::process::id()));
        AttachmentStore::open(&dir)
            .expect("store")
            .import_bytes("logo.png", b"\x89PNG", AttachmentSource::Generated, None)
            .expect("import")
            .with_content_id(Some("logo@x.test".to_owned()))
            .expect("a plain id is accepted")
    }

    #[test]
    fn an_inline_image_shares_a_related_root_with_the_body() {
        let mut draft = draft();
        draft.body = DraftBody::markdown("![logo](cid:logo@x.test)");
        let message = OutgoingMessage::from_draft(&draft, "me@x.test", 1).expect("msg");
        let logo = cid_part("cid");
        let raw = build_mime(&message, std::slice::from_ref(&logo), 4).expect("mime");

        assert!(
            String::from_utf8_lossy(&raw).contains("Content-ID: <logo@x.test>"),
            "no content id"
        );
        let parts = part_paths(&raw);
        let html = parts
            .iter()
            .find(|(_, mime, _)| mime == "text/html")
            .expect("the html part");
        let image = parts
            .iter()
            .find(|(_, _, id)| id.as_deref() == Some("logo@x.test"))
            .expect("the inline image");
        assert_eq!(html.0, vec!["related", "alternative"], "html under related");
        assert_eq!(
            image.0,
            vec!["related"],
            "the image is its sibling resource"
        );
        assert_eq!(image.2.as_deref(), Some("logo@x.test"));

        // The reference the body makes is still there to resolve. It travels
        // base64, so it is checked as what a reader decodes.
        let (_, html, _) = read_back(&raw);
        assert_eq!(html.as_deref(), Some(message.html.as_str()), "{html:?}");
        assert!(message.html.contains("cid:logo@x.test"), "{}", message.html);
        assert_eq!(logo.content_id.as_deref(), Some("logo@x.test"));
    }

    #[test]
    fn ordinary_files_sit_beside_the_related_group() {
        let mut draft = draft();
        draft.body = DraftBody::markdown("![logo](cid:logo@x.test)");
        let message = OutgoingMessage::from_draft(&draft, "me@x.test", 1).expect("msg");
        let logo = cid_part("cid-mixed");
        let file = attachment("hello");
        let raw = build_mime(&message, &[logo, file.clone()], 8).expect("mime");

        let parts = part_paths(&raw);
        let html = parts
            .iter()
            .find(|(_, mime, _)| mime == "text/html")
            .expect("html");
        let image = parts
            .iter()
            .find(|(_, _, id)| id.as_deref() == Some("logo@x.test"))
            .expect("image");
        // The ordinary file is the part sitting directly under the mixed — a
        // `notes.txt` is a text/plain part, so it is told apart from the body
        // by where it sits, not by its type, and by carrying no Content-ID.
        let ordinary = parts
            .iter()
            .find(|(path, _, id)| path.len() == 1 && id.is_none())
            .expect("the ordinary file");
        assert_eq!(html.0, vec!["mixed", "related", "alternative"]);
        assert_eq!(image.0, vec!["mixed", "related"]);
        assert_eq!(
            ordinary.0,
            vec!["mixed"],
            "the file is a sibling of the related group"
        );
        assert!(ordinary.2.is_none(), "an ordinary file has no content id");
        assert_eq!(file.content_id, None);
    }

    #[test]
    fn an_ordinary_attachment_is_not_given_a_content_id() {
        let attachment = attachment("hello");
        assert!(attachment.content_id.is_none());
        let raw = String::from_utf8(
            build_mime(&message(), std::slice::from_ref(&attachment), 6).expect("mime"),
        )
        .expect("utf8");
        assert!(raw.contains("Content-Disposition: attachment; "), "{raw}");
        assert!(!raw.contains("Content-ID"), "{raw}");
    }

    #[test]
    fn a_content_id_that_could_break_the_header_is_refused() {
        let mut draft = draft();
        draft.body = DraftBody::markdown("![logo](cid:logo@x.test)");
        let message = OutgoingMessage::from_draft(&draft, "me@x.test", 1).expect("msg");
        let logo = cid_part("cid-bad");

        for bad in [
            "logo@x.test\r\nX-Evil: 1",
            "logo@x.test\nBcc: a@b.test",
            "in<side>@x.test",
        ] {
            assert!(
                logo.clone().with_content_id(Some(bad.to_owned())).is_err(),
                "{bad:?} must not be accepted as an id"
            );
            // A part that reached us another way (older store, hand-edited
            // draft) must not be written into a header either.
            let mut tampered = logo.clone();
            tampered.content_id = Some(bad.to_owned());
            let err = build_mime(&message, std::slice::from_ref(&tampered), 2)
                .expect_err("an unusable Content-ID is refused");
            assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "{bad:?}");
        }

        assert!(build_mime(&message, std::slice::from_ref(&logo), 2).is_ok());
    }

    type DecodedFile = (Option<String>, Vec<u8>);
    /// What a MIME reader makes of the message: the plain body, the HTML body,
    /// and every file part with its decoded bytes.
    fn read_back(raw: &[u8]) -> (String, Option<String>, Vec<DecodedFile>) {
        use mailparse::{MailHeaderMap, ParsedMail, parse_mail};
        fn walk(
            part: &ParsedMail<'_>,
            text: &mut String,
            html: &mut Option<String>,
            files: &mut Vec<DecodedFile>,
        ) {
            let mime = part.ctype.mimetype.as_str();
            if part.subparts.is_empty() {
                let bytes = part.get_body_raw().expect("the transfer encoding decodes");
                let name = part
                    .get_content_disposition()
                    .params
                    .get("filename")
                    .cloned()
                    .or_else(|| part.ctype.params.get("name").cloned());
                if name.is_some() || part.get_headers().get_first_value("content-id").is_some() {
                    files.push((name, bytes));
                } else {
                    match mime {
                        "text/plain" => *text = String::from_utf8(bytes).expect("utf-8 text"),
                        "text/html" => *html = Some(String::from_utf8(bytes).expect("utf-8 html")),
                        _ => files.push((None, bytes)),
                    }
                }
                return;
            }
            for child in &part.subparts {
                walk(child, text, html, files);
            }
        }
        let mail = parse_mail(raw).expect("the message parses");
        let (mut text, mut html, mut files) = (String::new(), None, Vec::new());
        walk(&mail, &mut text, &mut html, &mut files);
        (text, html, files)
    }

    /// The body a recipient reads back must be the body that was composed,
    /// whatever the message also carries.
    fn assert_body_reads_back(raw: &[u8], message: &OutgoingMessage) {
        let (text, html, _) = read_back(raw);
        assert_eq!(
            text, message.text,
            "the plain part must survive the round trip"
        );
        assert_eq!(
            html.as_deref(),
            Some(message.html.as_str()),
            "the html part must survive"
        );
    }

    #[test]
    fn the_body_reads_back_with_a_plain_attachment() {
        let message = message();
        let file = attachment("hello");
        let raw = build_mime(&message, std::slice::from_ref(&file), 11).expect("mime");
        assert_body_reads_back(&raw, &message);

        let (_, _, files) = read_back(&raw);
        assert_eq!(
            files,
            vec![(Some("notes.txt".to_owned()), file.read().expect("bytes"))]
        );
    }

    #[test]
    fn the_body_reads_back_with_an_inline_image() {
        let mut draft = draft();
        draft.body = DraftBody::markdown("![logo](cid:logo@x.test)");
        let message = OutgoingMessage::from_draft(&draft, "me@x.test", 1).expect("msg");
        let logo = cid_part("cid-body");
        let raw = build_mime(&message, std::slice::from_ref(&logo), 12).expect("mime");
        assert_body_reads_back(&raw, &message);

        let (_, _, files) = read_back(&raw);
        assert_eq!(
            files,
            vec![(Some("logo.png".to_owned()), logo.read().expect("bytes"))]
        );
    }

    #[test]
    fn the_body_reads_back_with_an_inline_image_and_a_file() {
        let mut draft = draft();
        draft.body = DraftBody::markdown("![logo](cid:logo@x.test)");
        let message = OutgoingMessage::from_draft(&draft, "me@x.test", 1).expect("msg");
        let logo = cid_part("cid-both");
        let file = attachment("hello");
        let raw = build_mime(&message, &[logo.clone(), file.clone()], 13).expect("mime");
        assert_body_reads_back(&raw, &message);

        let (_, _, files) = read_back(&raw);
        assert_eq!(files.len(), 2, "{files:?}");
        assert!(files.iter().any(|(name, bytes)| {
            name.as_deref() == Some("logo.png") && *bytes == logo.read().expect("bytes")
        }));
        assert!(files.iter().any(|(name, bytes)| {
            name.as_deref() == Some("notes.txt") && *bytes == file.read().expect("bytes")
        }));
    }

    #[test]
    fn the_body_reads_back_on_its_own() {
        let message = message();
        let raw = build_mime(&message, &[], 14).expect("mime");
        assert_body_reads_back(&raw, &message);
    }

    #[test]
    fn an_imported_rich_body_and_its_parts_read_back_unchanged() {
        // The whole fidelity claim in one message: a body imported as rich HTML
        // that has not been approved for conversion must go out exactly as it
        // came in, with every part's bytes, declared type and inline id intact,
        // so a forwarded or edited-away-from draft can be handed on unchanged.
        let store = AttachmentStore::open(
            &std::env::temp_dir().join(format!("mail-classifier-mime-rich-{}", std::process::id())),
        )
        .expect("store");
        let image = store
            .import_bytes(
                "logo",
                b"\x89PNG\r\n\x1a\n",
                AttachmentSource::Generated,
                Some("image/png"),
            )
            .expect("import")
            .with_content_id(Some("logo@x.test".to_owned()))
            .expect("inline");
        // Extensionless and named against its type: the declaration is what
        // the recipient's client must see, not the absent extension.
        let report = store
            .import_bytes(
                "scan",
                b"%PDF-1.4\n",
                AttachmentSource::Generated,
                Some("application/pdf"),
            )
            .expect("import");

        let mut record = draft();
        let rich = "<div style=\"color:red\"><b>kept</b></div>";
        record.body = DraftBody::import_rich(rich, "");
        assert!(
            record.body.needs_conversion(),
            "the rich body is still undecided"
        );
        let message =
            OutgoingMessage::from_draft(&record, "me@x.test", 1_767_225_600).expect("msg");
        assert_eq!(
            message.html, rich,
            "the preview and the send must be the same HTML"
        );

        let raw = build_mime(&message, &[image.clone(), report.clone()], 21).expect("mime");
        let parsed = crate::provider::mime::parse_message(&raw).expect("the message reads back");

        // Untouched rich HTML survives byte for byte; no Markdown conversion
        // was smuggled in behind the user's back.
        assert_eq!(parsed.html.as_deref(), Some(rich));
        assert_eq!(parsed.files.len(), 2, "every part is still carried");

        let inline = parsed
            .files
            .iter()
            .find(|f| f.content_id.as_deref() == Some("logo@x.test"))
            .expect("the inline part kept the id the body refers to");
        assert_eq!(
            inline.content_type, "image/png",
            "the declared type survived"
        );
        assert_eq!(inline.bytes, image.read().expect("bytes"));

        let attached = parsed
            .files
            .iter()
            .find(|f| f.content_id.is_none())
            .expect("the ordinary attachment");
        assert_eq!(attached.content_type, "application/pdf");
        assert_eq!(attached.bytes, report.read().expect("bytes"));

        // The id the body refers to is exactly the one the message carries, so
        // a reader never resolves a `cid:` to nothing.
        let ids: Vec<&str> = parsed
            .files
            .iter()
            .filter_map(|f| f.content_id.as_deref())
            .collect();
        assert_eq!(ids, ["logo@x.test"]);
        assert!(String::from_utf8_lossy(&raw).contains("Content-ID: <logo@x.test>"));
    }
}
