use super::*;

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

/// The Updates row's description: this build's version and channel.
static UPDATES_ABOUT: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| match crate::update::current() {
        Some(build) => format!("Minimail {} ({}). Checks once a day.", build.version, build.channel.label()),
        None => "Development builds don't check for updates.".to_owned(),
    });

impl SettingsPanel {
    /// The daily update check.
    pub(super) fn updates_page(&self, weak: &Weak) -> SettingPage {
        SettingPage::new("Updates").resettable(true).group(
            SettingGroup::new().item(row(
                "Check for updates automatically",
                &UPDATES_ABOUT,
                &["update", "version", "nightly", "release", "upgrade"],
                Undo::of_panel(weak, |this| !this.auto_update_check, SettingsPanel::reset_auto_update_check),
                switch(weak, |this| this.auto_update_check, SettingsPanel::set_auto_update_check),
            )),
        )
    }
}
