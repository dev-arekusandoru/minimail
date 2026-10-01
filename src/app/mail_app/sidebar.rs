//! The accounts/folders sidebar: every location the list can show, in one column.
//!
//! Built on the kit [`Sidebar`]: an *All Inboxes* row, then one foldable submenu per account
//! carrying its Inbox / Snoozed / Sent / Archive / Trash and its folder tree as nested items.
//! Rows dispatch [`ShowLocation`]; Inbox and Snoozed rows (and All Inboxes) carry a count when
//! non-zero.
//!
//! The kit owns folding (account and folder submenus open with their caret). The app owns the
//! pane's width and visibility through the resizable panel and `cmd-b`.
use gpui_kit::component::ActiveTheme as _;

use super::*;
use crate::theme::ThemeColor;
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    sidebar::{Sidebar, SidebarCollapsible, SidebarMenuItem},
    tag::Tag,
    Icon, Sizable as _,
};

/// Titlebar and list-header label for the current location, e.g. `"All Inboxes"`,
/// `"Work · Archive"` or `"Projects/Northwind"`.
pub(super) fn view_label(view: &View, mailbox: &Mailbox) -> String {
    match &view.location {
        Location::AllInboxes => "All Inboxes".to_owned(),
        Location::Folder(id) => folder_path(mailbox, *id),
        loc => format!(
            "{} · {}",
            account_name(mailbox, &location_account(loc)),
            location_name(loc)
        ),
    }
}

/// Name of a location inside one account, e.g. `"Inbox"`.
fn location_name(loc: &Location) -> &'static str {
    match loc {
        Location::AllInboxes => "All Inboxes",
        Location::Inbox(_) => "Inbox",
        Location::Snoozed(_) => "Snoozed",
        Location::Sent(_) => "Sent",
        Location::Archive(_) => "Archive",
        Location::Trash(_) => "Trash",
        Location::Folder(_) => "Folder",
    }
}

/// Account a location belongs to; empty when the location is not account-specific.
fn location_account(loc: &Location) -> AccountId {
    match loc {
        Location::Inbox(a)
        | Location::Snoozed(a)
        | Location::Sent(a)
        | Location::Archive(a)
        | Location::Trash(a) => a.clone(),
        Location::AllInboxes | Location::Folder(_) => AccountId::new(),
    }
}

fn account_name(mailbox: &Mailbox, id: &str) -> String {
    mailbox
        .account(id)
        .map_or_else(|| id.to_owned(), |a| a.name.clone())
}

/// A folder's name including its parents, e.g. `"Projects/Northwind"`.
fn folder_path(mailbox: &Mailbox, id: FolderId) -> String {
    let mut names = Vec::new();
    let mut cursor = Some(id);
    while let Some(next) = cursor {
        let Some(folder) = mailbox.folder(next) else {
            break;
        };
        names.push(folder.name.clone());
        cursor = folder.parent;
    }
    names.reverse();
    names.join("/")
}

/// Whether a location shows Inbox mail — the only locations with a chip row.
pub(super) fn is_inbox_location(loc: &Location) -> bool {
    matches!(loc, Location::AllInboxes | Location::Inbox(_))
}

/// Icon and tint of a location.
fn location_icon(loc: &Location, t: &ThemeColor, open: bool) -> (IconName, Hsla) {
    match loc {
        Location::AllInboxes => (IconName::Mails, t.primary),
        Location::Inbox(_) => (IconName::Inbox, theme::inbox(t)),
        Location::Snoozed(_) => (IconName::AlarmClock, theme::snoozed(t)),
        Location::Sent(_) => (IconName::Send, t.info),
        Location::Archive(_) => (IconName::Archive, theme::archived(t)),
        Location::Trash(_) => (IconName::Trash, theme::deleted(t)),
        Location::Folder(_) if open => (IconName::FolderOpen, theme::filed(t)),
        Location::Folder(_) => (IconName::Folder, theme::filed(t)),
    }
}

/// A pill badge with a message count, used as a menu item's suffix.
fn count_badge(t: &ThemeColor, n: usize, highlighted: bool) -> Tag {
    let (bg, fg) = if highlighted {
        (t.primary, t.primary_foreground)
    } else {
        (t.secondary, t.muted_foreground)
    };
    Tag::custom(bg, fg, bg).small().rounded_full().flex_none().child(n.to_string())
}

