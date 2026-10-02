//! The Accounts page: the Gmail sign-in call to action and one card per account, each with its
//! icon and color, its nickname, and the sync status with the single action that status allows.

use super::*;

use crate::sync_status::SyncAction;
use crate::account_style;
use crate::app::icons;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonCustomVariant};
use gpui_kit::base::component_traits::Disableable as _;
use gpui_kit::component::menu::PopupMenuItem;

use gpui_kit::component::popover::Popover;

impl SettingsPanel {
    pub(super) fn accounts_page(&self, weak: &Weak) -> SettingPage {
        let keywords = ["accounts", "gmail", "add account", "remove account", "sign in", "email"];
        let mut cards = SettingGroup::new();
        if self.accounts.is_empty() {
            cards = cards.item(note("accounts-empty", "No accounts.").keywords(keywords));
        }
        for (index, account) in self.accounts.iter().cloned().enumerate() {
            let nickname_input = self.controls.get(&account.id).map(|c| c.nickname.clone());
            let weak = weak.clone();
            let label = account.email.clone();
            cards = cards.item(
                SettingItem::render(move |_, _, cx| {
                    account_card(index, &account, nickname_input.as_ref(), &weak, cx)
                })
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
                .child(div().flex_none().child(
                    Button::new("account-add-gmail")
                        .label("Add Gmail…")
                        .small()
                        .tooltip("Sign in to a Gmail account")
                        .disabled(!configured)
                        .on_click(move |_, _, cx| {
                            weak.update(cx, |_, cx| cx.emit(SettingsEvent::AddGmail)).ok();
                        }),
                ))
        })
        .keywords(keywords);
        SettingPage::new("Accounts")
            .resettable(false)
            .groups([SettingGroup::new().title("Add account").item(add), cards])
    }

    /// The user asked for what the account's sync status allows.
    pub(super) fn sync_action(&mut self, id: &str, action: SyncAction, cx: &mut Context<Self>) {
        cx.emit(SettingsEvent::SyncAction { id: id.to_owned(), action });
    }

    /// The user confirmed the removal of `id` in the alert dialog.
    pub(super) fn remove_account(&mut self, id: &str, cx: &mut Context<Self>) {
        cx.emit(SettingsEvent::RemoveAccount(id.to_owned()));
    }
}

/// One account as a card: a style tile (opens the icon and color picker), the nickname as an
/// inline-editable title with a pencil that focuses it, the address, an overflow menu, and a
/// footer with the sync status on the left and the one action that status allows on the right.
fn account_card(
    index: usize,
    account: &AccountRow,
    nickname: Option<&Entity<InputState>>,
    weak: &Weak,
    cx: &App,
) -> Stateful<Div> {
    let t = cx.theme();
    let color = theme::parse_color(&account.color).unwrap_or(t.primary);
    let tile = Button::new(("account-style", index))
        .custom(
            ButtonCustomVariant::new(cx)
                .color(color.opacity(0.16))
                .hover(color.opacity(0.28))
                .active(color.opacity(0.36))
                .foreground(color),
        )
        .icon(icons::account_icon(account.icon, &account.color, t, 26.))
        .w(px(52.))
        .h(px(52.))
        .rounded(px(12.))
        .tooltip("Change icon and color")
        .accessibility_label("Change icon and color");
    let picker = {
        let (weak, id, icon, selected_color) =
            (weak.clone(), account.id.clone(), account.icon, account.color.clone());
        Popover::new(("account-picker", index)).trigger(tile).content(move |_, _, cx| {
            let t = cx.theme();
            let icon_buttons = account_style::ICONS.iter().enumerate().map(|(i, (key, label))| {
                let (weak, id, key) = (weak.clone(), id.clone(), *key);
                let name = icons::account_icon_name(key).unwrap_or(IconName::Mail);
                Button::new(("account-icon", index * account_style::ICONS.len() + i))
                    .icon(name)
                    .small()
                    .tooltip(*label)
                    .accessibility_label(*label)
                    .selected(key == icon)
                    .on_click(move |_, _, cx| {
                        weak.update(cx, |this, cx| this.set_account_style(&id, Some(key), None, cx)).ok();
                    })
            });
            let swatches = account_style::COLORS.iter().enumerate().filter_map(|(i, hex)| {
                let (weak, id, hex) = (weak.clone(), id.clone(), *hex);
                let fill = theme::parse_color(hex)?;
                let chosen = hex.eq_ignore_ascii_case(&selected_color);
                Some(
                    Button::new(("account-color", index * account_style::COLORS.len() + i))
                        .custom(
                            ButtonCustomVariant::new(cx)
                                .color(fill)
                                .hover(fill)
                                .active(fill)
                                .foreground(t.background),
                        )
                        .when(chosen, |b| b.icon(IconName::Check))
                        .w(px(24.))
                        .h(px(24.))
                        .rounded(px(12.))
                        .border_2()
                        .border_color(if chosen { t.foreground } else { t.transparent })
                        .tooltip(format!("Color {hex}"))
                        .accessibility_label(format!("Color {hex}"))
                        .on_click(move |_, _, cx| {
                            weak.update(cx, |this, cx| {
                                this.set_account_style(&id, None, Some(hex.to_owned()), cx)
                            })
                            .ok();
                        }),
                )
            });
            div()
                .flex()
                .flex_col()
                .gap_2()
                .w(px(232.))
                .child(div().text_xs().text_color(t.muted_foreground).child("Icon"))
                .child(div().flex().flex_wrap().gap_1().children(icon_buttons))
                .child(div().text_xs().text_color(t.muted_foreground).child("Color"))
                .child(div().flex().flex_wrap().gap_1().children(swatches))
        })
    };
    let title = div().flex_1().min_w_0().flex().items_center().gap_1().child(
        div()
            .flex_1()
            .min_w_0()
            .text_base()
            .font_weight(FontWeight::SEMIBOLD)
            .children(nickname.map(|input| {
                Input::new(input).id(("account-nickname", index)).small().bordered(false).focus_bordered(true)
            })),
    );
    let header = div()
        .flex()
        .items_center()
        .gap_4()
        .p_4()
        .child(picker)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(title.children(rename_button(index, nickname, cx)))
                .child(
                    div().px_3().text_xs().text_color(t.muted_foreground).child(account.email.clone()),
                ),
        )
        .children(overflow_menu(index, account, weak, cx));
    let status_color = if account.sync_problem { t.danger } else { t.muted_foreground };
    let footer = div()
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .px_4()
        .py_2()
        .border_t_1()
        .border_color(t.border)
        .child(
            div()
                .id(("account-sync", index))
                .flex_1()
                .min_w_0()
                .truncate()
                .text_xs()
                .text_color(status_color)
                .child(account.sync.clone()),
        )
        .children(sync_action_button(index, account, weak));
    div()
        .id(("account-card", index))
        .flex()
        .flex_col()
        .rounded_lg()
        .border_1()
        .border_color(t.border)
        .bg(t.secondary)
        .child(header)
        .child(footer)
}

