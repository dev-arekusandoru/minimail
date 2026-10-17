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
            blocked = blocked.item(
                SettingItem::render(|_, _, _| {
                    div().id("blocked-empty").child(crate::app::ui::empty(
                        IconName::ShieldBan,
                        "No blocked senders",
                        "Blocking a sender — with s on a thread, or when you mark their mail as spam — hides their mail and stops it being classified.",
                    ))
                })
                .keywords(keywords),
            );
        } else if shown.is_empty() {
            let filter = self.filter.clone();
            blocked = blocked.item(
                SettingItem::render(move |_, _, _| {
                    div().id("blocked-no-match").child(crate::app::ui::empty(
                        IconName::SearchX,
                        format!("No blocked sender matches “{filter}”"),
                        "Clear the filter to see every blocked sender.",
                    ))
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
