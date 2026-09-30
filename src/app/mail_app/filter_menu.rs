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

fn tag_id(tag: TagFilter) -> &'static str {
    match tag {
        TagFilter::NeedsReply => "filter-tag-needs-reply",
        TagFilter::AwaitingReply => "filter-tag-awaiting-reply",
        TagFilter::FollowUp => "filter-tag-follow-up",
        TagFilter::Reminder => "filter-tag-reminder",
        TagFilter::NewSender => "filter-tag-new-sender",
        TagFilter::PossibleSpam => "filter-tag-possible-spam",
        TagFilter::Urgent => "filter-tag-urgent",
    }
}

/// Marker in front of an active filter row.
fn check(on: bool) -> &'static str {
    if on { "✓ " } else { "" }
}

impl MailApp {
    /// Filter ▾ menu rows.
    pub(super) fn filter_items(&self) -> Vec<MenuItem> {
        let filter = &self.triage.view.filter;
        let mut items: Vec<MenuItem> = TAGS
            .into_iter()
            .map(|tag| {
                let on = filter.tags.contains(&tag);
                MenuItem::action_fn(
                    tag_id(tag),
                    format!("{}{}", check(on), tag_label(tag)),
                    "",
                    move || Box::new(ToggleTagFilter { tag }),
                )
            })
            .collect();
        items.push(MenuItem::separator());
        let mut kinds = vec![kind_item("filter-kind-any", "Any kind", None, filter.kind)];
        for kind in Kind::ALL {
            kinds.push(kind_item(
                format!("filter-kind-{}", kind.label()),
                kind.label(),
                Some(kind),
                filter.kind,
            ));
        }
        items.push(MenuItem::submenu(
            "filter-kind",
            match filter.kind {
                Some(kind) => format!("Kind: {}", kind.label()),
                None => "Kind".to_owned(),
            },
            kinds,
        ));
        if matches!(self.triage.view.location, Location::AllInboxes) {
            let mut accounts = vec![account_item("filter-account-any", "Any account", None, filter)];
            for account in self.mailbox.accounts() {
                accounts.push(account_item(
                    format!("filter-account-{}", account.id),
                    account.name.clone(),
                    Some(account.id.clone()),
                    filter,
                ));
            }
            items.push(MenuItem::submenu(
                "filter-account",
                match filter.account.as_deref() {
                    Some(id) => format!("Account: {}", mailbox_name(&self.mailbox, id)),
                    None => "Account".to_owned(),
                },
                accounts,
            ));
        }
        items.push(MenuItem::separator());
        items.push(MenuItem::action("filter-clear", "Clear filters", "", || {
            Box::new(ClearFilters)
        }));
        items
    }
}

fn mailbox_name(mailbox: &Mailbox, id: &str) -> String {
    mailbox
        .account(id)
        .map_or_else(|| id.to_owned(), |a| a.name.clone())
}

fn kind_item(
    id: impl Into<SharedString>,
    label: &str,
    kind: Option<Kind>,
    current: Option<Kind>,
) -> MenuItem {
    let on = current == kind;
    MenuItem::action_fn(id, format!("{}{}", check(on), label), "", move || {
        Box::new(SetFilterKind { kind })
    })
}

fn account_item(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    account: Option<AccountId>,
    filter: &Filter,
) -> MenuItem {
    let on = filter.account == account;
    let label = label.into();
    MenuItem::action_fn(id, format!("{}{}", check(on), label), "", move || {
        Box::new(SetFilterAccount {
            account: account.clone(),
        })
    })
}
