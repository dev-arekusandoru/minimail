use super::*;

impl SettingsPanel {
    pub(super) fn inbox_page(&self, weak: &Weak) -> SettingPage {
        let preview_options = (0..=MAX_PREVIEW_LINES)
            .map(|lines| (SharedString::from(lines.to_string()), SharedString::from(preview::lines_label(lines))))
            .collect();
        let preview_field = {
            let (get, set) = (weak.clone(), weak.clone());
            SettingField::dropdown(
                preview_options,
                move |cx| {
                    let lines = get.read_with(cx, |this, _| this.preview_lines).unwrap_or_default();
                    SharedString::from(lines.to_string())
                },
                move |value: SharedString, cx| {
                    if let Ok(lines) = value.parse::<u8>() {
                        set.update(cx, |this, cx| this.set_preview_lines(lines, cx)).ok();
                    }
                },
            )
            .default_value(SharedString::from(crate::preview::DEFAULT_LINES.to_string()))
        };
        let follow_up_field = {
            let (get, set) = (weak.clone(), weak.clone());
            SettingField::number_input(
                NumberFieldOptions {
                    min: f64::from(*FOLLOW_UP_DAYS.start()),
                    max: f64::from(*FOLLOW_UP_DAYS.end()),
                    step: 1.,
                },
                move |cx| get.read_with(cx, |this, _| f64::from(this.follow_up_days)).unwrap_or_default(),
                move |days, cx| {
                    set.update(cx, |this, cx| this.set_follow_up(days, cx)).ok();
                },
            )
            .default_value(3.)
        };
        SettingPage::new("Inbox").resettable(false).group(
            SettingGroup::new()
                .item(
                    SettingItem::new(
                        "Group by thread",
                        switch(weak, |this| this.group, SettingsPanel::set_grouping).default_value(false),
                    )
                    .description("Show one inbox row per conversation instead of per message.")
                    .keywords(["inbox", "threads"]),
                )
                .item(
                    SettingItem::new("Preview lines", preview_field)
                        .description("Snippet lines shown under each subject in the inbox.")
                        .keywords(["message snippet", "inbox"]),
                )
                .item(
                    SettingItem::new("Follow-up after", follow_up_field)
                        .description("Days to wait for a reply before flagging (1–14).")
                        .keywords(["follow up", "wait for a response", "flag", "days"]),
                ),
        )
    }
}
