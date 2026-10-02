//! Color themes on top of gpui-kit's theme system.
//!
//! The kit's [`Theme`] global is the single source of truth: views read colors with
//! `cx.theme()` (`ActiveTheme`). Themes are kit `ThemeSet` JSON files. Built-ins live in
//! `themes/*.json` and are embedded; adding one is a new file plus one line in [`BUILTIN`]. User
//! themes are read (and hot-reloaded) from [`user_themes_dir`].
//!
//! The kit has no slot for this app's triage colors, so they are derived from kit colors by the
//! small helpers below ([`state_color`], [`urgent`], …).

pub use gpui_kit::component::theme::ThemeColor;
use gpui_kit::component::theme::{Theme as KitTheme, ThemeRegistry};
use gpui_kit::{App, Hsla, SharedString, px, rgba};
use std::{path::PathBuf, sync::LazyLock};

/// Name of the theme applied at startup.
pub const DEFAULT_THEME: &str = "One Dark Pro";

/// Embedded built-in themes. To add one, drop a JSON file into `themes/` and list it here.
const BUILTIN: &[&str] = &[
    include_str!("../themes/one-dark-pro.json"),
    include_str!("../themes/tokyo-night.json"),
    include_str!("../themes/adventure.json"),
    include_str!("../themes/alduin.json"),
    include_str!("../themes/asciinema.json"),
    include_str!("../themes/aurora.json"),
    include_str!("../themes/ayu.json"),
    include_str!("../themes/catppuccin.json"),
    include_str!("../themes/everforest.json"),
    include_str!("../themes/fahrenheit.json"),
    include_str!("../themes/flexoki.json"),
    include_str!("../themes/gruvbox.json"),
    include_str!("../themes/harper.json"),
    include_str!("../themes/hybrid.json"),
    include_str!("../themes/jellybeans.json"),
    include_str!("../themes/kibble.json"),
    include_str!("../themes/macos-classic.json"),
    include_str!("../themes/mellifluous.json"),
    include_str!("../themes/molokai.json"),
    include_str!("../themes/solarized.json"),
    include_str!("../themes/spaceduck.json"),
    include_str!("../themes/tokyonight.json"),
    include_str!("../themes/twilight.json"),
];

// Cache every variant's name: directory reloads can retain only part of a theme family.
static BUILTIN_NAMES: LazyLock<Vec<Vec<String>>> = LazyLock::new(|| {
    BUILTIN
        .iter()
        .map(|json| {
            serde_json::from_str::<gpui_kit::component::theme::ThemeSet>(json)
                .expect("built-in theme is valid")
                .themes
                .into_iter()
                .map(|theme| theme.name.to_string())
                .collect()
        })
        .collect()
});

/// Parse a `#rrggbb` or `#rrggbbaa` color, e.g. an [`crate::model::Account::color`].
pub fn parse_color(value: &str) -> Option<Hsla> {
    let digits = value.strip_prefix('#')?;
    if !digits.is_ascii() || !matches!(digits.len(), 6 | 8) {
        return None;
    }
    let v = u32::from_str_radix(digits, 16).ok()?;
    Some(
        rgba(if digits.len() == 6 {
            (v << 8) | 0xff
        } else {
            v
        })
        .into(),
    )
}

/// The `#rrggbb` form of `color` (alpha dropped); the inverse of [`parse_color`].
pub fn to_hex(color: Hsla) -> String {
    let c = gpui_kit::Rgba::from(color);
    let byte = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
    format!("#{:02x}{:02x}{:02x}", byte(c.r), byte(c.g), byte(c.b))
}

/// Directory user theme files are read from: `$MAIL_CLASSIFIER_THEMES`, else
/// `~/.config/mail-classifier/themes`.
pub fn user_themes_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("MAIL_CLASSIFIER_THEMES") {
        return Some(PathBuf::from(dir));
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config/mail-classifier/themes"))
}

