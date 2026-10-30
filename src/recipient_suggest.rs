//! The pure half of the composer's recipient fields: who to suggest for a typed prefix, which
//! addresses are new to you, which reply-all recipients were dropped, how typed or pasted text
//! splits into addresses, and how many chips fit on one blurred line. No GPUI, no clock.

use crate::reading::{Recipient, parse_recipients};
use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};

/// One row of the autocomplete.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub name: String,
    pub address: String,
    /// Threads in the mailbox this address took part in.
    pub threads: u32,
}

impl Suggestion {
    /// The chip text a pick commits: `Name <address>`, or the bare address without a name.
    pub fn chip(&self) -> String {
        if self.name.trim().is_empty() {
            self.address.clone()
        } else {
            format!("{} <{}>", self.name, self.address)
        }
    }
}

struct Entry {
    name: String,
    address: String,
    threads: HashSet<u32>,
}

/// Everyone the composer can suggest: address-book contacts plus the people the mailbox has
/// seen. Built by the owner from its stores; the composer only reads it.
#[derive(Default)]
pub struct Directory {
    entries: HashMap<String, Entry>,
    /// Addresses that have mailed you or that you mailed. Contacts alone do not count.
    history: HashSet<String>,
}

impl Directory {
    /// An address-book contact. Its name wins over one scraped from a header.
    pub fn add_contact(&mut self, name: &str, address: &str) {
        let entry = self.entry(address, name);
        if !name.trim().is_empty() {
            entry.name = name.trim().to_owned();
        }
    }

    /// People you corresponded with directly on one message of thread `thread`: its sender
    /// when it came in, its recipients when you sent it. They count as history.
    pub fn add_mail(&mut self, thread: u32, people: &[Recipient]) {
        for p in people {
            self.history.insert(p.email.trim().to_lowercase());
        }
        self.add_seen(thread, people);
    }

    /// People merely on a message of thread `thread` (cc'd along): suggestable, but you have
    /// not corresponded with them.
    pub fn add_seen(&mut self, thread: u32, people: &[Recipient]) {
        for p in people {
            if p.email.trim().is_empty() {
                continue;
            }
            self.entry(&p.email, p.name.as_deref().unwrap_or(""))
                .threads
                .insert(thread);
        }
    }

    fn entry(&mut self, address: &str, name: &str) -> &mut Entry {
        let address = address.trim();
        let entry = self
            .entries
            .entry(address.to_lowercase())
            .or_insert_with(|| Entry {
                name: String::new(),
                address: address.to_owned(),
                threads: HashSet::new(),
            });
        if entry.name.is_empty() {
            entry.name = name.trim().to_owned();
        }
        entry
    }

    /// Up to `limit` people matching `typed`, skipping addresses in `exclude`. Word-prefix
    /// matches come before substring matches, then the people you share most threads with.
    pub fn suggest(&self, typed: &str, exclude: &[String], limit: usize) -> Vec<Suggestion> {
        let q = typed.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        let taken: HashSet<String> = exclude.iter().map(|c| email_of(c)).collect();
        let mut hits: Vec<(u8, &Entry)> = self
            .entries
            .iter()
            .filter(|(key, _)| !taken.contains(*key))
            .filter_map(|(key, e)| {
                let name = e.name.to_lowercase();
                let starts = key.starts_with(&q)
                    || name
                        .split(|c: char| !c.is_alphanumeric())
                        .any(|w| !w.is_empty() && w.starts_with(&q))
                    || name.starts_with(&q);
                let rank = if starts {
                    0
                } else if key.contains(&q) || name.contains(&q) {
                    1
                } else {
                    return None;
                };
                Some((rank, e))
            })
            .collect();
        hits.sort_by(|a, b| {
            (a.0, Reverse(a.1.threads.len()), a.1.name.to_lowercase(), &a.1.address).cmp(&(
                b.0,
                Reverse(b.1.threads.len()),
                b.1.name.to_lowercase(),
                &b.1.address,
            ))
        });
        hits.into_iter()
            .take(limit)
            .map(|(_, e)| Suggestion {
                name: e.name.clone(),
                address: e.address.clone(),
                threads: e.threads.len() as u32,
            })
            .collect()
    }

    /// Whether you have never emailed or received mail from this chip's address. An empty
    /// history (nothing loaded yet) flags nobody rather than everybody.
    pub fn is_new(&self, chip: &str) -> bool {
        !self.history.is_empty() && !self.history.contains(&email_of(chip))
    }
}

/// The lowercased address of a chip (`Name <a@b>` or a bare address).
pub fn email_of(chip: &str) -> String {
    match parse_recipients(chip).as_slice() {
        [r] => r.email.trim().to_lowercase(),
        _ => chip.trim().to_lowercase(),
    }
}

/// `(name, address)` of a chip, the name `None` when it has none.
pub fn parts_of(chip: &str) -> (Option<String>, String) {
    match parse_recipients(chip).into_iter().next() {
        Some(r) => (r.name, r.email),
        None => (None, chip.trim().to_owned()),
    }
}

/// What a chip shows: the name, or the address when there is none.
pub fn label_of(chip: &str) -> String {
    let (name, address) = parts_of(chip);
    name.unwrap_or(address)
}

/// The recipients of `baseline` that `now` no longer carries, by address.
pub fn dropped(baseline: &[String], now: &[String]) -> Vec<String> {
    let now: HashSet<String> = now.iter().map(|c| email_of(c)).collect();
    baseline
        .iter()
        .filter(|c| !now.contains(&email_of(c)))
        .cloned()
        .collect()
}

