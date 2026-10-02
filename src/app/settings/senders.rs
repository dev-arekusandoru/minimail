use super::*;

impl SettingsPanel {
    pub(super) fn blocked_page(&self, weak: &Weak) -> SettingPage {
        let keywords = ["blocked senders", "blocked", "unblock", "senders", "filter"];
        let mut blocked = SettingGroup::new().title("Blocked senders");
        let filter = self.filter_input.clone();
        blocked = blocked.item(
            SettingItem::render(move |_, _, _| {
                Input::new(&filter).id("blocked-filter").cleanable(true).small().w_full()
            })
            .keywords(["filter", "search blocked senders"]),
        );
        let shown = self.filtered_blocked();
        if self.blocked.is_empty() {
            blocked = blocked
                .item(note("blocked-empty", "No blocked senders.").keywords(keywords))
                .item(
                    note(
                        "blocked-empty-hint",
                        "Blocking a sender — with s on a thread, or when you mark their mail as spam — hides their mail and stops it being classified.",
                    )
                    .keywords(keywords),
                );
        } else if shown.is_empty() {
            let filter = self.filter.clone();
            blocked = blocked.item(
                SettingItem::render(move |_, _, cx| {
                    div()
                        .id("blocked-no-match")
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("No blocked sender matches “{filter}”."))
                })
                .keywords(keywords),
            );
        }
        for (email, at) in shown {
            let weak = weak.clone();
            let email = email.clone();
            let address = email.clone();
            let since = crate::model::blocked_ago(&self.now, *at);
            blocked = blocked.item(
                SettingItem::render(move |_, _, cx| {
                    let (weak, email) = (weak.clone(), email.clone());
                    let t = cx.theme();
                    div()
                        .id(format!("blocked-row-{email}"))
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .text_sm()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .min_w_0()
                                .child(email.clone())
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(t.muted_foreground)
                                        .child(since.clone()),
                                ),
                        )
                        .child(
                            Button::new(format!("blocked-unblock-{email}"))
                                .label("Unblock")
                                .small()
                                .tooltip("Allow mail from this sender again")
                                .on_click(move |_, _, cx| {
                                    weak.update(cx, |this, cx| this.unblock(&email, cx)).ok();
                                }),
                        )
                })
                .keywords(keywords.into_iter().chain([address.as_str()])),
            );
        }
        SettingPage::new("Senders").resettable(false).group(blocked)
    }
}
