//! The sidebar's count badges: how a message count is written, and when a badge shows at all.

/// A count in at most four characters: `999` stays, `2285` becomes `2.3k`, `12000` `12k`.
/// Thousands round to one decimal, and a trailing `.0` is dropped.
pub fn compact_count(n: usize) -> String {
    if n < 1000 {
        return n.to_string();
    }
    let tenths = (n + 50) / 100;
    if tenths.is_multiple_of(10) {
        format!("{}k", tenths / 10)
    } else {
        format!("{}.{}k", tenths / 10, tenths % 10)
    }
}

/// The text of the badge on a sidebar tile: none for an empty count.
pub fn tile_badge(count: usize) -> Option<String> {
    (count > 0).then(|| compact_count(count))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_compact_to_thousands() {
        let text = |n| compact_count(n);
        assert_eq!(text(0), "0");
        assert_eq!(text(999), "999");
        assert_eq!(text(1000), "1k");
        assert_eq!(text(2285), "2.3k");
        assert_eq!(text(2249), "2.2k");
        assert_eq!(text(9950), "10k");
        assert_eq!(text(12_000), "12k");
    }

    #[test]
    fn a_badge_needs_a_count() {
        assert_eq!(tile_badge(2285).as_deref(), Some("2.3k"));
        assert_eq!(tile_badge(0), None);
    }
}
