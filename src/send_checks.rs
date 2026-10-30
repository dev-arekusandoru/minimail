//! Pure checks over a draft's body, for the composer's status line and body shortcuts.
//! Independent of GPUI.

use std::ops::Range;

/// Whether the body talks about an attachment. Quoted lines (`>`) are somebody else's words
/// and fenced code is not prose, so neither counts.
///
/// shortcut: stem matching on English words, not language-aware; extend the list when a
/// real miss shows up.
pub fn mentions_attachment(body: &str) -> bool {
    let mut fenced = false;
    for line in body.lines() {
        let line = line.trim_start();
        if line.starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced || line.starts_with('>') {
            continue;
        }
        let lower = line.to_lowercase();
        let hit = lower
            .split(|c: char| !c.is_alphanumeric())
            .any(|w| w.starts_with("attach") || w == "enclosed" || w == "enclosing");
        if hit {
            return true;
        }
    }
    false
}

/// The byte range of the `/query` that opens the body's slash menu: a `/` at the start of
/// the body or right after whitespace, with no whitespace between it and the caret.
/// `None` inside a word, so `and/or` and URLs are left as text.
pub fn slash_trigger(text: &str, caret: usize) -> Option<Range<usize>> {
    let before = text.get(..caret)?;
    let start = before
        .char_indices()
        .rfind(|&(_, c)| c.is_whitespace())
        .map_or(0, |(i, c)| i + c.len_utf8());
    before[start..].starts_with('/').then_some(start..caret)
}

#[cfg(test)]
mod tests {
    use super::{mentions_attachment, slash_trigger};

    #[test]
    fn attachment_words_are_found_outside_quotes_and_code() {
        assert!(mentions_attachment("Hi — I've attached the deck."));
        assert!(mentions_attachment("See the Attachment"));
        assert!(mentions_attachment("Enclosed is the form"));
        assert!(!mentions_attachment("> I attached it before\nThanks"));
        assert!(!mentions_attachment("```\nattach()\n```\nok"));
        assert!(!mentions_attachment("nothing here"));
    }

    #[test]
    fn the_slash_trigger_is_a_slash_at_the_start_of_a_word() {
        assert_eq!(slash_trigger("/", 1), Some(0..1));
        assert_eq!(slash_trigger("Hello\n/", 7), Some(6..7));
        assert_eq!(slash_trigger("Hello. /", 8), Some(7..8), "mid-line, after a space");
        assert_eq!(slash_trigger("hello /bo", 9), Some(6..9), "the query is the text after it");
        assert_eq!(slash_trigger("hello\t/x", 8), Some(6..8));
        assert_eq!(slash_trigger("/x", 1), Some(0..1), "the caret ends the query");
        assert_eq!(slash_trigger("a/", 2), None);
        assert_eq!(slash_trigger("and/or", 6), None, "a slash inside a word is text");
        assert_eq!(slash_trigger("https://example.com", 19), None);
        assert_eq!(slash_trigger("/x ", 3), None, "whitespace ends the query");
    }
}
