use mail_classifier::model::TriageState;
use mail_classifier::theme::{DEFAULT_THEME, Theme, ThemeRegistry, default_theme};

fn custom(name: &str, background: &str) -> String {
    let mut v: serde_json::Value =
        serde_json::from_str(include_str!("../themes/one-dark-pro.json")).unwrap();
    v["name"] = name.into();
    v["colors"]["background"] = background.into();
    v.to_string()
}

#[test]
fn default_theme_is_atom_one_dark_pro() {
    let t = default_theme();
    assert_eq!(t.name, DEFAULT_THEME);
    assert_eq!(t.background, gpui_kit::rgb(0x282c34).into());
    assert_eq!(t.text, gpui_kit::rgb(0xabb2bf).into());
}

#[test]
fn builtin_registry_has_one_dark_and_tokyo_night() {
    let names = ThemeRegistry::builtin().names();
    assert!(names.contains(&"One Dark Pro".to_owned()));
    assert!(names.contains(&"Tokyo Night".to_owned()));
}

#[test]
fn lookup_is_case_insensitive_and_unknown_falls_back() {
    let r = ThemeRegistry::builtin();
    assert_eq!(r.get("tokyo night").unwrap().name, "Tokyo Night");
    assert!(r.get("nope").is_none());
    assert_eq!(r.get_or_default("nope").name, DEFAULT_THEME);
}

#[test]
fn rejects_bad_json_and_bad_colors() {
    assert!(Theme::from_json("not json").is_err());
    assert!(Theme::from_json(&custom("Bad", "#12345")).is_err());
    assert!(Theme::from_json(&custom("Bad", "red")).is_err());
    assert!(Theme::from_json(r#"{"name":"x","colors":{}}"#).is_err());
}

#[test]
fn user_theme_replaces_same_name_and_extends_registry() {
    let mut r = ThemeRegistry::builtin();
    let before = r.names().len();
    r.add_json(&custom("Mine", "#010203")).unwrap();
    assert_eq!(r.names().len(), before + 1);
    r.add_json(&custom("mine", "#040506")).unwrap();
    assert_eq!(r.names().len(), before + 1);
    assert_eq!(r.get("Mine").unwrap().background, gpui_kit::rgb(0x040506).into());
}

#[test]
fn load_dir_skips_invalid_files_and_missing_dir_is_fine() {
    let dir = std::env::temp_dir().join(format!("mc-themes-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("good.json"), custom("Good", "#0a0b0c")).unwrap();
    std::fs::write(dir.join("bad.json"), "{").unwrap();
    std::fs::write(dir.join("ignored.txt"), "x").unwrap();
    let mut r = ThemeRegistry::builtin();
    let errors = r.load_dir(&dir);
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(errors.len(), 1);
    assert!(r.get("Good").is_some());
    assert!(ThemeRegistry::builtin().load_dir(&dir).is_empty());
}

#[test]
fn state_colors_are_distinct_per_state() {
    let t = default_theme();
    let all = [TriageState::Inbox, TriageState::Waiting, TriageState::Later, TriageState::Done];
    for (i, a) in all.iter().enumerate() {
        for b in &all[i + 1..] {
            assert_ne!(t.state_color(*a), t.state_color(*b));
        }
    }
}
