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
        let never = self.spam_group(weak, "never", "Never spam", self.spam_senders.never());
        let always = self.spam_group(weak, "always", "Always spam", self.spam_senders.always());
        SettingPage::new("Senders").resettable(false).groups([blocked, never, always])
    }

    /// One list of addresses the user marked never/always spam, each with a Remove button.
    fn spam_group(&self, weak: &Weak, id: &'static str, title: &'static str, emails: Vec<String>) -> SettingGroup {
        let mut group = SettingGroup::new().title(title);
        if emails.is_empty() {
            let detail = if id == "never" {
                "Answer yes when asked after marking mail not spam, and Jev stops asking whether that address sends spam."
            } else {
                "Answer yes when asked after deleting spam, and that address's new mail is marked spam without asking Jev."
            };
            return group.item(
                SettingItem::render(move |_, _, _| {
                    div().id(format!("{id}-spam-empty")).child(crate::app::ui::empty(
                        IconName::ShieldAlert,
                        format!("No {id}-spam senders"),
                        detail,
                    ))
                })
                .keywords([title, "spam", "senders"]),
            );
        }
        for email in emails {
            let weak = weak.clone();
            let address = email.clone();
            group = group.item(
                SettingItem::render(move |_, _, _| {
                    let (weak, email) = (weak.clone(), email.clone());
                    div()
                        .id(format!("{id}-spam-row-{email}"))
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .text_sm()
                        .child(div().min_w_0().child(email.clone()))
                        .child(
                            Button::new(format!("{id}-spam-remove-{email}"))
                                .label("Remove")
                                .small()
                                .on_click(move |_, _, cx| {
                                    weak.update(cx, |this, cx| this.clear_spam_sender(&email, cx)).ok();
                                }),
                        )
                })
                .keywords([title, "spam", address.as_str()]),
            );
        }
        group
    }
}
