//! Per-sender spam verdicts the user chose: addresses whose mail is never spam, or always spam.
//! Exact address, case-insensitive; never a domain. Pure logic, no UI.

use std::collections::BTreeSet;

/// What the user decided about a sender's mail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpamVerdict {
    Never,
    Always,
}

/// The two lists. An address is in at most one: the last choice wins.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpamSenders {
    never: BTreeSet<String>,
    always: BTreeSet<String>,
}

fn key(email: &str) -> String {
    email.trim().to_ascii_lowercase()
}

impl SpamSenders {
    pub fn from_lists(never: Vec<String>, always: Vec<String>) -> Self {
        let mut s = Self::default();
        for e in never {
            s.set(&e, Some(SpamVerdict::Never));
        }
        for e in always {
            s.set(&e, Some(SpamVerdict::Always));
        }
        s
    }

    pub fn verdict(&self, email: &str) -> Option<SpamVerdict> {
        let e = key(email);
        if self.never.contains(&e) {
            Some(SpamVerdict::Never)
        } else if self.always.contains(&e) {
            Some(SpamVerdict::Always)
        } else {
            None
        }
    }

    /// Record `verdict` for `email` (or clear it), removing the address from the other list.
    /// Returns the verdict before, so the caller can undo. Empty addresses are ignored.
    pub fn set(&mut self, email: &str, verdict: Option<SpamVerdict>) -> Option<SpamVerdict> {
        let e = key(email);
        if e.is_empty() {
            return None;
        }
        let before = self.verdict(&e);
        self.never.remove(&e);
        self.always.remove(&e);
        match verdict {
            Some(SpamVerdict::Never) => self.never.insert(e),
            Some(SpamVerdict::Always) => self.always.insert(e),
            None => false,
        };
        before
    }

    pub fn never(&self) -> Vec<String> {
        self.never.iter().cloned().collect()
    }

    pub fn always(&self) -> Vec<String> {
        self.always.iter().cloned().collect()
    }

    /// Whether Jev should not be asked the spam question for mail from `email`.
    pub fn skips_question(&self, email: &str) -> bool {
        self.verdict(email).is_some()
    }

    /// Whether accepting spam from `email` should prompt: it has an address, no verdict, and
    /// is not blocked.
    pub fn should_prompt(&self, email: &str, blocked: bool) -> bool {
        !blocked && !key(email).is_empty() && self.verdict(email).is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_insensitive_and_exclusive() {
        let mut s = SpamSenders::default();
        assert_eq!(s.set("A@X.com", Some(SpamVerdict::Never)), None);
        assert_eq!(s.verdict("a@x.COM"), Some(SpamVerdict::Never));
        assert_eq!(s.set("a@x.com", Some(SpamVerdict::Always)), Some(SpamVerdict::Never));
        assert!(s.never().is_empty());
        assert_eq!(s.always(), vec!["a@x.com"]);
        assert_eq!(s.verdict("b@x.com"), None, "never the domain");
        s.set("a@x.com", None);
        assert_eq!(s.verdict("a@x.com"), None);
    }

    #[test]
    fn skip_and_prompt_decisions() {
        let s = SpamSenders::from_lists(vec!["n@x.com".into()], vec!["a@x.com".into()]);
        assert!(s.skips_question("N@x.com") && s.skips_question("a@x.com"));
        assert!(!s.skips_question("o@x.com"));
        assert!(!s.should_prompt("n@x.com", false));
        assert!(!s.should_prompt("o@x.com", true), "blocked");
        assert!(!s.should_prompt("", false), "no address");
        assert!(s.should_prompt("o@x.com", false));
    }
}
