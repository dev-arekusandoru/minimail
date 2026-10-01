//! The accounts/folders sidebar: every location the list can show, in one column.
//!
//! *All Inboxes*, then one foldable section per account: a small header label (name; chevron
//! on hover, Inbox count while folded), its Inbox / Snoozed / Sent / Archive /
//! Trash, and the top-level folders, all as rail-less roots. Rows dispatch
//! [`ShowLocation`]; Inbox and Snoozed rows (and All Inboxes) carry a count when non-zero.
//!
//! Every row has an always-coloured icon. The only trees are subfolders: rails run down from
//! a parent's icon and elbow into each child, faint grey except along the path to the active
//! folder (accent). Hover/active highlight is a rounded pill starting at the row's icon; the
//! active one has a thin bar in the account's colour. A folder with children carries its fold
//! caret at the right end of its pill.
use gpui_kit::component::ActiveTheme as _;

use super::*;
use crate::theme::ThemeColor;
use gpui_kit::assets::IconName;
use gpui_kit::component::{scroll::ScrollableElement as _, tag::Tag, Icon, Sizable as _};

/// Width of one rail column. Icons sit in a slot of the same width, so a child's rail runs
/// straight down from its parent's icon.
const RAIL: f32 = 16.;
/// Height of every sidebar row.
const ROW_H: f32 = 26.;
/// Size of row icons.
const ICON: f32 = 14.;
/// Left padding inside a pill, before the icon slot.
const PAD: f32 = 6.;
/// Width of the active row's marker bar.
const BAR: f32 = 3.;
/// Opacity of the default (grey) rails.
const RAIL_FAINT: f32 = 0.7;

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

    /// The sidebar: All Inboxes, then per account a foldable section (its locations and the
    /// top-level folders, whose subfolders form the only trees).
    pub(super) fn render_sidebar(&self, cx: &Context<Self>) -> AnyElement {
        let t = cx.theme();
        let mailbox = &self.mailbox;
        let active = &self.triage.view.location;
        let chain = active_chain(mailbox, active);
        let all = Location::AllInboxes;
        let mut rows: Vec<AnyElement> =
            vec![nav_row(t, &all, "All Inboxes", active, Some(mailbox.count_at(&all)), t.primary)];
        for account in mailbox.accounts() {
            let color = crate::theme::parse_color(&account.color).unwrap_or(t.primary);
            let folded = self.collapsed_accounts.contains(&account.id);
            let inbox = Location::Inbox(account.id.clone());
            rows.push(account_header(
                t,
                &account.id,
                &account.name,
                folded,
                mailbox.count_at(&inbox),
            ));
            if folded {
                continue;
            }
            let account_locations = [
                (inbox, true),
                (Location::Snoozed(account.id.clone()), true),
                (Location::Sent(account.id.clone()), false),
                (Location::Archive(account.id.clone()), false),
                (Location::Trash(account.id.clone()), false),
            ];
            for (loc, counted) in account_locations {
                let count = counted.then(|| mailbox.count_at(&loc));
                rows.push(nav_row(t, &loc, location_name(&loc), active, count, color));
            }
            let folders = mailbox.folders(&account.id);
            if folders.iter().any(|f| f.parent.is_none()) {
                let mut through = Vec::new();
                let ctx = FolderCtx {
                    t,
                    folders: &folders,
                    active,
                    chain: &chain,
                    collapsed: &self.collapsed_folders,
                    color,
                };
                fold_rows(&ctx, None, &mut through, &mut rows);
            }
        }
        div()
            .id("sidebar")
            .test_support()
            .flex()
            .flex_col()
            .p_2()
            .flex_1()
            .min_h_0()
            .overflow_y_scrollbar()
            .children(rows)
            .into_any_element()
    }
}