impl MailApp {
    /// Account the `g`-prefix jumps land in: the current location's, else the first.
    pub(super) fn nav_account(&self) -> Option<AccountId> {
        match &self.triage.view.location {
            Location::Folder(id) => self.mailbox.folder(*id).map(|f| f.account.clone()),
            Location::AllInboxes => self.mailbox.accounts().first().map(|a| a.id.clone()),
            loc => Some(location_account(loc)).filter(|a| !a.is_empty()),
        }
    }

    /// The sidebar: All Inboxes, then per account a submenu of its locations and folder tree.
    pub(super) fn render_sidebar(&self, cx: &Context<Self>) -> AnyElement {
        let active = &self.triage.view.location;
        let mut items: Vec<SidebarMenuItem> =
            vec![self.location_item(Location::AllInboxes, active, cx)];
        for account in self.mailbox.accounts() {
            let mut children: Vec<SidebarMenuItem> = [
                Location::Inbox(account.id.clone()),
                Location::Snoozed(account.id.clone()),
                Location::Sent(account.id.clone()),
                Location::Archive(account.id.clone()),
                Location::Trash(account.id.clone()),
            ]
            .into_iter()
            .map(|loc| self.location_item(loc, active, cx))
            .collect();
            for folder in self.mailbox.folders(&account.id) {
                if folder.parent.is_none() {
                    children.push(self.folder_item(folder, &account.id, active, cx));
                }
            }
            items.push(
                SidebarMenuItem::new(account.name.clone())
                    .icon(Icon::new(IconName::User).text_color(
                        theme::parse_color(&account.color).unwrap_or(cx.theme().primary),
                    ))
                    .default_open(true)
                    .children(children),
            );
        }
        let header = div()
            .flex()
            .w_full()
            .items_center()
            .justify_between()
            .child(div().text_xs().text_color(cx.theme().muted_foreground).child("Mailboxes"))
            .child(
                icon_button("sidebar-add-account", IconName::Plus, "Add or manage accounts", "", cx)
                    .on_click(run(ManageAccounts)),
            );
        Sidebar::new("sidebar")
            .w_full()
            .collapsible(SidebarCollapsible::None)
            .header(header)
            .children(items)
            .into_any_element()
    }

    /// One location row: its icon in the location's colour, active when it is the view, a count
    /// suffix for All Inboxes / Inbox / Snoozed when non-zero. Clicking shows the location.
    fn location_item(
        &self,
        loc: Location,
        active: &Location,
        cx: &Context<Self>,
    ) -> SidebarMenuItem {
        let (icon, color) = location_icon(&loc, cx.theme(), false);
        let is_active = active == &loc;
        let counted =
            matches!(loc, Location::AllInboxes | Location::Inbox(_) | Location::Snoozed(_));
        let count = self.mailbox.count_at(&loc);
        let mut item = SidebarMenuItem::new(location_name(&loc).to_owned())
            .icon(Icon::new(icon).text_color(color))
            .active(is_active)
            .on_click(run(ShowLocation { location: loc }));
        if counted && count > 0 {
            item = item.suffix(move |_, cx| count_badge(cx.theme(), count, is_active));
        }
        item
    }

    /// One folder row: an item whose children are its subfolders. The kit submenu owns folding.
    fn folder_item(
        &self,
        folder: &Folder,
        account: &AccountId,
        active: &Location,
        cx: &Context<Self>,
    ) -> SidebarMenuItem {
        let loc = Location::Folder(folder.id);
        let subfolders: Vec<&Folder> = self
            .mailbox
            .folders(account)
            .into_iter()
            .filter(|f| f.parent == Some(folder.id))
            .collect();
        let (icon, color) = location_icon(&loc, cx.theme(), !subfolders.is_empty());
        let mut item = SidebarMenuItem::new(folder.name.clone())
            .icon(Icon::new(icon).text_color(color))
            .active(active == &loc)
            .on_click(run(ShowLocation { location: loc }));
        if !subfolders.is_empty() {
            item = item.default_open(true).children(
                subfolders
                    .into_iter()
                    .map(|f| self.folder_item(f, account, active, cx))
                    .collect::<Vec<_>>(),
            );
        }
        item
    }
}
