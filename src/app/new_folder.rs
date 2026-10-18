//! The two steps of "New folder": pick an account (skipped with one), then name the folder.
//!
//! Like the other dialog views these hold no business logic: the choice leaves through an
//! event and the owner creates the folder. Hosted in a kit dialog, which owns `escape`.
use gpui_kit::component::ActiveTheme as _;

use crate::app::icons;
use crate::model::{Account, AccountId};
use gpui_kit::{
    base::IndexPath,
    component::{
        command::{Command, CommandItem, CommandState},
        input::{Input, InputEvent, InputState},
    },
    *,
};

pub enum NewFolderEvent {
    Account(AccountId),
    Name(String),
}

impl EventEmitter<NewFolderEvent> for AccountPicker {}
impl EventEmitter<NewFolderEvent> for NamePrompt {}

/// Up/down/enter (or a click) over the accounts, in sidebar order.
pub struct AccountPicker {
    state: Entity<CommandState>,
    accounts: Vec<Account>,
}

impl AccountPicker {
    pub fn new(accounts: Vec<Account>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self { state: cx.new(|cx| CommandState::new(window, cx)), accounts }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.state.clone().update(cx, |state, cx| state.focus(window, cx));
    }
}

impl Render for AccountPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let weak = cx.weak_entity();
        let t = cx.theme();
        let items = self.accounts.iter().map(|a| {
            CommandItem::new()
                .label(crate::account_style::display_name(a).to_owned())
                .icon(icons::account_icon(crate::account_style::icon_key(a), &a.color, t, 16.))
        });
        let ids: Vec<AccountId> = self.accounts.iter().map(|a| a.id.clone()).collect();
        Command::new(&self.state)
            .bordered(false)
            .filterable(false)
            .max_h(px(360.))
            .placeholder("Account")
            .header(|_, _, cx| {
                div()
                    .px_3()
                    .pt_3()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(cx.theme().foreground)
                    .child("New folder in:")
            })
            .items(items)
            .on_confirm(move |ix: IndexPath, _, cx| {
                let Some(id) = ids.get(ix.row) else { return };
                weak.update(cx, |_, cx| cx.emit(NewFolderEvent::Account(id.clone()))).ok();
            })
    }
}

/// A single text field; enter submits the typed name.
pub struct NamePrompt {
    input: Entity<InputState>,
    account: SharedString,
    _sub: Subscription,
}

impl NamePrompt {
    pub fn new(account: impl Into<SharedString>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Folder name"));
        let _sub = cx.subscribe(&input, |_, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                cx.emit(NewFolderEvent::Name(input.read(cx).value().to_string()));
            }
        });
        Self { input, account: account.into(), _sub }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.input.focus_handle(cx), cx);
    }
}

impl Render for NamePrompt {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(cx.theme().foreground)
                    .child(format!("New folder in {}", self.account)),
            )
            .child(Input::new(&self.input))
    }
}
