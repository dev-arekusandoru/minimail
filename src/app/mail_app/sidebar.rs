//! The accounts/folders sidebar: every location the list can show, in one column.
//!
//! *All Inboxes*, then per account its Inbox / Snoozed / Sent / Archive / Trash and the
//! account's folders as a collapsible tree. Rows dispatch [`ShowLocation`]; only Inbox
//! rows carry a count (the unfiltered one), and the current location is highlighted.

use super::*;
use crate::theme::Theme;

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

/// Element id of a sidebar row, so tests can click it: `"nav-archive-work"`.
pub(super) fn location_id(loc: &Location) -> String {
    match loc {
        Location::AllInboxes => "nav-all-inboxes".to_owned(),
        Location::Inbox(a) => format!("nav-inbox-{a}"),
        Location::Snoozed(a) => format!("nav-snoozed-{a}"),
        Location::Sent(a) => format!("nav-sent-{a}"),
        Location::Archive(a) => format!("nav-archive-{a}"),
        Location::Trash(a) => format!("nav-trash-{a}"),
        Location::Folder(f) => format!("nav-folder-{f}"),
    }
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

    /// The sidebar: All Inboxes, then each account's locations and its folder tree.
    pub(super) fn render_sidebar(&self, cx: &Context<Self>) -> AnyElement {
        let t = theme::active(cx);
        let mailbox = &self.mailbox;
        let active = &self.triage.view.location;
        let mut rows: Vec<AnyElement> =
            vec![nav_row(&t, &Location::AllInboxes, "All Inboxes", active, Some(mailbox.count_at(&Location::AllInboxes)), 0.)];
        for account in mailbox.accounts() {
            let color = crate::theme::parse_color(&account.color).unwrap_or(t.accent);
            rows.push(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .pt_3()
                    .pb_1()
                    .child(div().w(px(7.)).h(px(7.)).flex_none().rounded_full().bg(color))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(t.text_muted)
                            .child(account.name.clone()),
                    )
                    .into_any_element(),
            );
            let account_locations = [
                (Location::Inbox(account.id.clone()), true),
                (Location::Snoozed(account.id.clone()), false),
                (Location::Sent(account.id.clone()), false),
                (Location::Archive(account.id.clone()), false),
                (Location::Trash(account.id.clone()), false),
            ];
            for (loc, counted) in account_locations {
                let count = counted.then(|| mailbox.count_at(&loc));
                rows.push(nav_row(&t, &loc, location_name(&loc), active, count, 0.));
            }
            let folders = mailbox.folders(&account.id);
            fold_rows(&t, &folders, None, active, &self.collapsed_folders, 0, &mut rows);
        }
        div()
            .id("sidebar")
            .test_support()
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .children(rows)
            .into_any_element()
    }
}

/// One sidebar row; `count` is Some on Inbox rows only.
fn nav_row(
    t: &Theme,
    loc: &Location,
    label: &str,
    active: &Location,
    count: Option<usize>,
    indent: f32,
) -> AnyElement {
    let is_active = active == loc;
    let hover = t.hover;
    div()
        .id(SharedString::from(location_id(loc)))
        .test_support()
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .h(px(24.))
        .pl(px(8. + indent))
        .pr_2()
        .rounded_sm()
        .text_size(px(12.))
        .text_color(if is_active { t.accent } else { t.text })
        .when(is_active, |d| d.bg(t.selection))
        .when(!is_active, |d| d.hover(move |d| d.bg(hover)))
        .cursor_pointer()
        .on_click(run(ShowLocation {
            location: loc.clone(),
        }))
        .child(div().flex_1().min_w_0().truncate().child(SharedString::from(label.to_owned())))
        .when_some(count, |d, n| {
            d.child(
                div()
                    .flex_none()
                    .text_size(px(11.))
                    .text_color(if is_active { t.accent } else { t.text_muted })
                    .child(n.to_string()),
            )
        })
        .into_any_element()
}

/// Push a folder and, unless it is folded away, its children (indented one level deeper).
fn fold_rows(
    t: &Theme,
    folders: &[&Folder],
    parent: Option<FolderId>,
    active: &Location,
    collapsed: &HashSet<FolderId>,
    depth: usize,
    rows: &mut Vec<AnyElement>,
) {
    for folder in folders.iter().filter(|f| f.parent == parent) {
        let has_children = folders.iter().any(|f| f.parent == Some(folder.id));
        let folded = collapsed.contains(&folder.id);
        rows.push(folder_row(t, folder, active, depth, has_children, folded));
        if has_children && !folded {
            fold_rows(t, folders, Some(folder.id), active, collapsed, depth + 1, rows);
        }
    }
}

/// A folder row: a fold caret (when it has children) and the folder name.
fn folder_row(
    t: &Theme,
    folder: &Folder,
    active: &Location,
    depth: usize,
    has_children: bool,
    folded: bool,
) -> AnyElement {
    let loc = Location::Folder(folder.id);
    let is_active = active == &loc;
    let hover = t.hover;
    let caret = if !has_children {
        " "
    } else if folded {
        "▸"
    } else {
        "▾"
    };
    let id = folder.id;
    div()
        .id(SharedString::from(location_id(&loc)))
        .test_support()
        .flex()
        .items_center()
        .gap_1()
        .h(px(24.))
        .pl(px(6. + depth as f32 * 12.))
        .pr_2()
        .rounded_sm()
        .text_size(px(12.))
        .text_color(if is_active { t.accent } else { t.text })
        .when(is_active, |d| d.bg(t.selection))
        .when(!is_active, |d| d.hover(move |d| d.bg(hover)))
        .cursor_pointer()
        .on_click(run(ShowLocation { location: loc }))
        .child(
            div()
                .id(("folder-caret", id as usize))
                .test_support()
                .w(px(12.))
                .flex_none()
                .text_color(t.text_muted)
                .when(has_children, |d| {
                    d.cursor_pointer().on_click(move |_, window, cx| {
                        // Folding must not also open the folder the caret belongs to.
                        cx.stop_propagation();
                        window.dispatch_action(Box::new(ToggleFolder { folder: id }), cx);
                    })
                })
                .child(caret),
        )
        .child(div().flex_1().min_w_0().truncate().child(folder.name.clone()))
        .into_any_element()
}
