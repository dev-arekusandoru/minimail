//! Plain-text message previews: the muted snippet under a row's subject and the
//! `Preview lines` setting. Pure functions, no UI.

/// Longest snippet produced, in characters. Enough for five wide lines.
const MAX_CHARS: usize = 480;

/// Selectable `Preview lines` values: `0` is Off.
pub const OPTIONS: [u8; 6] = [0, 1, 2, 3, 4, 5];
/// Default number of preview lines.
pub const DEFAULT_LINES: u8 = 2;

/// Settings label for a preview line count: `Off`, `1 line`, `2 lines`, ...
pub fn lines_label(lines: u8) -> String {
    match lines {
        0 => "Off".to_owned(),
        1 => "1 line".to_owned(),
        n => format!("{n} lines"),
    }
}

/// Next (`step > 0`) or previous option, clamped at Off and 5.
pub fn step_lines(lines: u8, step: i8) -> u8 {
    let max = OPTIONS[OPTIONS.len() - 1] as i16;
    (lines as i16 + step as i16).clamp(0, max) as u8
}

/// Next option, wrapping from 5 back to Off (used by toggles and the palette).
pub fn cycle_lines(lines: u8) -> u8 {
    if lines >= OPTIONS[OPTIONS.len() - 1] { 0 } else { lines + 1 }
}

fn is_signature_delimiter(line: &str) -> bool {
    matches!(line.trim_end(), "--" | "—" | "___")
}

fn is_reply_header(line: &str) -> bool {
    let l = line.trim();
    let lower = l.to_lowercase();
    (lower.starts_with("on ") && lower.ends_with("wrote:"))
        || lower.starts_with("-----original message")
        || lower.starts_with("sent from my ")
        || lower.starts_with("get outlook for ")
        || (lower.starts_with("from:") && lower.contains("sent:"))
}

/// Plain-text snippet of `body`: quoted lines (`>`), everything after a signature
/// delimiter or reply header, and blank lines are dropped; whitespace is collapsed to
/// single spaces; the result is capped at [`MAX_CHARS`] characters (ellipsis-free, the
/// row clamps by lines).
pub fn snippet(body: &str) -> String {
    let mut out = String::new();
    for line in body.lines() {
        if is_signature_delimiter(line) || is_reply_header(line) {
            break;
        }
        let line = line.trim();
        if line.is_empty() || line.starts_with('>') {
            continue;
        }
        for word in line.split_whitespace() {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(word);
        }
        if out.chars().count() >= MAX_CHARS {
            break;
        }
    }
    match out.char_indices().nth(MAX_CHARS) {
        Some((at, _)) => {
            out.truncate(at);
            out
        }
        None => out,
    }
}

/// Whether the subject or the opening of the body announces an attachment
/// (the mailbox model carries no attachment list, so this is a text heuristic).
pub fn mentions_attachment(subject: &str, body: &str) -> bool {
    let head: String = snippet(body).chars().take(160).collect();
    let text = format!("{subject} {head}").to_lowercase();
    if text.contains("no longer attach") || text.contains("not attach") {
        return false;
    }
    text.contains("attached") || text.contains("attachment")
}