/// Add any missing built-in theme to the kit registry. The kit does not replace a name that is
/// already registered.
fn register_builtin(cx: &mut App) {
    let registry = ThemeRegistry::global(cx);
    let missing: Vec<&str> = BUILTIN
        .iter()
        .zip(BUILTIN_NAMES.iter())
        .filter(|(_, names)| {
            names
                .iter()
                .any(|name| !registry.themes().contains_key(name.as_str()))
        })
        .map(|(json, _)| *json)
        .collect();
    if missing.is_empty() {
        return;
    }
    let registry = ThemeRegistry::global_mut(cx);
    for json in missing {
        registry
            .load_themes_from_str(json)
            .expect("built-in theme is valid");
    }
}

/// Register the built-in themes and activate [`DEFAULT_THEME`]. Call after `gpui_kit::init`.
pub fn init(cx: &mut App) {
    register_builtin(cx);
    // The kit's registry drops every theme it did not read from its directory when that
    // directory reloads; put the built-ins back whenever they go missing.
    cx.observe_global::<ThemeRegistry>(register_builtin)
        .detach();
    apply(cx, DEFAULT_THEME);
}

/// Load and hot-reload the user themes directory into the kit registry (see [`user_themes_dir`]).
pub fn watch_user_themes(cx: &mut App) {
    let Some(dir) = user_themes_dir() else {
        return;
    };
    if let Err(err) = ThemeRegistry::watch_dir(dir, cx, |_| {}) {
        eprintln!("theme directory not watched: {err}");
    }
}

/// Names of all registered themes: the default first, then the kit's order (its own defaults,
/// then light before dark, then by name).
pub fn names(cx: &App) -> Vec<SharedString> {
    let mut names: Vec<SharedString> = ThemeRegistry::global(cx)
        .sorted_themes()
        .into_iter()
        .map(|t| t.name.clone())
        .collect();
    if let Some(i) = names.iter().position(|n| n == DEFAULT_THEME) {
        let default = names.remove(i);
        names.insert(0, default);
    }
    names
}

/// Activate the named theme. Returns false, changing nothing, when no such theme is registered.
pub fn apply(cx: &mut App, name: &str) -> bool {
    let Some(config) = ThemeRegistry::global(cx).themes().get(name).cloned() else {
        return false;
    };
    KitTheme::update(cx, |theme| {
        theme.apply_config(&config);
        // Bottom toasts clear the hint bar.
        theme.notification.margins.bottom = px(40.);
    });
    true
}

/// Color of a triage state.
pub fn state_color(c: &ThemeColor, state: crate::model::TriageState) -> Hsla {
    use crate::model::TriageState::*;
    match state {
        Inbox => inbox(c),
        Snoozed => snoozed(c),
        Archived => archived(c),
        Filed(_) => filed(c),
        Deleted => deleted(c),
    }
}

pub fn inbox(c: &ThemeColor) -> Hsla {
    c.primary
}

pub fn snoozed(c: &ThemeColor) -> Hsla {
    c.yellow
}

pub fn archived(c: &ThemeColor) -> Hsla {
    c.magenta
}

pub fn filed(c: &ThemeColor) -> Hsla {
    c.cyan
}

pub fn deleted(c: &ThemeColor) -> Hsla {
    c.danger
}

/// Unread status bar.
pub fn unread(c: &ThemeColor) -> Hsla {
    c.yellow
}

/// Spam and possible-spam marks.
pub fn spam(c: &ThemeColor) -> Hsla {
    c.red
}

pub fn needs_reply(c: &ThemeColor) -> Hsla {
    c.warning
}

pub fn awaiting(c: &ThemeColor) -> Hsla {
    c.cyan
}

pub fn follow_up(c: &ThemeColor) -> Hsla {
    c.yellow
}

pub fn reminder(c: &ThemeColor) -> Hsla {
    c.magenta
}

pub fn new_sender(c: &ThemeColor) -> Hsla {
    c.blue
}

pub fn urgent(c: &ThemeColor) -> Hsla {
    c.danger
}

pub fn kind(c: &ThemeColor) -> Hsla {
    c.cyan
}