/// Folder ids from the top-level folder down to the active folder; empty when the active
/// location is not a folder.
fn active_chain(mailbox: &Mailbox, active: &Location) -> Vec<FolderId> {
    let Location::Folder(start) = active else {
        return Vec::new();
    };
    let mut id = *start;
    let mut chain = vec![id];
    while let Some(parent) = mailbox.folder(id).and_then(|f| f.parent) {
        if chain.contains(&parent) {
            break;
        }
        chain.push(parent);
        id = parent;
    }
    chain.reverse();
    chain
}

/// How a folder row relates to the path down to the active folder, among its siblings.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PathRel {
    /// Not on the path, or a later sibling of the path's folder: the rail stays grey.
    Off,
    /// An earlier sibling of the path's folder: the parent's accent rail runs past it.
    Before,
    /// The path's own folder: the accent rail ends in its elbow.
    On,
}

/// Where a subfolder row hangs in its parent's tree.
#[derive(Clone, Copy)]
struct Branch<'a> {
    /// Per ancestor level below the top-level folder: `(on, accent)` — whether that ancestor
    /// has later siblings (its rail passes this row) and whether the pass is on the active path.
    through: &'a [(bool, bool)],
    /// Whether the row is its parent's last child, i.e. its elbow ends the parent's rail.
    last: bool,
    rel: PathRel,
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

/// Colour of a rail: faint grey, or the accent on the path to the active folder.
fn rail_color(t: &ThemeColor, hot: bool) -> Hsla {
    if hot { t.primary } else { t.border.opacity(RAIL_FAINT) }
}

/// A 1px vertical rail centred in a [`RAIL`]-wide column, inset `top`/`bottom` from the row edges.
fn vline(color: Hsla, top: f32, bottom: f32) -> Div {
    div()
        .absolute()
        .left(px(RAIL / 2. - 0.5))
        .w(px(1.))
        .top(px(top))
        .bottom(px(bottom))
        .bg(color)
}

/// A [`RAIL`]-wide, full-height column that rails and icons are positioned in.
fn column() -> Div {
    div().relative().flex_none().w(px(RAIL)).h_full()
}

/// Below-the-icon stem that leads into the first child's rail.
fn stem(color: Hsla) -> Div {
    vline(color, (ROW_H + ICON) / 2. + 1., 0.)
}

/// The rail columns left of a subfolder's pill: pass-through rails of its ancestors, then its
/// own elbow. The wrapper is offset by [`PAD`] and the elbow column is [`PAD`] narrower, so
/// each rail sits under its parent's icon centre and the pill starts where the stub ends.
fn rails(t: &ThemeColor, branch: Branch) -> Div {
    let mid = ROW_H / 2.;
    let mut cols: Vec<AnyElement> = branch
        .through
        .iter()
        .map(|&(on, hot)| {
            column()
                .when(on, |d| d.child(vline(rail_color(t, hot), 0., 0.)))
                .into_any_element()
        })
        .collect();
    let elbow = column()
        .w(px(RAIL - PAD))
        .child(vline(rail_color(t, branch.rel != PathRel::Off), 0., mid))
        .when(!branch.last, |d| {
            d.child(vline(rail_color(t, branch.rel == PathRel::Before), mid, 0.))
        })
        .child(
            div()
                .absolute()
                .left(px(RAIL / 2. - 0.5))
                .right_0()
                .top(px(mid - 0.5))
                .h(px(1.))
                .bg(rail_color(t, branch.rel == PathRel::On)),
        );
    cols.push(elbow.into_any_element());
    div().flex().flex_none().h_full().ml(px(PAD)).children(cols)
}

/// The fold toggle of a folder with children, at the pill's right end; folding never opens
/// the folder.
fn caret_button(t: &ThemeColor, id: FolderId, folded: bool) -> impl IntoElement {
    let icon = if folded { IconName::ChevronRight } else { IconName::ChevronDown };
    div()
        .id(("folder-caret", id as usize))
        .test_support()
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .size(px(18.))
        .ml_1()
        .cursor_pointer()
        .on_click(move |_, window, cx| {
            // Folding must not also open the folder the caret belongs to.
            cx.stop_propagation();
            window.dispatch_action(Box::new(ToggleFolder { folder: id }), cx);
        })
        .child(Icon::new(icon).with_size(px(12.)).text_color(t.muted_foreground))
}

