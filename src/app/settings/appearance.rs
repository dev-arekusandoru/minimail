use super::*;

impl SettingsPanel {
    pub(super) fn appearance_page(&self, weak: &Weak, cx: &App) -> SettingPage {
        let mode_field = {
            let (get, set) = (weak.clone(), weak.clone());
            SettingField::dropdown(
                options(&[("light", "Light"), ("dark", "Dark"), ("system", "System")]),
                move |cx| {
                    let mode = get.read_with(cx, |this, _| this.theme_mode).unwrap_or_default();
                    match mode {
                        theme::ThemeMode::Light => "light".into(),
                        theme::ThemeMode::Dark => "dark".into(),
                        theme::ThemeMode::System => "system".into(),
                    }
                },
                move |value: SharedString, cx| {
                    let mode = match &*value {
                        "light" => theme::ThemeMode::Light,
                        "dark" => theme::ThemeMode::Dark,
                        _ => theme::ThemeMode::System,
                    };
                    set.update(cx, |this, cx| this.set_theme_mode(mode, cx)).ok();
                },
            )
            .default_value(SharedString::from("system"))
        };
        let light_names = theme::names_for(cx, true);
        let light_options = light_names.iter().map(|n| (n.clone(), n.clone())).collect();
        let light_field = {
            let (get, set) = (weak.clone(), weak.clone());
            SettingField::dropdown(
                light_options,
                move |cx| get.read_with(cx, |this, _| this.light_theme.clone().into()).unwrap_or_default(),
                move |value: SharedString, cx| {
                    set.update(cx, |this, cx| this.set_light_theme(value.to_string(), cx)).ok();
                },
            )
            .default_value(light_names.first().cloned().unwrap_or_default())
        };
        let dark_names = theme::names_for(cx, false);
        let dark_options = dark_names.iter().map(|n| (n.clone(), n.clone())).collect();
        let dark_field = {
            let (get, set) = (weak.clone(), weak.clone());
            SettingField::dropdown(
                dark_options,
                move |cx| get.read_with(cx, |this, _| this.dark_theme.clone().into()).unwrap_or_default(),
                move |value: SharedString, cx| {
                    set.update(cx, |this, cx| this.set_dark_theme(value.to_string(), cx)).ok();
                },
            )
            .default_value(dark_names.first().cloned().unwrap_or_default())
        };
        let layout_field = {
            let (get, set) = (weak.clone(), weak.clone());
            SettingField::dropdown(
                options(&PANE_LAYOUTS),
                move |cx| {
                    let stacked = get
                        .read_with(cx, |this, _| this.orientation == Orientation::Stacked)
                        .unwrap_or_default();
                    pick(&PANE_LAYOUTS, usize::from(stacked))
                },
                move |value: SharedString, cx| {
                    let orientation =
                        if value == "stacked" { Orientation::Stacked } else { Orientation::SideBySide };
                    set.update(cx, |this, cx| this.set_orientation(orientation, cx)).ok();
                },
            )
            .default_value(pick(&PANE_LAYOUTS, 0))
        };
        SettingPage::new("Appearance").resettable(false).group(
            SettingGroup::new()
                .item(SettingItem::new("Mode", mode_field).keywords(["appearance", "light dark system"]))
                .item(SettingItem::new("Light theme", light_field).keywords(["theme", "light appearance"]))
                .item(SettingItem::new("Dark theme", dark_field).keywords(["theme", "dark appearance"]))
                .item(
                    SettingItem::new("Pane layout", layout_field)
                        .description("Stack the message list and the reader side by side or one above the other.")
                        .keywords(["side by side", "stacked", "layout", "appearance"]),
                )
                .item(
                    SettingItem::new(
                        "Show sender avatar in tabs",
                        switch(weak, |this| this.tab_avatars, SettingsPanel::set_tab_avatars).default_value(true),
                    )
                    .description("Show the sender's monogram as each reader tab's icon.")
                    .keywords(["avatar", "monogram", "tab icon", "appearance"]),
                ),
        )
    }
}
