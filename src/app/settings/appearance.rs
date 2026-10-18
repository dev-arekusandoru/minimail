use super::*;

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

impl SettingsPanel {
    pub(super) fn appearance_page(&self, weak: &Weak, cx: &App) -> SettingPage {
        let light = self.theme_mode == theme::ThemeMode::Light;
        let dark = self.theme_mode == theme::ThemeMode::Dark;
        // System follows the OS, so both pickers stay reachable there.
        let system = self.theme_mode == theme::ThemeMode::System;
        let mut group = SettingGroup::new().item(row(
            "Mode",
            "Follow the system appearance or pick light or dark.",
            &["appearance", "light dark system"],
            Undo::of_panel(
                weak,
                |this| this.theme_mode != theme::ThemeMode::System,
                |this, cx| this.set_theme_mode(theme::ThemeMode::System, cx),
            ),
            segmented(
                "mode-group",
                segments(&THEME_MODES),
                weak,
                |this| theme_mode_index(this.theme_mode),
                |this, index, cx| this.set_theme_mode(theme_mode(index), cx),
            ),
        ));
        if light || system {
            group = group.item(row(
                "Light theme",
                "Theme used while the app is light.",
                &["theme", "light appearance"],
                theme_undo(
                    weak,
                    |this| this.light_theme.clone(),
                    crate::app_settings::DEFAULT_LIGHT_THEME,
                    SettingsPanel::set_light_theme,
                ),
                theme_picker(
                    weak,
                    theme::names_for(cx, true),
                    |this| this.light_theme.clone(),
                    SettingsPanel::set_light_theme,
                ),
            ));
        }
        if dark || system {
            group = group.item(row(
                "Dark theme",
                "Theme used while the app is dark.",
                &["theme", "dark appearance"],
                theme_undo(
                    weak,
                    |this| this.dark_theme.clone(),
                    crate::app_settings::DEFAULT_DARK_THEME,
                    SettingsPanel::set_dark_theme,
                ),
                theme_picker(
                    weak,
                    theme::names_for(cx, false),
                    |this| this.dark_theme.clone(),
                    SettingsPanel::set_dark_theme,
                ),
            ));
        }
        group = group
            .item(row(
                "Pane layout",
                "Show the reader beside the list or below it.",
                &["side by side", "stacked", "layout", "appearance"],
                Undo::of_panel(
                    weak,
                    |this| this.orientation != Orientation::SideBySide,
                    |this, cx| this.set_orientation(Orientation::SideBySide, cx),
                ),
                segmented(
                    "layout-group",
                    segments(&PANE_LAYOUTS),
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
                ),
            ))
            .item(row(
                "Sender avatar in tabs",
                "Use the sender's initial as each reader tab's icon.",
                &["avatar", "monogram", "tab icon", "appearance"],
                switch_undo(
                    weak,
                    |this| this.tab_avatars,
                    true,
                    SettingsPanel::set_tab_avatars,
                ),
                switch(weak, |this| this.tab_avatars, SettingsPanel::set_tab_avatars),
            ));
        SettingPage::new("Appearance")
            .resettable(true)
            .group(group)
            .group(self.message_list_group(weak))
    }

