//! Pure composer logic: recipients, subject and quoted body for Reply, Reply all and Forward.
use crate::model::Message;
use crate::reading::{Recipient, parse_recipients};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DraftKind {
    Reply,
    ReplyAll,
    Forward,
}

/// Initial composer fields. `to`/`cc` are comma-separated, like `Message::to`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Draft {
    pub to: String,
    pub cc: String,
    pub subject: String,
    /// Prefilled body (the forwarded-message block for `Forward`, empty otherwise).
    pub body: String,
}

fn format_recipient(r: &Recipient) -> String {
    match &r.name {
        Some(name) => format!("{name} <{}>", r.email),
        None => r.email.clone(),
    }
}

fn has_prefix(subject: &str, prefixes: &[&str]) -> bool {
    let s = subject.trim_start().to_lowercase();
    prefixes.iter().any(|p| s.starts_with(p))
}

pub fn reply_subject(subject: &str) -> String {
    if has_prefix(subject, &["re:"]) {
        subject.to_owned()
    } else {
        format!("Re: {subject}")
    }
}

pub fn forward_subject(subject: &str) -> String {
    if has_prefix(subject, &["fwd:", "fw:"]) {
        subject.to_owned()
    } else {
        format!("Fwd: {subject}")
    }
}

/// Reply all: the sender plus the original's To, with the original's Cc behind them, minus
/// `own_email`, deduped case-insensitively across both fields (To wins).
///
/// On one of our own sent messages the sender *is* us, so it contributes nothing and would
/// only address the reply to ourselves: the original's recipients are the whole audience.
pub fn reply_all_recipients(msg: &Message, own_email: &str) -> (String, String) {
    let own = own_email.trim().to_lowercase();
    let mut seen: Vec<String> = vec![own.clone()];
    let take = |list: Vec<Recipient>, seen: &mut Vec<String>| -> String {
        let mut out = Vec::new();
        for r in list {
            let key = r.email.trim().to_lowercase();
            if !seen.contains(&key) {
                seen.push(key);
                out.push(format_recipient(&r));
            }
        }
        out.join(", ")
    };
    let from_own = msg.from_email.trim().to_lowercase() == own;
    let mut to = Vec::new();
    if !from_own {
        to.extend(parse_recipients(&format!(
            "{} <{}>",
            msg.from_name, msg.from_email
        )));
    }
    to.extend(parse_recipients(&msg.to));
    let to = take(to, &mut seen);
    let cc = take(parse_recipients(&msg.cc), &mut seen);
    (to, cc)
}

fn size_label(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / (1 << 20) as f64),
        b if b >= 1 << 10 => format!("{} KB", b >> 10),
        b => format!("{b} B"),
    }
}

/// Standard forwarded-message block followed by the original text body.
pub fn forward_body(msg: &Message) -> String {
    let mut out = format!(
        "\n\n---------- Forwarded message ----------\nFrom: {} <{}>\nDate: {}\nSubject: {}\nTo: {}\n",
        msg.from_name, msg.from_email, msg.received, msg.subject, msg.to
    );
    if !msg.cc.trim().is_empty() {
        out.push_str(&format!("Cc: {}\n", msg.cc));
    }
    if !msg.attachments.is_empty() {
        let list: Vec<String> = msg
            .attachments
            .iter()
            .map(|a| format!("{} ({})", a.name, size_label(a.size)))
            .collect();
        out.push_str(&format!("Attachments: {}\n", list.join(", ")));
    }
    out.push('\n');
    out.push_str(&msg.body);
    out
}

/// Who a reply is addressed to.
///
/// Replying to one of our own sent messages answers the people it was sent to, not our own
/// address: sending a message to ourselves is never what "reply" means. With no recipient on
/// the original to answer, the sender is the only address there is.
fn reply_recipient(msg: &Message, own_email: &str) -> String {
    let own = own_email.trim().to_lowercase();
    let from_own = msg.from_email.trim().to_lowercase() == own;
    if from_own {
        let to = parse_recipients(&msg.to)
            .into_iter()
            .filter(|r| r.email.trim().to_lowercase() != own)
            .map(|r| format_recipient(&r))
            .collect::<Vec<_>>()
            .join(", ");
        if !to.is_empty() {
            return to;
        }
    }
    format!("{} <{}>", msg.from_name, msg.from_email)
}