/// The icon slot of a row; `stem_down` continues the rail (in that colour) to the row's children.
fn icon_slot(icon: IconName, color: Hsla, stem_down: Option<Hsla>) -> Div {
    column()
        .flex()
        .items_center()
        .justify_center()
        .when_some(stem_down, |d, c| d.child(stem(c)))
        .child(Icon::new(icon).with_size(px(ICON)).text_color(color))
}

/// A pill badge with a message count.
fn count_badge(t: &ThemeColor, n: usize, highlighted: bool) -> Tag {
    let (bg, fg) = if highlighted {
        (t.primary, t.primary_foreground)
    } else {
        (t.secondary, t.muted_foreground)
    };
    Tag::custom(bg, fg, bg).small().rounded_full().flex_none().ml_2().child(n.to_string())
}

/// An account's section label: the name, a fold chevron that appears on hover, and — while
/// folded — the account's Inbox count. Clicking anywhere on it folds the account.
fn account_header(
    t: &ThemeColor,
    id: &str,
    name: &str,
    folded: bool,
    inbox_count: usize,
) -> AnyElement {
    let group = SharedString::from(format!("account-head-{id}"));
    let chevron = if folded { IconName::ChevronRight } else { IconName::ChevronDown };
    let account = id.to_owned();
    let hover = t.list_hover;
    div()
        .id(SharedString::from(format!("account-{id}")))
        .test_support()
        .group(group.clone())
        .flex()
        .items_center()
        .flex_none()
        .h(px(ROW_H))
        .mt_2()
        .pl(px(PAD))
        .pr_2()
        .rounded_md()
        .cursor_pointer()
        .hover(move |d| d.bg(hover))
        .on_click(run(ToggleAccount { account }))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(10.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.muted_foreground)
                .child(SharedString::from(name.to_uppercase())),
        )
        .when(folded && inbox_count > 0, |d| {
            d.child(
                div()
                    .id(SharedString::from(format!("account-count-{id}")))
                    .test_support()
                    .child(count_badge(t, inbox_count, false)),
            )
        })
        .child(
            div()
                .flex_none()
                .ml_1()
                .opacity(0.)
                .group_hover(group, |s| s.opacity(1.))
                .child(Icon::new(chevron).with_size(px(12.)).text_color(t.muted_foreground)),
        )
        .into_any_element()
}

/// Clickable, rounded pill shared by every location row: highlight, navigation. Active rows
/// get a soft fill and a thin `bar` at the left edge. Callers add `.test_support()` once the
/// pill is complete and wrap it with [`row`].
fn pill(t: &ThemeColor, loc: &Location, is_active: bool, bar: Hsla) -> Stateful<Div> {
    let hover = t.list_hover;
    div()
        .id(SharedString::from(location_id(loc)))
        .relative()
        .flex()
        .flex_1()
        .min_w_0()
        .items_center()
        .h_full()
        .pl(px(PAD))
        .pr_2()
        .rounded_md()
        .text_size(px(12.))
        .text_color(t.foreground)
        .when(is_active, |d| {
            d.bg(t.list_active).child(
                div()
                    .absolute()
                    .left_0()
                    .top(px(5.))
                    .bottom(px(5.))
                    .w(px(BAR))
                    .rounded_full()
                    .bg(bar),
            )
        })
        .when(!is_active, |d| d.hover(move |d| d.bg(hover)))
        .cursor_pointer()
        .on_click(run(ShowLocation { location: loc.clone() }))
}

/// One sidebar row: optional rails, then the pill.
fn row(rails: Option<Div>, pill: impl IntoElement) -> AnyElement {
    div()
        .flex()
        .flex_none()
        .h(px(ROW_H))
        .when_some(rails, |d, r| d.child(r))
        .child(pill)
        .into_any_element()
}

