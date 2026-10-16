//! Built-in, typed global preferences. This module is independent of GPUI.
use crate::{judge::{Confidence, Mode}, prefs::Setting, theme::ThemeMode};

pub const THEME_MODE: Setting<ThemeMode> = Setting::new("theme_mode", ThemeMode::System);
pub const LIGHT_THEME: Setting<String> =
    Setting::with_default_fn("light_theme", String::new(), default_light_theme);
pub const DARK_THEME: Setting<String> =
    Setting::with_default_fn("dark_theme", String::new(), default_dark_theme);
pub const PANE_LAYOUT: Setting<String> =
    Setting::with_default_fn("pane_layout", String::new(), default_pane_layout);
pub const TAB_AVATARS: Setting<bool> = Setting::new("tab_avatars", true);
pub const GROUP_THREADS: Setting<bool> = Setting::new("group_threads", false);
pub const BLOCK_REMOTE_IMAGES: Setting<bool> = Setting::new("block_remote_images", false);
pub const PREVIEW_LINES: Setting<u8> = Setting::new("preview_lines", crate::preview::DEFAULT_LINES);
pub const FOLLOW_UP_DAYS: Setting<u8> = Setting::new("follow_up_days", 3);
pub const SUMMARIES: Setting<bool> = Setting::new("summaries", false);
pub const MODE_SPAM: Setting<Mode> = Setting::new("mode_spam", Mode::Auto(Confidence::High));
pub const MODE_NEEDS_REPLY: Setting<Mode> =
    Setting::new("mode_needs_reply", Mode::Auto(Confidence::Medium));
pub const MODE_URGENCY: Setting<Mode> =
    Setting::new("mode_urgency", Mode::Auto(Confidence::Medium));
pub const MODE_KIND: Setting<Mode> = Setting::new("mode_kind", Mode::Auto(Confidence::Low));
pub const MODE_EXPECTS_REPLY: Setting<Mode> = Setting::new("mode_expects_reply", Mode::Review);
pub const MODE_FOLDER: Setting<Mode> = Setting::new("mode_folder", Mode::Review);
/// Classify new mail automatically on arrival/at startup. Off means Jev only runs on demand.
pub const AUTO_CLASSIFY: Setting<bool> = Setting::new("auto_classify", false);
/// Count recruiters' and sales cold outreach as spam.
pub const SPAM_COLD_OUTREACH: Setting<bool> = Setting::new("spam_cold_outreach", false);
/// Jev endpoint; empty means [`crate::judge::jev::DEFAULT_BASE_URL`].
pub const JEV_BASE_URL: Setting<String> =
    Setting::with_default_fn("jev_base_url", String::new(), default_jev_base_url);
/// Jev model alias; empty means [`crate::judge::jev::DEFAULT_MODEL`].
pub const JEV_MODEL: Setting<String> =
    Setting::with_default_fn("jev_model", String::new(), default_jev_model);

pub const DEFAULT_LIGHT_THEME: &str = "Default Light";
pub const DEFAULT_DARK_THEME: &str = crate::theme::DEFAULT_THEME;
pub const DEFAULT_PANE_LAYOUT: &str = "side_by_side";

fn default_light_theme() -> String {
    DEFAULT_LIGHT_THEME.to_owned()
}

fn default_dark_theme() -> String {
    DEFAULT_DARK_THEME.to_owned()
}

fn default_pane_layout() -> String {
    DEFAULT_PANE_LAYOUT.to_owned()
}

fn default_jev_base_url() -> String {
    crate::judge::jev::DEFAULT_BASE_URL.to_owned()
}

fn default_jev_model() -> String {
    crate::judge::jev::DEFAULT_MODEL.to_owned()
}
