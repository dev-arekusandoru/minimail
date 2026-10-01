use gpui_kit::TestAppContext;
use gpui_kit::component::ActiveTheme as _;
use mail_classifier::model::TriageState;
use mail_classifier::theme::{self, DEFAULT_THEME};

fn boot(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init(cx);
    });
}

#[gpui_kit::gpui::test]
fn built_in_themes_load_and_default_is_active(cx: &mut TestAppContext) {
    boot(cx);
    cx.update(|cx| {
        let names = theme::names(cx);
        assert_eq!(names[0], DEFAULT_THEME, "the default theme leads the picker");
        assert!(names.iter().any(|n| n == "Tokyo Night"));
        assert_eq!(cx.theme().theme_name(), DEFAULT_THEME);
        assert!(cx.theme().is_dark());
    });
}

#[gpui_kit::gpui::test]
fn switching_theme_repaints_kit_colors(cx: &mut TestAppContext) {
    boot(cx);
    cx.update(|cx| {
        let one_dark = cx.theme().colors;
        assert!(theme::apply(cx, "Tokyo Night"));
        let tokyo = cx.theme().colors;
        assert_eq!(cx.theme().theme_name(), "Tokyo Night");
        assert_ne!(one_dark.background, tokyo.background);
        assert_ne!(one_dark.secondary, tokyo.secondary);
        // The kit's own tab tokens come from the theme file, not the kit's default palette.
        assert_eq!(tokyo.tab_active, tokyo.background);
        assert_eq!(tokyo.tab_bar, tokyo.secondary);
        assert_eq!(tokyo.tab_active_foreground, tokyo.foreground);

        assert!(theme::apply(cx, DEFAULT_THEME));
        assert_eq!(cx.theme().colors.background, one_dark.background);
    });
}

#[gpui_kit::gpui::test]
fn unknown_theme_changes_nothing(cx: &mut TestAppContext) {
    boot(cx);
    cx.update(|cx| {
        assert!(!theme::apply(cx, "No Such Theme"));
        assert_eq!(cx.theme().theme_name(), DEFAULT_THEME);
    });
}

#[gpui_kit::gpui::test]
fn domain_colors_stay_distinct_in_every_built_in_theme(cx: &mut TestAppContext) {
    boot(cx);
    cx.update(|cx| {
        for name in [DEFAULT_THEME, "Tokyo Night"] {
            theme::apply(cx, name);
            let c = &cx.theme().colors;

            let all = TriageState::ALL;
            for (i, a) in all.iter().enumerate() {
                for b in &all[i + 1..] {
                    assert_ne!(theme::state_color(c, *a), theme::state_color(c, *b), "{name}: {a:?} vs {b:?}");
                }
            }

            let tags = [
                ("needs_reply", theme::needs_reply(c)),
                ("follow_up", theme::follow_up(c)),
                ("reminder", theme::reminder(c)),
                ("new_sender", theme::new_sender(c)),
                ("urgent", theme::urgent(c)),
                ("spam", theme::spam(c)),
                ("awaiting", theme::awaiting(c)),
            ];
            for (i, (a, ca)) in tags.iter().enumerate() {
                for (b, cb) in &tags[i + 1..] {
                    assert_ne!(ca, cb, "{name}: {a} vs {b}");
                }
            }
        }
    });
}

#[test]
fn parse_color_accepts_hex_only() {
    assert!(theme::parse_color("#61afef").is_some());
    assert!(theme::parse_color("#61afef80").is_some());
    assert!(theme::parse_color("61afef").is_none());
    assert!(theme::parse_color("#12345").is_none());
    assert!(theme::parse_color("red").is_none());
}

#[test]
fn hex_colors_round_trip_and_the_account_palette_survives() {
    for color in mail_classifier::account_style::COLORS {
        let parsed = theme::parse_color(color).expect("palette color parses");
        assert_eq!(theme::to_hex(parsed), color);
    }
    assert_eq!(theme::to_hex(theme::parse_color("#12345678").unwrap()), "#123456", "alpha is dropped");
}
