//! Contents of the Filter ▾ menu in the list header: tags, Kind, account and Clear.
//!
//! Rows carry the choice they toggle, so one menu build serves every location.

use super::*;
use crate::app::menu::MenuItem;

/// Tag filters, in menu order.
const TAGS: [TagFilter; 7] = [
    TagFilter::NeedsReply,
    TagFilter::AwaitingReply,
    TagFilter::FollowUp,
    TagFilter::Reminder,
    TagFilter::NewSender,
    TagFilter::PossibleSpam,
    TagFilter::Urgent,
];

fn tag_label(tag: TagFilter) -> &'static str {
    match tag {
        TagFilter::NeedsReply => "Needs reply",
        TagFilter::AwaitingReply => "Awaiting reply",
        TagFilter::FollowUp => "Follow up",
        TagFilter::Reminder => "Reminder",
        TagFilter::NewSender => "New sender",
        TagFilter::PossibleSpam => "Possible spam",
        TagFilter::Urgent => "Urgent",
    }
}

impl MailApp {
    /// Filter ▾ menu rows; the active choices carry a check mark.
    pub(super) fn filter_items(&self) -> Vec<MenuItem> {
        let filter = &self.triage.view.filter;
        let mut items: Vec<MenuItem> = TAGS
            .into_iter()
            .map(|tag| {
                MenuItem::action_fn(tag_label(tag), move || Box::new(ToggleTagFilter { tag }))
                    .checked(filter.tags.contains(&tag))
            })
            .collect();
        items.push(MenuItem::separator());
        let mut kinds = vec![kind_item("Any kind", None, filter.kind)];
        for kind in Kind::ALL {
            kinds.push(kind_item(kind.label(), Some(kind), filter.kind));
        }
        items.push(MenuItem::submenu(
            match filter.kind {
                Some(kind) => format!("Kind: {}", kind.label()),
                None => "Kind".to_owned(),
            },
            kinds,
        ));
        if matches!(self.triage.view.location, Location::AllInboxes) {
            let mut accounts = vec![account_item("Any account", None, filter)];
            for account in self.mailbox.accounts() {
                accounts.push(account_item(account.name.clone(), Some(account.id.clone()), filter));
            }
            items.push(MenuItem::submenu(
                match filter.account.as_deref() {
                    Some(id) => format!("Account: {}", mailbox_name(&self.mailbox, id)),
                    None => "Account".to_owned(),
                },
                accounts,
            ));
        }
        items.push(MenuItem::separator());
        items.push(MenuItem::action("Clear filters", || Box::new(ClearFilters)));
        items
    }
}

fn mailbox_name(mailbox: &Mailbox, id: &str) -> String {
    mailbox
        .account(id)
        .map_or_else(|| id.to_owned(), |a| a.name.clone())
}

fn kind_item(label: &str, kind: Option<Kind>, current: Option<Kind>) -> MenuItem {
    MenuItem::action_fn(label.to_owned(), move || Box::new(SetFilterKind { kind }))
        .checked(current == kind)
}

fn account_item(
    label: impl Into<SharedString>,
    account: Option<AccountId>,
    filter: &Filter,
) -> MenuItem {
    let on = filter.account == account;
    MenuItem::action_fn(label, move || {
        Box::new(SetFilterAccount {
            account: account.clone(),
        })
    })
    .checked(on)
}
