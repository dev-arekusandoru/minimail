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
                ))
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