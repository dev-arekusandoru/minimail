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
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{App, Hsla, SharedString, rgba};
use std::path::PathBuf;

/// Name of the theme applied at startup.
pub const DEFAULT_THEME: &str = "One Dark Pro";

/// Embedded built-in themes. To add one, drop a JSON file into `themes/` and list it here.
const BUILTIN: &[&str] = &[
    include_str!("../themes/one-dark-pro.json"),
    include_str!("../themes/tokyo-night.json"),
];

/// Parse a `#rrggbb` or `#rrggbbaa` color, e.g. an [`crate::model::Account::color`].
pub fn parse_color(value: &str) -> Option<Hsla> {
    let digits = value.strip_prefix('#')?;
    if !digits.is_ascii() || !matches!(digits.len(), 6 | 8) {
        return None;
    }
    let v = u32::from_str_radix(digits, 16).ok()?;
    Some(rgba(if digits.len() == 6 { (v << 8) | 0xff } else { v }).into())
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
        .copied()
        .filter(|json| {
            let name = serde_json::from_str::<serde_json::Value>(json)
                .ok()
                .and_then(|v| v["themes"][0]["name"].as_str().map(str::to_owned));
            name.is_some_and(|n| !registry.themes().contains_key(n.as_str()))
        })
        .collect();
    if missing.is_empty() {
        return;
    }
    let registry = ThemeRegistry::global_mut(cx);
    for json in missing {
        registry.load_themes_from_str(json).expect("built-in theme is valid");
    }
}

/// Register the built-in themes and activate [`DEFAULT_THEME`]. Call after `gpui_kit::init`.
pub fn init(cx: &mut App) {
    register_builtin(cx);
    // The kit's registry drops every theme it did not read from its directory when that
    // directory reloads; put the built-ins back whenever they go missing.
    cx.observe_global::<ThemeRegistry>(register_builtin).detach();
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
    let mut names: Vec<SharedString> =
        ThemeRegistry::global(cx).sorted_themes().into_iter().map(|t| t.name.clone()).collect();
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
    KitTheme::update(cx, |theme| theme.apply_config(&config));
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

// ---- TEMPORARY legacy adapter (removed once the layout files migrate) ----

/// Old token names over the kit theme, for files not yet migrated.
pub struct Theme {
    pub colors: ThemeColor,
    pub name: String,
    pub text: Hsla,
    pub text_muted: Hsla,
    pub surface: Hsla,
    pub accent: Hsla,
    pub on_accent: Hsla,
    pub selection: Hsla,
    pub hover: Hsla,
    pub row_cursor: Hsla,
    pub selected: Hsla,
    pub unread: Hsla,
    pub needs_reply: Hsla,
    pub awaiting: Hsla,
    pub follow_up: Hsla,
    pub reminder: Hsla,
    pub possible_spam: Hsla,
    pub new_sender: Hsla,
    pub urgent: Hsla,
    pub kind: Hsla,
    pub state_inbox: Hsla,
    pub state_snoozed: Hsla,
    pub state_archived: Hsla,
    pub state_filed: Hsla,
    pub state_deleted: Hsla,
    pub error: Hsla,
}

impl std::ops::Deref for Theme {
    type Target = ThemeColor;
    fn deref(&self) -> &ThemeColor {
        &self.colors
    }
}

impl Theme {
    pub fn state_color(&self, state: crate::model::TriageState) -> Hsla {
        state_color(&self.colors, state)
    }
}

pub fn active(cx: &App) -> std::sync::Arc<Theme> {
    let k = cx.theme();
    let c = &k.colors;
    std::sync::Arc::new(Theme {
        colors: *c,
        name: k.theme_name().to_string(),
        text: c.foreground,
        text_muted: c.muted_foreground,
        surface: c.secondary,
        accent: c.primary,
        on_accent: c.primary_foreground,
        selection: c.list_active,
        hover: c.list_hover,
        row_cursor: c.list_active,
        selected: c.primary,
        unread: unread(c),
        needs_reply: needs_reply(c),
        awaiting: awaiting(c),
        follow_up: follow_up(c),
        reminder: reminder(c),
        possible_spam: spam(c),
        new_sender: new_sender(c),
        urgent: urgent(c),
        kind: kind(c),
        state_inbox: inbox(c),
        state_snoozed: snoozed(c),
        state_archived: archived(c),
        state_filed: filed(c),
        state_deleted: deleted(c),
        error: c.danger,
    })
}