/// The quiet pencil that puts the cursor in the nickname input.
fn rename_button(index: usize, nickname: Option<&Entity<InputState>>, cx: &App) -> Option<Button> {
    let input = nickname?.clone();
    let faint = cx.theme().muted_foreground;
    Some(
        Button::new(("account-rename", index))
            .icon(IconName::Pencil)
            .small()
            .ghost()
            .tooltip("Rename")
            .accessibility_label("Rename")
            .text_color(faint)
            .on_click(move |_, window, cx| {
                input.read(cx).focus_handle(cx).focus(window, cx);
            }),
    )
}

/// The card's overflow menu: removal, which asks first.
fn overflow_menu(
    index: usize,
    account: &AccountRow,
    weak: &Weak,
    cx: &App,
) -> Option<AnyElement> {
    if !account.gmail {
        return None;
    }
    let (weak, id, email) = (weak.clone(), account.id.clone(), account.email.clone());
    let faint = cx.theme().muted_foreground;
    let trigger = Button::new(("account-overflow", index))
        .icon(IconName::Ellipsis)
        .small()
        .ghost()
        .tooltip("Account actions")
        .accessibility_label("Account actions")
        .text_color(faint);
    Some(
        div()
            .flex_none()
            .child(trigger.dropdown_menu(move |menu, _, _| {
                let weak = weak.clone();
                let id = id.clone();
                let title = format!("Remove \"{email}\"?");
                menu.item(PopupMenuItem::new("Remove…").on_click(move |_, window, cx| {
                    confirm_remove(&weak, &id, &title, window, cx)
                }))
            }))
            .into_any_element(),
    )
}

/// Ask before unlinking `id`, naming the account. Cancel changes nothing; Remove emits the removal.
fn confirm_remove(weak: &Weak, id: &str, title: &str, window: &mut Window, cx: &mut App) {
    let (weak, id, title) = (weak.clone(), id.to_owned(), title.to_owned());
    window.open_alert_dialog(cx, move |alert, _, _| {
        alert
            .title(title.clone())
            .description("Removes the account and its downloaded mail from this app. Gmail is not changed.")
            .confirm()
            .ok_text("Remove")
            .ok_variant(ButtonVariant::Danger)
            .cancel_text("Cancel")
            .on_ok({
                let (weak, id) = (weak.clone(), id.clone());
                move |_, _, cx| {
                    weak.update(cx, |this, cx| this.remove_account(&id, cx)).ok();
                    true
                }
            })
    })
}

/// The one action `account`'s sync status allows, or nothing while it is healthy.
fn sync_action_button(index: usize, account: &AccountRow, weak: &Weak) -> Option<Button> {
    let action = account.sync_action?;
    let weak = weak.clone();
    let (id, element) = (
        account.id.clone(),
        match action {
            SyncAction::SignInAgain => Button::new(("account-sign-in", index))
                .label("Sign in again…")
                .small()
                .tooltip("Sign in to this Gmail account again"),
            SyncAction::Retry => Button::new(("account-retry", index))
                .label("Retry")
                .small()
                .tooltip("Run this account's sync again"),
        },
    );
    Some(element.on_click(move |_, _, cx| {
        weak.update(cx, |this, cx| this.sync_action(&id, action, cx)).ok();
    }))
}