    /// The "Message list" group: row density and, for Standard rows only, the options that
    /// need room. Options a `RowStyle` ignores are not offered, so a collision cannot be picked.
    fn message_list_group(&self, weak: &Weak) -> SettingGroup {
        use crate::row_style::{Chips, Density, GroupTile, Leading, RowStyle};
        let standard = self.row_style.density == Density::Standard;
        let mut group = SettingGroup::new().title("Message list").item(style_row(
            weak,
            "Row density",
            "Standard rows show a preview; compact rows fit one line.",
            &["density", "compact", "rows", "list"],
            "density-group",
            &[("density-standard", "Standard"), ("density-compact", "Compact")],
            &[Density::Standard, Density::Compact],
            |s| s.density,
            |s, v| s.density = v,
        ));
        if standard {
            group = group.item(style_row(
                weak,
                "Leading element",
                "The timeline rail with the time, or a tile for the sender.",
                &["rail", "timeline", "tiles", "avatar", "list"],
                "leading-group",
                &[("leading-rail", "Timeline rail"), ("leading-tiles", "Sender tiles")],
                &[Leading::Rail, Leading::Tiles],
                |s| s.leading,
                |s, v| s.leading = v,
            ));
        }
        if standard && self.row_style.leading == Leading::Tiles {
            group = group.item(style_row(
                weak,
                "Group tile style",
                "How a thread with several senders draws its tile.",
                &["ring", "deck", "mosaic", "pair", "thread", "tiles"],
                "group-tile-group",
                &[
                    ("tile-ring", "Ring"),
                    ("tile-deck", "Deck"),
                    ("tile-mosaic", "Mosaic"),
                    ("tile-pair", "Pair"),
                ],
                &[GroupTile::Ring, GroupTile::Deck, GroupTile::Mosaic, GroupTile::Pair],
                |s| s.group_tile,
                |s, v| s.group_tile = v,
            ));
        }
        if standard {
            group = group
                .item(style_row(
                    weak,
                    "Chips",
                    "Suggested folder and urgency as labels",
                    &["chips", "folder", "urgency", "labels", "list"],
                    "chips-group",
                    &[("chips-off", "Off"), ("chips-above", "Above"), ("chips-below", "Below")],
                    &[Chips::Off, Chips::Above, Chips::Below],
                    |s| s.chips,
                    |s, v| s.chips = v,
                ))
                .item(row(
                    "Subject first",
                    "Put the subject on the first line and the sender below it.",
                    &["subject", "sender", "order", "list"],
                    switch_undo(
                        weak,
                        |this| this.row_style.subject_first,
                        RowStyle::default().subject_first,
                        |this, on, cx| this.edit_row_style(|s| s.subject_first = on, cx),
                    ),
                    switch(weak, |this| this.row_style.subject_first, |this, on, cx| {
                        this.edit_row_style(|s| s.subject_first = on, cx)
                    }),
                ));
        }
        group
    }
}

/// A segmented row over one `RowStyle` field: `values[i]` is what button `entries[i]` selects,
/// and the row's undo puts the field back to its `RowStyle::default()` value.
#[allow(clippy::too_many_arguments)]
fn style_row<T: Copy + PartialEq + 'static>(
    weak: &Weak,
    title: &'static str,
    description: &'static str,
    keywords: &[&'static str],
    group: &'static str,
    entries: &'static [(&'static str, &'static str)],
    values: &'static [T],
    get: fn(&crate::row_style::RowStyle) -> T,
    set: fn(&mut crate::row_style::RowStyle, T),
) -> SettingItem {
    let default = get(&crate::row_style::RowStyle::default());
    row(
        title,
        description,
        keywords,
        Undo::of_panel(
            weak,
            move |this| get(&this.row_style) != default,
            move |this, cx| this.edit_row_style(|s| set(s, default), cx),
        ),
        segmented(
            group,
            segments(entries),
            weak,
            move |this| values.iter().position(|v| *v == get(&this.row_style)).unwrap_or(0),
            move |this, index, cx| {
                if let Some(value) = values.get(index) {
                    this.edit_row_style(|s| set(s, *value), cx);
                }
            },
        ),
    )
}

/// The theme picker of one appearance: a dropdown over the registered theme names.
fn theme_picker(
    weak: &Weak,
    names: Vec<SharedString>,
    get: fn(&SettingsPanel) -> String,
    set: fn(&mut SettingsPanel, String, &mut Context<SettingsPanel>),
) -> Control {
    select(theme_entries(&names), theme_reader(weak, get), theme_writer(weak, set))
}

/// The undo of a theme row: a reset puts back the app's default theme for it.
fn theme_undo(
    weak: &Weak,
    get: fn(&SettingsPanel) -> String,
    default: &str,
    set: fn(&mut SettingsPanel, String, &mut Context<SettingsPanel>),
) -> Undo {
    Undo::of_value(weak, get, set, default.to_owned())
}

/// The name/value pairs a theme dropdown offers.
fn theme_entries(names: &[SharedString]) -> Vec<(SharedString, SharedString)> {
    names.iter().map(|name| (name.clone(), name.clone())).collect()
}

/// Reads the theme a dropdown row shows.
fn theme_reader(
    weak: &Weak,
    get: fn(&SettingsPanel) -> String,
) -> Read {
    let weak = weak.clone();
    Rc::new(move |cx| match weak.read_with(cx, |this, _| get(this)) {
        Ok(name) => name.into(),
        Err(_) => SharedString::default(),
    })
}

/// Stores the theme a dropdown row picked.
fn theme_writer(
    weak: &Weak,
    set: fn(&mut SettingsPanel, String, &mut Context<SettingsPanel>),
) -> Write {
    let weak = weak.clone();
    Rc::new(move |name: SharedString, cx: &mut App| {
        weak.update(cx, |this, cx| set(this, name.to_string(), cx)).ok();
    })
}
