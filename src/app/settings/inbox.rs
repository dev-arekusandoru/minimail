use super::*;
use gpui_kit::component::input::NumberInput;

impl SettingsPanel {
    /// "Remind me if no reply after [N] days": a number input with a `days` suffix, so the
    /// row reads as one sentence. Reset restores the default number of days.
    fn follow_up_field(&self, weak: &Weak) -> SettingField<SharedString> {
        let input = self.follow_up_input.clone();
        let (write, dirty, undo) = (weak.clone(), weak.clone(), weak.clone());
        SettingField::render(move |_, window, cx| {
            // Only a change made outside the field (a reset) is written back; pushing the
            // panel's text on every render would overwrite what the user is typing.
            if let Some(text) = write.update(cx, |this, _| this.take_follow_up_write_back()).ok().flatten()
            {
                input.update(cx, |input, cx| input.set_value(SharedString::from(text), window, cx));
            }
            let muted = cx.theme().muted;
            NumberInput::new(&input).suffix(div().text_sm().text_color(muted).child("days"))
        })
        .on_reset(
            move |cx| {
                dirty.read_with(cx, |this, _| this.follow_up_is_modified()).unwrap_or_default()
            },
            move |_window, cx| {
                undo.update(cx, |this, cx| this.reset_follow_up(cx)).ok();
            },
        )
    }

    pub(super) fn inbox_page(&self, weak: &Weak) -> SettingPage {
        let preview_field = {
            let (read, write) = (weak.clone(), weak.clone());
            SettingField::number_input(
                NumberFieldOptions { min: 0., max: f64::from(MAX_PREVIEW_LINES), step: 1. },
                move |cx| read.read_with(cx, |this, _| f64::from(this.preview_lines)).unwrap_or_default(),
                move |lines: f64, cx| {
                    let lines = lines.round().clamp(0., f64::from(MAX_PREVIEW_LINES)) as u8;
                    write.update(cx, |this, cx| this.set_preview_lines(lines, cx)).ok();
                },
            )
            .default_value(f64::from(preview::DEFAULT_LINES))
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
                        .description("Snippet lines shown under each subject; 0 hides them.")
                        .keywords(["message snippet", "inbox", "preview"]),
                )
                .item(
                    SettingItem::new("Remind me if no reply after", self.follow_up_field(weak))
                        .description("Days to wait for an answer before flagging the thread (1–14).")
                        .keywords(["follow up", "wait for a response", "flag", "days"]),
                ),
        )
    }
}
