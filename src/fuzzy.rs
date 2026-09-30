//! Command-palette matching: exact key, name prefix, substring, then subsequence. Pure, no GPUI.

/// How well a command matches a query; lower is better.
pub type Rank = u8;

/// The query is exactly the command's key (`"e"`, `"g s"`).
pub const KEY: Rank = 0;
/// The name starts with the query.
pub const PREFIX: Rank = 1;
/// The name contains the query.
pub const SUBSTRING: Rank = 2;
/// The query's non-space characters appear in the name in order.
pub const SUBSEQUENCE: Rank = 3;
/// Rank of every command for an empty query.
pub const ALL: Rank = 0;

/// Rank of a command called `name`, bound to `key` (a hint such as `"shift-e"` or `"g i"`,
/// possibly empty), for the typed `query`, or `None` when it does not match.
///
/// Case-insensitive. An empty query matches everything.
pub fn rank(name: &str, key: &str, query: &str) -> Option<Rank> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Some(ALL);
    }
    if key.to_lowercase() == q {
        return Some(KEY);
    }
    let name = name.to_lowercase();
    if name.starts_with(&q) {
        return Some(PREFIX);
    }
    if name.contains(&q) {
        return Some(SUBSTRING);
    }
    let mut wanted = q.chars().filter(|c| !c.is_whitespace()).peekable();
    for ch in name.chars() {
        if wanted.peek() == Some(&ch) {
            wanted.next();
        }
    }
    wanted.peek().is_none().then_some(SUBSEQUENCE)
}

/// Whether the command matches at all.
pub fn matches(name: &str, key: &str, query: &str) -> bool {
    rank(name, key, query).is_some()
}

#[cfg(test)]
mod tests {
    use super::{KEY, PREFIX, SUBSEQUENCE, SUBSTRING, matches, rank};

    #[test]
    fn empty_or_blank_query_matches_everything() {
        assert!(matches("Archive", "e", ""));
        assert!(matches("Archive", "e", "   "));
    }

    #[test]
    fn substring_is_case_insensitive() {
        assert!(matches("Archive", "e", "ARCHIVE"));
        assert!(matches("Go to snoozed", "g s", "to snooz"));
        assert!(!matches("Undo", "u", "archive"));
    }

    #[test]
    fn subsequence_tolerates_gaps_and_spaces() {
        assert!(matches("Go to snoozed", "g s", "gtsn"));
        assert!(matches("Go to snoozed", "g s", "g t s"));
        assert!(!matches("Go to snoozed", "g s", "snz go"), "order matters");
    }

    #[test]
    fn exact_key_matches_even_without_name_overlap() {
        assert!(matches("Toggle command palette", "cmd-k", "cmd-k"));
        assert!(matches("Go to snoozed", "g s", "G S"));
        assert!(!matches("Archive", "e", "cmd-k"));
    }

    #[test]
    fn ranks_order_key_prefix_substring_subsequence() {
        assert_eq!(rank("Archive", "e", "e"), Some(KEY));
        assert_eq!(rank("Archive", "e", "arch"), Some(PREFIX));
        assert_eq!(rank("Go to archive", "g a", "arch"), Some(SUBSTRING));
        assert_eq!(rank("Go to archive", "g a", "gta"), Some(SUBSEQUENCE));
        assert_eq!(rank("Undo", "u", "zz"), None);
    }
}
