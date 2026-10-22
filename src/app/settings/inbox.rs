use super::*;
use gpui_kit::base::component_traits::Disableable as _;

impl SettingsPanel {
    pub(super) fn inbox_page(&self, weak: &Weak) -> SettingPage {
        // "Remind me if no reply after [N] days": a number input with a `days` suffix, so
        // the row reads as one sentence. Reset restores the default number of days.
        let follow_up = number_field(
            weak,
            SettingsPanel::follow_up_row,
            self.follow_up.input.clone(),
            Some("days"),
        );
        let preview = number_field(weak, SettingsPanel::preview_row, self.preview.input.clone(), None);
        SettingPage::new("Inbox").resettable(true).group(
            SettingGroup::new()
                .item(row(
                    "Group by thread",
                    "Show one inbox row per conversation instead of per message.",
                    &["inbox", "threads"],
                    Undo::of_panel(
                        weak,
                        |this| this.group,
                        SettingsPanel::reset_grouping,
                    ),
                    switch(weak, |this| this.group, SettingsPanel::set_grouping),
                ))
                .item(row(
                    "Preview lines",
                    "Snippet lines shown under each subject; 0 hides them.",
                    &["message snippet", "inbox", "preview"],
                    Undo::of_panel(
                        weak,
                        |this| this.preview_lines != preview::DEFAULT_LINES,
                        SettingsPanel::reset_preview_lines,
                    ),
                    preview,
                ))
                .item(row(
                    "Remind me if no reply after",
                    "Days to wait for an answer before flagging the thread (1–14).",
                    &["follow up", "wait for a response", "flag", "days"],
                    Undo::of_panel(
                        weak,
                        SettingsPanel::follow_up_is_modified,
                        SettingsPanel::reset_follow_up,
                    ),
                    follow_up,
                )),
        )
    }
}

impl SettingsPanel {
    /// How message bodies are shown.
    pub(super) fn reader_page(&self, weak: &Weak) -> SettingPage {
        SettingPage::new("Reader").resettable(true).group(
            SettingGroup::new()
                .item(row(
                    "Block remote images",
                    "Remote images can tell the sender you opened the message. Block them to stop \
                     tracking; images load by default.",
                    &["images", "remote images", "tracking", "privacy", "pixel", "block"],
                    Undo::of_panel(
                        weak,
                        |this| this.block_remote_images,
                        SettingsPanel::reset_block_remote_images,
                    ),
                    switch(weak, |this| this.block_remote_images, SettingsPanel::set_block_remote_images),
                )),
        )
    }
}

impl SettingsPanel {
    /// This build's update status with its button (`mockups/updates-default.html`), then the
    /// daily check.
    pub(super) fn updates_page(&self, weak: &Weak) -> SettingPage {
        let status = self.update_status.clone();
        let button_weak = weak.clone();
        let status_row = SettingItem::render(move |_, _, cx| {
            use crate::update::Tone;
            let theme = cx.theme();
            let lead_color = match status.tone {
                Tone::Muted => theme.muted_foreground,
                Tone::Success => theme.success,
                Tone::Accent => theme.primary,
                Tone::Error => theme.danger,
            };
            let button = status.button;
            let weak = button_weak.clone();
            // Same text column + control as `row`, minus the ↺: the status isn't a preference.
            div()
                .w_full()
                .h_flex()
                .justify_between()
                .items_center()
                .gap_3()
                .child(
                    v_flex()
                        .flex_1()
                        .max_w_3_5()
                        .child(Label::new(status.title.clone()).text_sm())
                        .child(
                            div()
                                .text_sm()
                                .text_color(theme.muted_foreground)
                                .child(div().text_color(lead_color).child(status.lead.clone()))
                                .when_some(status.detail.clone(), |this, detail| this.child(format!(" · {detail}")))
                                .h_flex(),
                        ),
                )
                .child(
                    div().id("field").child(
                        Button::new("update-button")
                            .label(button.label())
                            .small()
                            .when(button.installs(), |b| b.primary())
                            .loading(button.busy())
                            .disabled(button.busy() || button == crate::update::UpdateButton::Unavailable)
                            .on_click(move |_, _, cx| {
                                weak.update(cx, |_, cx| cx.emit(SettingsEvent::UpdateButton)).ok();
                            }),
                    ),
                )
        })
        .keywords(["update", "version", "check", "nightly", "release", "upgrade"]);
        SettingPage::new("Updates").resettable(true).groups([
            SettingGroup::new().item(status_row),
            SettingGroup::new().item(row(
                "Check for updates automatically",
                "Once a day, in the background.",
                &["update", "nightly", "release", "upgrade"],
                Undo::of_panel(weak, |this| !this.auto_update_check, SettingsPanel::reset_auto_update_check),
                switch(weak, |this| this.auto_update_check, SettingsPanel::set_auto_update_check),
            )),
        ])
    }
}
