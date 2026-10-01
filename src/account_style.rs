//! Per-account look: an icon (a Lucide icon key) and a color, shown in the sender line of the
//! unified inbox. Pure logic: the key -> `IconName` mapping lives in `app/icons.rs`.
//!
//! New accounts take the next style from the palette by account count. Accounts stored without
//! an icon (configs from before icons existed) fall back to a style derived from their id, which
//! stays put when other accounts come and go.

use crate::model::Account;

/// Account tint colors (`#rrggbb`), the first six being the colors accounts always had.
pub const COLORS: [&str; 8] =
    ["#61afef", "#c678dd", "#98c379", "#e5c07b", "#e06c75", "#56b6c2", "#d19a66", "#be5046"];

/// Lucide icon keys an account can pick from, as `(key, label)`. Keys are what gets stored.
pub const ICONS: [(&str, &str); 16] = [
    ("mail", "Mail"),
    ("briefcase", "Briefcase"),
    ("house", "Home"),
    ("star", "Star"),
    ("heart", "Heart"),
    ("building-2", "Office"),
    ("graduation-cap", "School"),
    ("shopping-bag", "Shopping"),
    ("users", "Community"),
    ("globe", "Web"),
    ("gamepad-2", "Games"),
    ("music", "Music"),
    ("coffee", "Coffee"),
    ("rocket", "Projects"),
    ("wallet", "Finance"),
    ("leaf", "Nature"),
];

/// An icon key and a `#rrggbb` color.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountStyle {
    pub icon: &'static str,
    pub color: &'static str,
}

/// Style for the `index`-th account created: palette order, wrapping.
pub fn style_for_index(index: usize) -> AccountStyle {
    AccountStyle { icon: ICONS[index % ICONS.len()].0, color: COLORS[index % COLORS.len()] }
}

/// Style for an account that has no stored icon: stable per id (FNV-1a), so it survives
/// accounts being added or removed.
pub fn style_for_id(id: &str) -> AccountStyle {
    let hash = id.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3));
    style_for_index(hash as usize)
}

/// The canonical key for `key` if it names an icon in [`ICONS`].
pub fn known_icon(key: &str) -> Option<&'static str> {
    ICONS.iter().find(|(k, _)| *k == key).map(|(k, _)| *k)
}

/// The icon key to draw for `account`: its stored icon, or the id-derived default when none (or
/// an unknown one, e.g. from a newer build) is stored.
pub fn icon_key(account: &Account) -> &'static str {
    account.icon.as_deref().and_then(known_icon).unwrap_or_else(|| style_for_id(&account.id).icon)
}

/// The label of an icon key, `None` when unknown.
pub fn icon_label(key: &str) -> Option<&'static str> {
    ICONS.iter().find(|(k, _)| *k == key).map(|(_, label)| *label)
}

/// A nickname as stored: trimmed, and `None` when empty or whitespace-only.
pub fn normalize_nickname(input: &str) -> Option<String> {
    let trimmed = input.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn set_nickname(account: &Account) -> Option<&str> {
    account.nickname.as_deref().map(str::trim).filter(|n| !n.is_empty())
}

/// How the UI names `account`: its nickname when set, else its `name`.
pub fn display_name(account: &Account) -> &str {
    set_nickname(account).unwrap_or(&account.name)
}

/// Like [`display_name`], but falls back to `fallback` (e.g. the email) instead of the name,
/// for places that label an account by address.
pub fn nickname_or<'a>(account: &'a Account, fallback: &'a str) -> &'a str {
    set_nickname(account).unwrap_or(fallback)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ProviderKind;

    #[test]
    fn an_account_stored_without_an_icon_still_parses() {
        let old: Account =
            serde_json::from_str(r##"{"id":"x","name":"X","email":"x@y.z","color":"#61afef"}"##).unwrap();
        assert_eq!(old.icon, None);
        let json = serde_json::to_string(&old).unwrap();
        assert!(!json.contains("icon"), "unset icons are not written: {json}");
        let styled = Account { icon: Some("star".into()), nickname: Some("Mine".into()), ..old };
        let back: Account = serde_json::from_str(&serde_json::to_string(&styled).unwrap()).unwrap();
        assert_eq!(back, styled);
    }

    fn account(id: &str, icon: Option<&str>) -> Account {
        Account {
            id: id.into(),
            name: id.into(),
            email: format!("{id}@x.io"),
            color: "#61afef".into(),
            icon: icon.map(str::to_owned),
            nickname: None,
            provider: ProviderKind::Gmail,
        }
    }

    #[test]
    fn nicknames_are_trimmed_and_blank_means_unset() {
        assert_eq!(normalize_nickname("  Work mail "), Some("Work mail".into()));
        assert_eq!(normalize_nickname(""), None);
        assert_eq!(normalize_nickname(" \t "), None);
    }

    #[test]
    fn display_name_prefers_a_nickname_and_falls_back() {
        let mut a = account("a", None);
        a.name = "Alice".into();
        assert_eq!(display_name(&a), "Alice");
        assert_eq!(nickname_or(&a, "a@x.io"), "a@x.io");
        a.nickname = Some("  ".into());
        assert_eq!(display_name(&a), "Alice", "a blank stored nickname is unset");
        assert_eq!(nickname_or(&a, "a@x.io"), "a@x.io");
        a.nickname = Some("Home".into());
        assert_eq!(display_name(&a), "Home");
        assert_eq!(nickname_or(&a, "a@x.io"), "Home");
    }

    #[test]
    fn index_styles_walk_the_palette_and_wrap() {
        assert_eq!(style_for_index(0), AccountStyle { icon: "mail", color: "#61afef" });
        assert_eq!(style_for_index(1).icon, "briefcase");
        assert_eq!(style_for_index(COLORS.len()).color, COLORS[0]);
        assert_ne!(style_for_index(COLORS.len()).icon, style_for_index(0).icon);
        assert_eq!(style_for_index(ICONS.len()), style_for_index(0));
    }

    #[test]
    fn palette_entries_are_valid_and_unique() {
        for color in COLORS {
            assert!(crate::theme::parse_color(color).is_some(), "{color}");
        }
        let mut keys: Vec<_> = ICONS.iter().map(|(k, _)| *k).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), ICONS.len());
    }

    #[test]
    fn stored_icon_wins_and_unknown_falls_back_to_the_id_default() {
        assert_eq!(icon_key(&account("a", Some("star"))), "star");
        let fallback = style_for_id("a").icon;
        assert_eq!(icon_key(&account("a", None)), fallback);
        assert_eq!(icon_key(&account("a", Some("no-such-icon"))), fallback);
    }

    #[test]
    fn id_default_is_deterministic_and_independent_of_other_accounts() {
        assert_eq!(style_for_id("gmail:me@x.io"), style_for_id("gmail:me@x.io"));
        let distinct: std::collections::HashSet<_> =
            (0..40).map(|i| style_for_id(&format!("gmail:user{i}@x.io")).icon).collect();
        assert!(distinct.len() > 4);
    }

    #[test]
    fn labels_resolve_for_every_key() {
        for (key, label) in ICONS {
            assert_eq!(icon_label(key), Some(label));
        }
        assert_eq!(icon_label("nope"), None);
    }
}
