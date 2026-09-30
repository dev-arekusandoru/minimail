//! Color themes: semantic tokens, a registry of built-in and user themes, and the active theme.
//!
//! Themes are plain data (JSON): `{ "name": "…", "colors": { "<token>": "#rrggbb", … } }`.
//! Built-ins live in `themes/*.json` and are embedded; adding one is a new file plus one line in
//! [`BUILTIN`]. User themes are loaded from a directory of `*.json` files (see
//! [`user_themes_dir`]). Views read colors only through [`active`].

use gpui_kit::component::theme::{Colorize as _, Theme as KitTheme};
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

/// How far a solid fill lightens for its hover shade, and darkens for its pressed one.
const HOVER: f32 = 0.10;
const ACTIVE: f32 = 0.14;

/// Keep the kit widgets (inputs, kbd, labels, tabs, buttons, dialogs) in step with the active
/// theme.
///
/// gpui-kit falls back to its own default palette for any token left alone — which is why the
/// reader tabs looked foreign, since their `tab*` tokens were never copied. Map every token a
/// component this app uses can read. Neutral hover and pressed shades reuse our own
/// [`Theme::hover`] and [`Theme::selection`]; a solid accent or semantic fill derives its own
/// by a lightness step.
fn sync_kit(cx: &mut App, t: &Theme) {
    if !cx.has_global::<KitTheme>() {
        return;
    }
    let t = t.clone();
    KitTheme::update(cx, move |k| {
        let c = &mut k.colors;

        // Window and shared text.
        c.background = t.background;
        c.foreground = t.text;
        c.border = t.border;
        c.window_border = t.border;
        c.overlay = rgba(0x00000059).into();

        // Neutral interactions: our own hover and selection shades.
        c.accent = t.hover;
        c.accent_foreground = t.text;
        c.secondary = t.surface;
        c.secondary_foreground = t.text;
        c.secondary_hover = t.hover;
        c.secondary_active = t.selection;
        c.muted = t.surface;
        c.muted_foreground = t.text_muted;
        c.selection = t.selection;
        c.caret = t.accent;
        c.ring = t.accent;

        // Brand accent: solid primary fills and links.
        c.primary = t.accent;
        c.primary_foreground = t.on_accent;
        c.primary_hover = t.accent.lighten(HOVER);
        c.primary_active = t.accent.darken(ACTIVE);
        c.link = t.accent;
        c.link_hover = t.accent.lighten(HOVER);
        c.link_active = t.accent.darken(ACTIVE);

        // Inputs.
        c.input = t.border;

        // Popovers, menus, palettes and lists (tooltips land here too).
        c.popover = t.surface;
        c.popover_foreground = t.text;
        c.list = t.background;
        c.list_active = t.selection;
        c.list_active_border = t.accent;
        c.list_hover = t.hover;
        c.list_head = t.surface;
        c.list_even = t.surface;

        // Buttons: the default, secondary and ghost variants the app builds, plus primary.
        c.button = t.surface;
        c.button_foreground = t.text;
        c.button_hover = t.hover;
        c.button_active = t.selection;
        c.button_primary = t.accent;
        c.button_primary_foreground = t.on_accent;
        c.button_primary_hover = t.accent.lighten(HOVER);
        c.button_primary_active = t.accent.darken(ACTIVE);
        c.button_secondary = t.surface;
        c.button_secondary_foreground = t.text;
        c.button_secondary_hover = t.hover;
        c.button_secondary_active = t.selection;

        // Semantic fills, each with its on-colour, hover and pressed shades. The fill is bright
        // enough in every theme for the ink used on the accent.
        c.danger = t.error;
        c.danger_foreground = t.on_accent;
        c.danger_hover = t.error.lighten(HOVER);
        c.danger_active = t.error.darken(ACTIVE);
        c.button_danger = t.error;
        c.button_danger_foreground = t.on_accent;
        c.button_danger_hover = t.error.lighten(HOVER);
        c.button_danger_active = t.error.darken(ACTIVE);
        c.warning = t.warning;
        c.warning_foreground = t.on_accent;
        c.warning_hover = t.warning.lighten(HOVER);
        c.warning_active = t.warning.darken(ACTIVE);
        c.button_warning = t.warning;
        c.button_warning_foreground = t.on_accent;
        c.button_warning_hover = t.warning.lighten(HOVER);
        c.button_warning_active = t.warning.darken(ACTIVE);
        c.success = t.success;
        c.success_foreground = t.on_accent;
        c.success_hover = t.success.lighten(HOVER);
        c.success_active = t.success.darken(ACTIVE);
        c.button_success = t.success;
        c.button_success_foreground = t.on_accent;
        c.button_success_hover = t.success.lighten(HOVER);
        c.button_success_active = t.success.darken(ACTIVE);
        c.info = t.info;
        c.info_foreground = t.on_accent;
        c.info_hover = t.info.lighten(HOVER);
        c.info_active = t.info.darken(ACTIVE);
        c.button_info = t.info;
        c.button_info_foreground = t.on_accent;
        c.button_info_hover = t.info.lighten(HOVER);
        c.button_info_active = t.info.darken(ACTIVE);

        // Tabs: the strip and inactive tabs take the chrome surface, the active tab opens onto
        // the reader's own background.
        c.tab_bar = t.surface;
        c.tab_bar_segmented = t.surface;
        c.tab = t.surface;
        c.tab_active = t.background;
        c.tab_foreground = t.text_muted;
        c.tab_active_foreground = t.text;

        // Title bar, status bar and sidebar chrome.
        c.title_bar = t.sidebar;
        c.title_bar_border = t.border;
        c.status_bar = t.sidebar;
        c.status_bar_border = t.border;
        c.sidebar = t.sidebar;
        c.sidebar_foreground = t.text;
        c.sidebar_border = t.border;
        c.sidebar_accent = t.hover;
        c.sidebar_accent_foreground = t.text;
        c.sidebar_primary = t.accent;
        c.sidebar_primary_foreground = t.on_accent;

        // Switch, skeleton and scrollbar.
        c.switch = t.border;
        c.switch_thumb = t.text;
        c.skeleton = t.surface;
        c.scrollbar = t.background.opacity(0.0);
        c.scrollbar_thumb = t.border;
        c.scrollbar_thumb_hover = t.text_muted;
    });
}
