use super::*;

impl SettingsPanel {
    pub(super) fn blocked_page(&self, weak: &Weak) -> SettingPage {
        let keywords = ["blocked senders", "blocked", "unblock", "senders"];
        let mut blocked = SettingGroup::new().title("Blocked senders");
        if self.blocked.is_empty() {
            blocked = blocked.item(note("blocked-empty", "No blocked senders.").keywords(keywords));
        }
        for (index, email) in self.blocked.iter().cloned().enumerate() {
            let weak = weak.clone();
            let label = email.clone();
            blocked = blocked.item(
                SettingItem::render(move |_, _, cx| {
                    let (weak, email) = (weak.clone(), email.clone());
                    let t = cx.theme();
                    div()
                        .id(("blocked-row", index))
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .text_sm()
                        .child(div().flex().flex_col().child(email.clone()).child(
                            div().text_xs().text_color(t.muted_foreground).child("Blocked sender"),
                        ))
                        .child(
                            button(("blocked-unblock", index), "Unblock", "Allow mail from this sender again", "", cx)
                                .on_click(move |_, _, cx| {
                                    weak.update(cx, |this, cx| this.unblock(&email, cx)).ok();
                                }),
                        )
                })
                .keywords(keywords.into_iter().chain([label.as_str()])),
            );
        }
        SettingPage::new("Senders").resettable(false).group(blocked)
    }
}
