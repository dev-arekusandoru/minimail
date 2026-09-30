//! Color themes: semantic tokens, a registry of built-in and user themes, and the active theme.
//!
//! Themes are plain data (JSON): `{ "name": "…", "colors": { "<token>": "#rrggbb", … } }`.
//! Built-ins live in `themes/*.json` and are embedded; adding one is a new file plus one line in
//! [`BUILTIN`]. User themes are loaded from a directory of `*.json` files (see
//! [`user_themes_dir`]). Views read colors only through [`active`].

use gpui_kit::component::theme::Theme as KitTheme;
use gpui_kit::{App, Global, Hsla, rgba};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

/// Name of the theme used when none (or an unknown one) is requested.
pub const DEFAULT_THEME: &str = "One Dark Pro";

/// Embedded built-in themes. To add one, drop a JSON file into `themes/` and list it here.
const BUILTIN: &[&str] = &[
    include_str!("../themes/one-dark-pro.json"),
    include_str!("../themes/tokyo-night.json"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeError(pub String);

impl std::fmt::Display for ThemeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ThemeError {}

fn parse_hex(token: &str, value: &str) -> Result<Hsla, ThemeError> {
    let bad = || {
        ThemeError(format!(
            "color `{token}`: `{value}` is not #rrggbb or #rrggbbaa"
        ))
    };
    let digits = value.strip_prefix('#').ok_or_else(bad)?;
    if !digits.is_ascii() || !matches!(digits.len(), 6 | 8) {
        return Err(bad());
    }
    let v = u32::from_str_radix(digits, 16).map_err(|_| bad())?;
    let v = if digits.len() == 6 {
        (v << 8) | 0xff
    } else {
        v
    };
    Ok(rgba(v).into())
}

/// Parse a `#rrggbb` or `#rrggbbaa` color, e.g. an [`crate::model::Account::color`].
pub fn parse_color(value: &str) -> Option<Hsla> {
    parse_hex("color", value).ok()
}

/// Declares the token list once: the serde palette, the runtime [`Theme`] and the conversion.
macro_rules! tokens {
    ($($(#[$doc:meta])* $field:ident),+ $(,)?) => {
        /// Token name to `#rrggbb` string, as written in theme files.
        #[derive(Debug, Clone, Deserialize)]
        pub struct Palette { $($(#[$doc])* pub $field: String,)+ }

        /// A resolved theme: every semantic color token.
        #[derive(Debug, Clone)]
        pub struct Theme {
            pub name: String,
            $($(#[$doc])* pub $field: Hsla,)+
        }

        impl Theme {
            /// Resolve a parsed file into colors.
            pub fn from_spec(spec: &ThemeSpec) -> Result<Theme, ThemeError> {
                Ok(Theme {
                    name: spec.name.clone(),
                    $($field: parse_hex(stringify!($field), &spec.colors.$field)?,)+
                })
            }
        }
    };
}

tokens! {
    /// Window background.
    background,
    /// Raised surfaces: panels, inputs, buttons.
    surface,
    /// Sidebar and top bar.
    sidebar,
    border,
    text,
    text_muted,
    accent,
    /// Text on top of `accent`.
    on_accent,
    /// Selected / cursor row background.
    selection,
    hover,
    /// Background of the row under the keyboard cursor / mouse focus.
    row_cursor,
    /// Selection fill and bar.
    selected,
    /// Unread status bar.
    unread,
    spam,
    needs_reply,
    awaiting,
    follow_up,
    reminder,
    possible_spam,
    new_sender,
    urgent,
    kind,
    state_inbox,
    state_snoozed,
    state_archived,
    state_filed,
    state_deleted,
    success,
    warning,
    error,
    info,
}

/// The on-disk shape of a theme file.
#[derive(Debug, Clone, Deserialize)]
pub struct ThemeSpec {
    pub name: String,
    pub colors: Palette,
}

impl Theme {
    /// Parse a theme from JSON.
    pub fn from_json(json: &str) -> Result<Theme, ThemeError> {
        let spec: ThemeSpec = serde_json::from_str(json).map_err(|e| ThemeError(e.to_string()))?;
        Theme::from_spec(&spec)
    }

    /// Color of a triage state.
    pub fn state_color(&self, state: crate::model::TriageState) -> Hsla {
        use crate::model::TriageState::*;
        match state {
            Inbox => self.state_inbox,
            Snoozed => self.state_snoozed,
            Archived => self.state_archived,
            Filed(_) => self.state_filed,
            Deleted => self.state_deleted,
        }
    }
}

/// All known themes, in display order.
#[derive(Clone, Default)]
pub struct ThemeRegistry {
    themes: Vec<Arc<Theme>>,
}

impl ThemeRegistry {
    /// Registry holding the built-in themes.
    pub fn builtin() -> Self {
        let mut r = Self::default();
        for json in BUILTIN {
            r.add(Theme::from_json(json).expect("built-in theme is valid"));
        }
        r
    }

    /// Add a theme, replacing any with the same (case-insensitive) name.
    pub fn add(&mut self, theme: Theme) {
        let theme = Arc::new(theme);
        match self
            .themes
            .iter()
            .position(|t| t.name.eq_ignore_ascii_case(&theme.name))
        {
            Some(i) => self.themes[i] = theme,
            None => self.themes.push(theme),
        }
    }

    pub fn add_json(&mut self, json: &str) -> Result<(), ThemeError> {
        self.add(Theme::from_json(json)?);
        Ok(())
    }

    /// Load every `*.json` file in `dir` (sorted by file name). Unreadable or invalid files
    /// are skipped and reported in the returned errors; a missing directory is not an error.
    pub fn load_dir(&mut self, dir: &Path) -> Vec<ThemeError> {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut paths: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect();
        paths.sort();
        let mut errors = Vec::new();
        for path in paths {
            let result = std::fs::read_to_string(&path)
                .map_err(|e| ThemeError(e.to_string()))
                .and_then(|json| self.add_json(&json));
            if let Err(e) = result {
                errors.push(ThemeError(format!("{}: {e}", path.display())));
            }
        }
        errors
    }

    pub fn get(&self, name: &str) -> Option<Arc<Theme>> {
        self.themes
            .iter()
            .find(|t| t.name.eq_ignore_ascii_case(name))
            .cloned()
    }

    /// The named theme, else the default theme, else the first registered one.
    pub fn get_or_default(&self, name: &str) -> Arc<Theme> {
        self.get(name)
            .or_else(|| self.get(DEFAULT_THEME))
            .or_else(|| self.themes.first().cloned())
            .unwrap_or_else(default_theme)
    }

    pub fn names(&self) -> Vec<String> {
        self.themes.iter().map(|t| t.name.clone()).collect()
    }
}

/// The built-in default theme.
pub fn default_theme() -> Arc<Theme> {
    static DEFAULT: LazyLock<Arc<Theme>> = LazyLock::new(|| {
        ThemeRegistry::builtin()
            .get(DEFAULT_THEME)
            .expect("default theme is built in")
    });
    DEFAULT.clone()
}

/// Directory user theme files are read from: `$MAIL_CLASSIFIER_THEMES`, else
/// `~/.config/mail-classifier/themes`.
pub fn user_themes_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("MAIL_CLASSIFIER_THEMES") {
        return Some(PathBuf::from(dir));
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config/mail-classifier/themes"))
}

struct ThemeState {
    registry: ThemeRegistry,
    active: Arc<Theme>,
}

impl Global for ThemeState {}

/// Install the registry and activate `active_name` (unknown names fall back to the default).
pub fn install(cx: &mut App, registry: ThemeRegistry, active_name: &str) {
    let active = registry.get_or_default(active_name);
    sync_kit(cx, &active);
    cx.set_global(ThemeState { registry, active });
}

/// Install the built-in registry with the default theme unless something is installed already.
pub fn ensure_installed(cx: &mut App) {
    if !cx.has_global::<ThemeState>() {
        install(cx, ThemeRegistry::builtin(), DEFAULT_THEME);
    }
}

/// The active theme (the default one before anything is installed).
pub fn active(cx: &App) -> Arc<Theme> {
    cx.try_global::<ThemeState>()
        .map_or_else(default_theme, |s| s.active.clone())
}

/// Names of all registered themes.
pub fn names(cx: &App) -> Vec<String> {
    cx.try_global::<ThemeState>()
        .map_or_else(|| ThemeRegistry::builtin().names(), |s| s.registry.names())
}

/// Activate the named theme; an unknown name selects the default theme. Returns the theme used.
pub fn set_active(cx: &mut App, name: &str) -> Arc<Theme> {
    ensure_installed(cx);
    let theme = cx.global::<ThemeState>().registry.get_or_default(name);
    sync_kit(cx, &theme);
    cx.global_mut::<ThemeState>().active = theme.clone();
    theme
}

/// Keep the kit widgets (inputs, kbd, labels) in step with the active theme.
fn sync_kit(cx: &mut App, t: &Theme) {
    if !cx.has_global::<KitTheme>() {
        return;
    }
    let t = t.clone();
    KitTheme::update(cx, move |k| {
        let c = &mut k.colors;
        c.background = t.background;
        c.foreground = t.text;
        c.border = t.border;
        c.input = t.border;
        c.muted = t.surface;
        c.muted_foreground = t.text_muted;
        c.primary = t.accent;
        c.primary_foreground = t.on_accent;
        c.secondary = t.surface;
        c.popover = t.surface;
        c.popover_foreground = t.text;
        c.caret = t.accent;
        c.selection = t.selection;
        c.list_active = t.selection;
        c.list_hover = t.hover;
    });
}
