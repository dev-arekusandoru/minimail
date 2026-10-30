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

/// The byte range of a lone `/` that is the whole line the caret sits at the end of — the
/// trigger for the body's slash menu. `None` for anything else, so continued typing
/// (`/x`) or a `/` mid-line dismisses it.
pub fn slash_trigger(text: &str, caret: usize) -> Option<Range<usize>> {
    let before = text.get(..caret)?;
    let start = before.rfind('\n').map_or(0, |i| i + 1);
    let at_line_end = text[caret..].chars().next().is_none_or(|c| c == '\n');
    (&before[start..] == "/" && at_line_end).then_some(start..caret)
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
    fn the_slash_trigger_is_a_lone_slash_on_its_line() {
        assert_eq!(slash_trigger("/", 1), Some(0..1));
        assert_eq!(slash_trigger("Hello\n/", 7), Some(6..7));
        assert_eq!(slash_trigger("a/", 2), None);
        assert_eq!(slash_trigger("/x", 2), None);
        assert_eq!(slash_trigger("/\nmore", 1), Some(0..1));
        assert_eq!(slash_trigger("/ab", 1), None, "text follows the caret");
    }
}
