use super::*;
use gpui_kit::component::button::ButtonGroup;

/// Segmented theme mode control: one entry per [`theme::ThemeMode`] variant.
const THEME_MODES: [(&str, &str); 3] =
    [("mode-light", "Light"), ("mode-dark", "Dark"), ("mode-system", "System")];
/// Segmented pane layout control. The button ids are what the UI tests click.
const PANE_LAYOUTS: [(&str, &str); 2] =
    [("layout-side-by-side", "Side by side"), ("layout-stacked", "Stacked")];

/// Index of the active theme mode in [`THEME_MODES`].
fn theme_mode_index(mode: theme::ThemeMode) -> usize {
    match mode {
        theme::ThemeMode::Light => 0,
        theme::ThemeMode::Dark => 1,
        theme::ThemeMode::System => 2,
    }
}

/// The theme mode at `index` in [`THEME_MODES`].
fn theme_mode(index: usize) -> theme::ThemeMode {
    match index {
        0 => theme::ThemeMode::Light,
        1 => theme::ThemeMode::Dark,
        _ => theme::ThemeMode::System,
    }
}

/// A single-selection segmented control (one kit `ButtonGroup`): `entries` are the buttons
/// in order, `selected` reports which one is active, and `apply` stores a click. Its reset
/// button appears while `is_dirty` holds and runs `reset`.
fn segmented(
    id: &'static str,
    entries: &'static [(&'static str, &'static str)],
    weak: &Weak,
    selected: fn(&SettingsPanel) -> usize,
    apply: fn(&mut SettingsPanel, usize, &mut Context<SettingsPanel>),
    is_dirty: fn(&SettingsPanel) -> bool,
    reset: fn(&mut SettingsPanel, &mut Context<SettingsPanel>),
) -> SettingField<SharedString> {
    let (read, write, dirty, undo) = (weak.clone(), weak.clone(), weak.clone(), weak.clone());
    SettingField::render(move |_, _window, cx| {
        let active = read.read_with(cx, |this, _| selected(this)).unwrap_or_default();
        let mut group = ButtonGroup::new(id).compact();
        for (index, (button_id, label)) in entries.iter().enumerate() {
            group = group.child(Button::new(*button_id).label(*label).selected(index == active));
        }
        let write = write.clone();
        group.on_click(move |clicks, _, cx| {
            if let Some(index) = clicks.first() {
                write.update(cx, |this, cx| apply(this, *index, cx)).ok();
            }
        })
    })
    .on_reset(
        move |cx| dirty.read_with(cx, |this, _| is_dirty(this)).unwrap_or_default(),
        move |_window, cx| {
            undo.update(cx, reset).ok();
        },
    )
}

/// A picker listing the registered themes of one appearance; reset restores the first one.
fn theme_field(
    weak: &Weak,
    names: Vec<SharedString>,
    get: fn(&SettingsPanel) -> String,
    set: fn(&mut SettingsPanel, String, &mut Context<SettingsPanel>),
) -> SettingField<SharedString> {
    let options = names.iter().map(|name| (name.clone(), name.clone())).collect();
    let default = names.first().cloned().unwrap_or_default();
    let (read, write) = (weak.clone(), weak.clone());
    SettingField::dropdown(
        options,
        move |cx| read.read_with(cx, |this, _| get(this).into()).unwrap_or_default(),
        move |name: SharedString, cx| {
            write.update(cx, |this, cx| set(this, name.to_string(), cx)).ok();
        },
    )
    .default_value(default)
}

impl SettingsPanel {
    pub(super) fn appearance_page(&self, weak: &Weak, cx: &App) -> SettingPage {
        let light = self.theme_mode == theme::ThemeMode::Light;
        let dark = self.theme_mode == theme::ThemeMode::Dark;
        // System follows the OS, so both pickers stay reachable there.
        let system = self.theme_mode == theme::ThemeMode::System;
        let mut group = SettingGroup::new().item(
            SettingItem::new(
                "Mode",
                segmented(
                    "mode-group",
                    &THEME_MODES,
                    weak,
                    |this| theme_mode_index(this.theme_mode),
                    |this, index, cx| this.set_theme_mode(theme_mode(index), cx),
                    |this| this.theme_mode != theme::ThemeMode::System,
                    |this, cx| this.set_theme_mode(theme::ThemeMode::System, cx),
                ),
            )
            .description("Follow the system appearance or pick light or dark.")
            .keywords(["appearance", "light dark system"]),
        );
        if light || system {
            group = group.item(
                SettingItem::new(
                    "Light theme",
                    theme_field(
                        weak,
                        theme::names_for(cx, true),
                        |this| this.light_theme.clone(),
                        SettingsPanel::set_light_theme,
                    ),
                )
                .description("Theme used while the app is light.")
                .keywords(["theme", "light appearance"]),
            );
        }
        if dark || system {
            group = group.item(
                SettingItem::new(
                    "Dark theme",
                    theme_field(
                        weak,
                        theme::names_for(cx, false),
                        |this| this.dark_theme.clone(),
                        SettingsPanel::set_dark_theme,
                    ),
                )
                .description("Theme used while the app is dark.")
                .keywords(["theme", "dark appearance"]),
            );
        }
        group = group
            .item(
                SettingItem::new(
                    "Pane layout",
                    segmented(
                        "layout-group",
                        &PANE_LAYOUTS,
                        weak,
                        |this| usize::from(this.orientation == Orientation::Stacked),
                        |this, index, cx| {
                            let orientation = if index == 0 {
                                Orientation::SideBySide
                            } else {
                                Orientation::Stacked
                            };
                            this.set_orientation(orientation, cx);
                        },
                        |this| this.orientation != Orientation::SideBySide,
                        |this, cx| this.set_orientation(Orientation::SideBySide, cx),
                    ),
                )
                .description("Show the reader beside the list or below it.")
                .keywords(["side by side", "stacked", "layout", "appearance"]),
            )
            .item(
                SettingItem::new(
                    "Sender avatar in tabs",
                    switch(
                        weak,
                        |this| this.tab_avatars,
                        SettingsPanel::set_tab_avatars,
                    )
                    .default_value(true),
                )
                .description("Use the sender's initial as each reader tab's icon.")
                .keywords(["avatar", "monogram", "tab icon", "appearance"]),
            );
        SettingPage::new("Appearance").resettable(false).group(group)
    }
}