fn row_label(label: impl Into<SharedString>) -> Div {
    div().ml(px(6.)).flex_1().min_w_0().truncate().child(label.into())
}

/// One root location row; counts show only when non-zero, `bar` tints the active marker.
fn nav_row(
    t: &ThemeColor,
    loc: &Location,
    label: &str,
    active: &Location,
    count: Option<usize>,
    bar: Hsla,
) -> AnyElement {
    let is_active = active == loc;
    let (icon, color) = location_icon(loc, t, false);
    let pill = pill(t, loc, is_active, bar)
        .child(icon_slot(icon, color, None))
        .child(row_label(label.to_owned()))
        .when_some(count.filter(|&n| n > 0), |d, n| d.child(count_badge(t, n, is_active)))
        .test_support();
    row(None, pill)
}

/// What [`fold_rows`] needs to draw one account's folder tree.
struct FolderCtx<'a> {
    t: &'a ThemeColor,
    folders: &'a [&'a Folder],
    active: &'a Location,
    /// Path from the top-level folder to the active folder (see [`active_chain`]).
    chain: &'a [FolderId],
    collapsed: &'a HashSet<FolderId>,
    /// The account's colour, for the active marker.
    color: Hsla,
}

/// Push a folder and, unless it is folded away, its children one level deeper. `through`
/// holds the ancestors' pass-through flags (see [`Branch::through`]); top-level folders are
/// roots without rails, so nothing is pushed for them.
fn fold_rows(
    ctx: &FolderCtx,
    parent: Option<FolderId>,
    through: &mut Vec<(bool, bool)>,
    rows: &mut Vec<AnyElement>,
) {
    let siblings: Vec<&Folder> =
        ctx.folders.iter().copied().filter(|f| f.parent == parent).collect();
    // The path's folder among these siblings, when the parent is on the path.
    let path_child = parent
        .and_then(|p| ctx.chain.iter().position(|&c| c == p))
        .and_then(|i| ctx.chain.get(i + 1))
        .copied();
    let mut passed = false;
    for (i, folder) in siblings.iter().enumerate() {
        let last = i + 1 == siblings.len();
        let rel = if Some(folder.id) == path_child {
            passed = true;
            PathRel::On
        } else if path_child.is_some() && !passed {
            PathRel::Before
        } else {
            PathRel::Off
        };
        let has_children = ctx.folders.iter().any(|f| f.parent == Some(folder.id));
        let folded = ctx.collapsed.contains(&folder.id);
        let branch = Branch { through: &through[..], last, rel };
        rows.push(folder_row(ctx, folder, parent.is_some().then_some(branch), has_children, folded));
        if has_children && !folded {
            if parent.is_some() {
                through.push((!last, rel == PathRel::Before));
            }
            fold_rows(ctx, Some(folder.id), through, rows);
            if parent.is_some() {
                through.pop();
            }
        }
    }
}

/// A folder row: rails when it is a subfolder, icon, name and — with children — the fold caret.
fn folder_row(
    ctx: &FolderCtx,
    folder: &Folder,
    branch: Option<Branch>,
    has_children: bool,
    folded: bool,
) -> AnyElement {
    let t = ctx.t;
    let loc = Location::Folder(folder.id);
    let open = has_children && !folded;
    let (icon, color) = location_icon(&loc, t, open);
    // The stem is on the active path when the path continues below this folder.
    let on_path_below = ctx
        .chain
        .iter()
        .position(|&c| c == folder.id)
        .is_some_and(|i| i + 1 < ctx.chain.len());
    let stem_color = open.then(|| rail_color(t, on_path_below));
    let pill = pill(t, &loc, ctx.active == &loc, ctx.color)
        .child(icon_slot(icon, color, stem_color))
        .child(row_label(folder.name.clone()))
        .when(has_children, |d| d.child(caret_button(t, folder.id, folded)))
        .test_support();
    row(branch.map(|b| rails(t, b)), pill)
}