/// `cc:` or `bcc:` at the start of typed text: which field it names and the text after it.
pub fn field_prefix(typed: &str) -> Option<(&'static str, &str)> {
    let t = typed.trim_start();
    let lower = t.to_ascii_lowercase();
    ["bcc", "cc"].into_iter().find_map(|name| {
        lower
            .strip_prefix(name)?
            .starts_with(':')
            .then(|| (name, t[name.len() + 1..].trim_start()))
    })
}

/// Typed or pasted text cut at `,` `;` and newlines: the finished parts and the unfinished
/// tail after the last separator. Separators inside quotes or `<>` do not cut.
pub fn split_input(text: &str) -> (Vec<String>, String) {
    let (mut parts, mut cur) = (Vec::new(), String::new());
    let (mut quote, mut angle) = (false, false);
    for c in text.chars() {
        match c {
            '"' if !angle => quote = !quote,
            '<' if !quote => angle = true,
            '>' if !quote => angle = false,
            ',' | ';' | '\n' | '\r' if !quote && !angle => {
                let part = cur.trim();
                if !part.is_empty() {
                    parts.push(part.to_owned());
                }
                cur.clear();
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    (parts, cur.trim_start().to_owned())
}

/// Rough width of a chip, so a blurred field can decide how many fit on a line without a
/// layout pass: avatar, paddings and ~6.6px a character, capped at the chip's max width.
pub fn chip_width(label: &str, new: bool) -> f32 {
    (33. + 6.6 * label.chars().count() as f32 + if new { 26. } else { 0. }).min(220.)
}

/// How many of `widths` (chips, `gap` apart) fit in `avail` px beside the dashed add button
/// (`add` wide) and, when some are left out, the `+N` chip (`more` wide). At least one shows.
pub fn visible_count(widths: &[f32], avail: f32, gap: f32, more: f32, add: f32) -> usize {
    let total: f32 = widths.iter().map(|w| w + gap).sum::<f32>() + add;
    if total <= avail {
        return widths.len();
    }
    let mut used = more + gap + add;
    let mut n = 0;
    for w in widths {
        if used + w + gap > avail {
            break;
        }
        used += w + gap;
        n += 1;
    }
    n.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(name: &str, email: &str) -> Recipient {
        Recipient {
            name: (!name.is_empty()).then(|| name.to_owned()),
            email: email.to_owned(),
        }
    }

    fn directory() -> Directory {
        let mut d = Directory::default();
        d.add_mail(1, &[person("Jon Parker", "jon@acme.io"), person("Maya", "maya@x.io")]);
        d.add_mail(2, &[person("Jon Parker", "jon@acme.io")]);
        d.add_mail(3, &[person("Jonas Weber", "jonas@weber.de")]);
        d.add_contact("Jordan Lee", "j.lee@vendor.co");
        d
    }

    #[test]
    fn prefix_matches_rank_by_shared_threads() {
        let got = directory().suggest("jo", &[], 5);
        let names: Vec<_> = got.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Jon Parker", "Jonas Weber", "Jordan Lee"]);
        assert_eq!(got[0].threads, 2);
        assert_eq!(got[2].threads, 0, "a contact never mailed has no threads");
    }

    #[test]
    fn word_prefix_beats_substring_and_chips_are_excluded() {
        let d = directory();
        assert_eq!(d.suggest("parker", &[], 5)[0].address, "jon@acme.io");
        assert!(d.suggest("zzz", &[], 5).is_empty());
        assert!(d.suggest("", &[], 5).is_empty());
        let taken = ["Jon Parker <JON@acme.io>".to_owned()];
        assert!(d.suggest("jon@", &taken, 5).iter().all(|s| s.address != "jon@acme.io"));
    }

    #[test]
    fn new_means_never_in_the_mail_history() {
        let d = directory();
        assert!(!d.is_new("Jon <JON@acme.io>"));
        assert!(d.is_new("j.lee@vendor.co"), "a contact you never mailed is still new");
        assert!(!Directory::default().is_new("a@b.co"), "nothing loaded flags nobody");
    }

    #[test]
    fn dropped_lists_baseline_people_that_are_gone() {
        let base = ["Raj <raj@x.io>".to_owned(), "maya@x.io".to_owned()];
        let now = ["Maya Chen <MAYA@x.io>".to_owned(), "new@y.io".to_owned()];
        assert_eq!(dropped(&base, &now), ["Raj <raj@x.io>"]);
        assert!(dropped(&base, &base).is_empty());
    }

    #[test]
    fn prefixes_and_pastes() {
        assert_eq!(field_prefix("cc: jo"), Some(("cc", "jo")));
        assert_eq!(field_prefix("BCC:x"), Some(("bcc", "x")));
        assert_eq!(field_prefix("ccx:"), None);
        let (parts, tail) = split_input("a@x.io; \"Doe, J\" <j@x.io>\nb@x.io, c@");
        assert_eq!(parts, ["a@x.io", "\"Doe, J\" <j@x.io>", "b@x.io"]);
        assert_eq!(tail, "c@");
    }

    #[test]
    fn blurred_line_keeps_what_fits() {
        let w = [100., 100., 100., 100.];
        assert_eq!(visible_count(&w, 1000., 4., 36., 24.), 4);
        assert_eq!(visible_count(&w, 300., 4., 36., 24.), 2);
        assert_eq!(visible_count(&w, 10., 4., 36., 24.), 1);
        assert_eq!(label_of("Ana <a@x.io>"), "Ana");
        assert_eq!(label_of("a@x.io"), "a@x.io");
    }
}