pub fn draft(kind: DraftKind, msg: &Message, own_email: &str) -> Draft {
    match kind {
        DraftKind::Reply => Draft {
            to: reply_recipient(msg, own_email),
            cc: String::new(),
            subject: reply_subject(&msg.subject),
            body: String::new(),
        },
        DraftKind::ReplyAll => {
            let (to, cc) = reply_all_recipients(msg, own_email);
            Draft {
                to,
                cc,
                subject: reply_subject(&msg.subject),
                body: String::new(),
            }
        }
        DraftKind::Forward => Draft {
            to: String::new(),
            cc: String::new(),
            subject: forward_subject(&msg.subject),
            body: forward_body(msg),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Attachment, Mailbox};

    fn msg() -> Message {
        let mut m = Mailbox::load_default().messages().first().unwrap().clone();
        m.from_name = "Priya Raman".into();
        m.from_email = "Priya@Northwind.io".into();
        m.to = "You <you@example.com>, Cal Okonkwo <cal@northwind.io>, priya@northwind.io".into();
        m.cc = "CAL@northwind.io, Ines Duarte <ines@whitfield.dev>".into();
        m.subject = "Plan".into();
        m.body = "Hello".into();
        m.attachments.clear();
        m
    }

    #[test]
    fn reply_all_excludes_self_and_dedupes_case_insensitively() {
        let (to, cc) = reply_all_recipients(&msg(), "YOU@example.com");
        assert_eq!(
            to,
            "Priya Raman <Priya@Northwind.io>, Cal Okonkwo <cal@northwind.io>"
        );
        assert_eq!(cc, "Ines Duarte <ines@whitfield.dev>");
    }

    #[test]
    fn reply_all_to_own_message_keeps_original_recipients() {
        let mut m = msg();
        m.from_email = "you@example.com".into();
        m.from_name = "You".into();
        let (to, _) = reply_all_recipients(&m, "you@example.com");
        assert_eq!(to, "Cal Okonkwo <cal@northwind.io>, priya@northwind.io");
    }

    /// Replying to our own sent message answers the people it went to. Addressing it to
    /// ourselves would mail ourselves and no one else, which is what the sender field would
    /// otherwise produce now that a send really goes out.
    #[test]
    fn reply_to_our_own_message_addresses_its_recipients() {
        let mut m = msg();
        m.from_email = "you@example.com".into();
        m.from_name = "You".into();
        let d = draft(DraftKind::Reply, &m, "you@example.com");
        assert!(
            !d.to.to_lowercase().contains("you@example.com"),
            "a reply must never be addressed to us, got {:?}",
            d.to
        );
        assert!(
            d.to.contains("cal@northwind.io"),
            "the audience is kept, got {:?}",
            d.to
        );
    }

    /// With nobody else on the original there is no one to answer, so the sender is all
    /// that is left — the user can change it, but the field is never silently blank.
    #[test]
    fn reply_to_our_own_solo_message_falls_back_to_the_sender() {
        let mut m = msg();
        m.from_email = "you@example.com".into();
        m.from_name = "You".into();
        m.to = String::new();
        let d = draft(DraftKind::Reply, &m, "you@example.com");
        assert_eq!(d.to, "You <you@example.com>");
    }

    /// Someone else's message is still answered directly, whatever else is on it.
    #[test]
    fn reply_to_someone_else_addresses_the_sender() {
        assert_eq!(
            draft(DraftKind::Reply, &msg(), "you@example.com").to,
            "Priya Raman <Priya@Northwind.io>"
        );
    }

    #[test]
    fn subjects_do_not_stack_prefixes() {
        assert_eq!(reply_subject("Plan"), "Re: Plan");
        assert_eq!(reply_subject("RE: Plan"), "RE: Plan");
        assert_eq!(forward_subject("Plan"), "Fwd: Plan");
        assert_eq!(forward_subject("fwd: Plan"), "fwd: Plan");
        assert_eq!(forward_subject("FW: Plan"), "FW: Plan");
        assert_eq!(forward_subject("Re: Plan"), "Fwd: Re: Plan");
    }

    #[test]
    fn forward_has_empty_to_and_header_block() {
        let mut m = msg();
        m.attachments = vec![Attachment {
            name: "a.pdf".into(),
            size: 2048,
        }];
        let d = draft(DraftKind::Forward, &m, "you@example.com");
        assert!(d.to.is_empty() && d.cc.is_empty());
        assert_eq!(d.subject, "Fwd: Plan");
        assert!(d.body.contains(
            "---------- Forwarded message ----------\nFrom: Priya Raman <Priya@Northwind.io>\n"
        ));
        assert!(d.body.contains(&format!(
            "Date: {}\nSubject: Plan\nTo: {}\n",
            m.received, m.to
        )));
        assert!(d.body.contains("Cc: CAL@northwind.io"));
        assert!(d.body.contains("Attachments: a.pdf (2 KB)\n\nHello"));
    }
}
