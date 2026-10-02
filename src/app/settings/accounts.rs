use super::*;

impl SettingsPanel {
    pub(super) fn accounts_page(&self, weak: &Weak) -> SettingPage {
        let keywords = ["accounts", "gmail", "add account", "remove account", "sign in", "email"];
        let mut linked = SettingGroup::new();
        if self.accounts.is_empty() {
            linked = linked.item(note("accounts-empty", "No accounts.").keywords(keywords));
        }
        for (index, account) in self.accounts.iter().cloned().enumerate() {
            let nickname_input = self.controls.get(&account.id).map(|c| c.nickname.clone());
            let weak = weak.clone();
            let label = account.email.clone();
            let confirming = self.confirm_remove.as_deref() == Some(account.id.as_str());
            linked = linked.item(
                SettingItem::render(move |_, _, cx| account_card(index, &account, nickname_input.as_ref(), confirming, &weak, cx))
                    .keywords(keywords.into_iter().chain([label.as_str()])),
            );
        }
        let configured = self.gmail_configured;
        let weak = weak.clone();
        let add = SettingItem::render(move |_, _, cx| {
            let weak = weak.clone();
            let hint = if configured {
                "Sign in with your browser; the token is kept in the system keychain."
            } else {
                GMAIL_ENV_HINT
            };
            div()
                .id("account-add")
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .text_sm()
                .child(
                    div().flex().flex_col().flex_1().min_w_0().child("Add Gmail account").child(
                        div().text_xs().text_color(cx.theme().muted_foreground).child(hint),
                    ),
                )
                .child(
                    div().flex_none().child(
                        button("account-add-gmail", "Add Gmail…", "Sign in to a Gmail account", "", cx)
                            .on_click(move |_, _, cx| {
                                weak.update(cx, |_, cx| cx.emit(SettingsEvent::AddGmail)).ok();
                            }),
                    ),
                )
        })
        .keywords(keywords);
        SettingPage::new("Accounts")
            .resettable(false)
            .groups([linked, SettingGroup::new().title("Add account").item(add)])
    }
}
