//! The sidebar's folder panel: the part of the sidebar beside the account rail (see
//! [`super::rail`]), showing one account's locations at a time.
//!
//! Top to bottom it is a title (the account, or *All Inboxes*), a grid of tiles for the five
//! places mail lives (Inbox, Snoozed, Sent, Archive, Trash) plus a *new folder* tile, and the
//! folders. An account's view carries its whole folder tree; the All view carries the
//! cross-account tiles and then one section per account with its top-level folders.
//!
//! Which account the panel shows is [`MailApp::sidebar_account`]: it follows the list's
//! location, and a click on a rail tile overrides it until the list moves on. Tiles and
//! folder rows dispatch [`MailApp::show_location`] through [`click_to_show`].
//!
//! Folder rows are ours, because a kit row cannot drag: a folder row carries a [`FolderDrag`]
//! on the row itself, so it can be dropped on the list strip to open its view. Folding is ours
//! too, remembered per folder and per account section in `MailApp::sidebar_folds`. The app
//! owns the pane's width and visibility through the resizable panel and `cmd-b`.
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::menu::ContextMenuExt as _;

use super::drag::FolderDragPreview;
use super::*;
use crate::app::ui::key_tooltip;
use crate::badge::{compact_count, tile_badge};
use crate::dnd::FolderDrag;
use gpui_kit::base::StyledExt as _;
use crate::model::Account;
use crate::theme::ThemeColor;
use gpui_kit::assets::IconName;
use gpui_kit::component::dock::AnyDrag as DockDrag;
use gpui_kit::component::{Icon, Sizable as _};

/// Titlebar and list-header label for a location, e.g. `"All Inboxes"`, `"Work · Archive"`
/// or `"Projects/Northwind"`.
pub(super) fn location_label(loc: &Location, mailbox: &Mailbox) -> String {
    match loc {
        Location::AllInboxes
        | Location::AllSnoozed
        | Location::AllSent
        | Location::AllArchive
        | Location::AllTrash => location_name(loc).to_owned(),
        Location::Folder(id) => mailbox.folder_path(*id),
        loc => format!(
            "{} · {}",
            account_name(mailbox, &location_account(loc)),
            location_name(loc)
        ),
    }
}

/// The label a sidebar row shows for a location: the location's own name, with the account
/// left to the panel around the row. [`location_label`] is the account-qualified form, for
/// headers; this is what the row — and so the list tab that opens it — reads.
pub(super) fn row_label(loc: &Location, mailbox: &Mailbox) -> String {
    match loc {
        Location::Folder(id) => {
            mailbox.folder(*id).map_or_else(|| location_name(loc).to_owned(), |folder| folder.name.clone())
        }
        loc => location_name(loc).to_owned(),
    }
}

/// Name of a location inside one account, e.g. `"Inbox"`.
fn location_name(loc: &Location) -> &'static str {
    match loc {
        Location::AllInboxes => "All Inboxes",
        Location::AllSnoozed => "All Snoozed",
        Location::AllSent => "All Sent",
        Location::AllArchive => "All Archive",
        Location::AllTrash => "All Trash",
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
        Location::AllInboxes
        | Location::AllSnoozed
        | Location::AllSent
        | Location::AllArchive
        | Location::AllTrash
        | Location::Folder(_) => AccountId::new(),
    }
}

fn account_name(mailbox: &Mailbox, id: &str) -> String {
    mailbox
        .account(id)
        .map_or_else(|| id.to_owned(), |a| crate::account_style::display_name(a).to_owned())
}

/// Icon and tint of a location.
fn location_icon(loc: &Location, t: &ThemeColor) -> (IconName, Hsla) {
    match loc {
        Location::AllInboxes => (IconName::Mails, t.primary),
        Location::Inbox(_) => (IconName::Inbox, theme::inbox(t)),
        Location::Snoozed(_) | Location::AllSnoozed => (IconName::AlarmClock, theme::snoozed(t)),
        Location::Sent(_) | Location::AllSent => (IconName::Send, t.info),
        Location::Archive(_) | Location::AllArchive => (IconName::Archive, theme::archived(t)),
        Location::Trash(_) | Location::AllTrash => (IconName::Trash, theme::deleted(t)),
        Location::Folder(_) => (IconName::Folder, theme::filed(t)),
    }
}

/// The rail's background: the sidebar colour with 20% black over it, one step below the panel
/// in a light theme as well as a dark one, so no theme needs a token of its own for it.
pub(super) fn rail_bg(t: &ThemeColor) -> Hsla {
    t.sidebar.blend(black().opacity(0.2))
}

/// Text that stands out from the sidebar's normal text: the foreground pushed halfway to white
/// on a dark theme and to black on a light one. Row text turns to it while a folder has unread.
fn emphasis(t: &ThemeColor) -> Hsla {
    let c = t.sidebar_foreground;
    Hsla { l: if c.l > 0.5 { c.l + (1. - c.l) * 0.5 } else { c.l * 0.5 }, ..c }
}

/// The pill every count badge is made of: the foreground colour with its number in the sidebar
/// colour. The inversion is what makes a count the strongest thing around it, and it reads the
/// same way in a light and a dark theme.
fn pill_base(t: &ThemeColor, text: String) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(8.))
        .bg(t.foreground)
        .text_color(t.sidebar)
        .text_size(px(10.))
        .font_weight(FontWeight::BOLD)
        .line_height(relative(1.))
        .child(text)
}

/// A folder row's unread pill, inline at the end of the row.
fn row_pill(t: &ThemeColor, count: usize) -> Div {
    pill_base(t, compact_count(count)).min_w(px(18.)).h(px(16.)).px(px(5.))
}

/// A count badge on a tile's top-right corner, ringed in `ring` (the colour behind the tile) so
/// it cuts out of the tile. `right` is how far it hangs past the tile's edge. The ring sits
/// outside a 16px-high pill, as the design has it, so the badge is 20px tall.
fn corner_badge(t: &ThemeColor, text: String, ring: Hsla, right: f32) -> Div {
    pill_base(t, text)
        .absolute()
        .top(px(-5.))
        .right(px(-right))
        .min_w(px(30.))
        .h(px(20.))
        .px(px(5.))
        .border_2()
        .border_color(ring)
}

/// What a sidebar tile or row does when it is clicked: show the location in the list strip. The
/// plain click takes over the preview tab, and a cmd-click or a double-click pins the view,
/// so a second location does not push it away again.
fn click_to_show(loc: Location, cx: &Context<MailApp>) -> impl Fn(&ClickEvent, &mut Window, &mut App) + use<> {
    let app = cx.weak_entity();
    move |event: &ClickEvent, _window: &mut Window, cx: &mut App| {
        let pin = event.modifiers().platform || event.click_count() >= 2;
        app.update(cx, |app, cx| app.show_location_in_panel(loc.clone(), pin, cx)).ok();
    }
}

/// The stable, readable id of a location's sidebar tile or row.
fn row_id(loc: &Location) -> ElementId {
    match loc {
        Location::AllInboxes => "sidebar-all-inboxes".into(),
        Location::AllSnoozed => "sidebar-all-snoozed".into(),
        Location::AllSent => "sidebar-all-sent".into(),
        Location::AllArchive => "sidebar-all-archive".into(),
        Location::AllTrash => "sidebar-all-trash".into(),
        Location::Inbox(a) => format!("sidebar-inbox-{a}").into(),
        Location::Snoozed(a) => format!("sidebar-snoozed-{a}").into(),
        Location::Sent(a) => format!("sidebar-sent-{a}").into(),
        Location::Archive(a) => format!("sidebar-archive-{a}").into(),
        Location::Trash(a) => format!("sidebar-trash-{a}").into(),
        Location::Folder(id) => format!("sidebar-folder-{id}").into(),
    }
}

/// The key a folder's fold state is stored under. A folder folds on its own in each view: the
/// account view opens parents by default, the All view keeps them closed.
pub(super) fn folder_fold_key(id: FolderId, in_all: bool) -> String {
    format!("{}-folder-{id}", if in_all { "all" } else { "account" })
}

impl MailApp {
    /// The account the panel shows (`None` is All): the rail tile picked in the active view, else
    /// the account the active view belongs to. An account that is gone reads as All.
    pub(super) fn sidebar_account(&self) -> Option<AccountId> {
        let here = self.folder();
        let account = match &self.sidebar_pick {
            Some((picked, at)) if at == here => picked.clone(),
            _ => match here {
                Location::Folder(id) => self.mailbox.folder(*id).map(|f| f.account.clone()),
                loc => Some(location_account(loc)).filter(|a| !a.is_empty()),
            },
        };
        account.filter(|a| self.mailbox.account(a).is_some())
    }

    /// Account the `g`-prefix jumps land in: the one the panel shows, else the first.
    pub(super) fn nav_account(&self) -> Option<AccountId> {
        self.sidebar_account().or_else(|| self.mailbox.accounts().first().map(|a| a.id.clone()))
    }

    /// A rail tile was clicked: show `account` (`None` for All) in the panel. The list stays on
    /// its location; the panel follows it again as soon as the list moves.
    pub(super) fn pick_sidebar_account(&mut self, account: Option<AccountId>, cx: &mut Context<Self>) {
        self.sidebar_pick = Some((account, self.folder().clone()));
        cx.notify();
    }

    /// Show `location` from a sidebar click or drop: the panel stays on the account it showed, so
    /// a folder opened from the All view does not jump the panel to that folder's account.
    pub(super) fn show_location_in_panel(&mut self, location: Location, pin: bool, cx: &mut Context<Self>) {
        let panel = self.sidebar_account();
        self.show_location(location, pin, cx);
        self.pick_sidebar_account(panel, cx);
    }

    /// Whether the folder or section stored under `key` is open, `default` if never toggled.
    fn fold_open(&self, key: &str, default: bool) -> bool {
        self.sidebar_folds.get(key).copied().unwrap_or(default)
    }

    /// A click handler that flips the fold stored under `key`, from `open` as drawn.
    fn toggle_fold(
        &self,
        key: String,
        open: bool,
        cx: &Context<Self>,
    ) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
        cx.listener(move |this, _: &ClickEvent, _, cx| {
            // The chevron sits inside a row that opens the folder: this click is only the fold.
            cx.stop_propagation();
            this.sidebar_folds.insert(key.clone(), !open);
            cx.notify();
        })
    }

    /// The sidebar: the account rail, then the folder panel.
    pub(super) fn render_sidebar(&self, cx: &Context<Self>) -> AnyElement {
        div()
            .w_full()
            .flex_1()
            .min_h_0()
            .flex()
            .child(self.render_rail(cx))
            .child(self.render_panel(cx))
            .into_any_element()
    }

    /// The panel: title, tile grid, then the folders of the account (or one section per account).
    fn render_panel(&self, cx: &Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        let account = self.sidebar_account().and_then(|id| self.mailbox.account(&id).cloned());
        let active = self.location();
        let unread = self.mailbox.folder_unread();
        let (title_icon, title) = match &account {
            Some(a) => (
                icons::account_icon(crate::account_style::icon_key(a), &a.color, t, 16.),
                crate::account_style::display_name(a).to_owned(),
            ),
            None => (Icon::new(IconName::Mails).with_size(px(16.)).text_color(t.primary), "All Inboxes".to_owned()),
        };
        let places: [(Location, &str); 5] = match &account {
            Some(a) => [
                (Location::Inbox(a.id.clone()), "i"),
                (Location::Snoozed(a.id.clone()), "s"),
                (Location::Sent(a.id.clone()), "t"),
                (Location::Archive(a.id.clone()), "a"),
                (Location::Trash(a.id.clone()), "d"),
            ],
            None => [
                (Location::AllInboxes, ""),
                (Location::AllSnoozed, ""),
                (Location::AllSent, ""),
                (Location::AllArchive, ""),
                (Location::AllTrash, ""),
            ],
        };
        let new_folder_for = account.as_ref().map(|a| a.id.clone());
        let new_folder = key_tooltip(div().id("sidebar-new-folder"), "New folder", "")
            .test_support()
            .h(px(44.))
            .rounded(px(10.))
            .border(px(1.5))
            .border_dashed()
            .border_color(t.sidebar_border)
            .flex()
            .items_center()
            .justify_center()
            .text_color(t.muted_foreground)
            .hover(|s| s.bg(t.secondary).text_color(t.foreground))
            .child(Icon::new(IconName::FolderPlus).with_size(px(18.)))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.new_folder(new_folder_for.clone(), window, cx);
            }));
        let grid = div()
            .grid()
            .grid_cols(3)
            .gap(px(6.))
            .pt(px(4.))
            .pb(px(6.))
            .children(places.into_iter().map(|(loc, key)| self.location_tile(loc, key, &active, cx)))
            .child(new_folder);
        let body: Vec<AnyElement> = match &account {
            Some(a) => self.folder_tree(&a.id, None, false, &unread, &active, cx),
            None => self
                .mailbox
                .accounts()
                .iter()
                .flat_map(|a| self.account_section(a, &unread, &active, cx))
                .collect(),
        };
        div()
            .id("sidebar-panel")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .pt(px(12.))
            .px(px(8.))
            .pb(px(8.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(8.))
                    .pb(px(8.))
                    .text_size(px(14.))
                    .font_semibold()
                    .text_color(emphasis(t))
                    .child(title_icon)
                    .child(div().min_w_0().truncate().child(title)),
            )
            .child(grid)
            .children(body)
    }

    /// One tile of the grid: the location's icon in its colour, a count badge on Inbox and
    /// Snoozed when there is mail, the active fill while the list shows it. `key` is the letter
    /// of the `g` jump that lands here, shown in the tooltip, or empty.
    fn location_tile(&self, loc: Location, key: &str, active: &Option<Location>, cx: &Context<Self>) -> AnyElement {
        let t = cx.theme();
        let (icon, color) = location_icon(&loc, t);
        let is_active = *active == Some(loc.clone());
        let counted = matches!(
            loc,
            Location::AllInboxes | Location::AllSnoozed | Location::Inbox(_) | Location::Snoozed(_)
        );
        let count = if counted { self.mailbox.count_at(&loc, &self.local_now()) } else { 0 };
        let mut tip = row_label(&loc, &self.mailbox);
        if !key.is_empty() {
            tip = format!("{tip} · g {key}");
        }
        key_tooltip(div().id(row_id(&loc)), &tip, "")
            .test_support()
            .relative()
            .h(px(44.))
            .rounded(px(10.))
            .bg(if is_active { t.secondary_active } else { t.secondary })
            .when(!is_active, |d| d.hover(|s| s.bg(t.secondary_hover)))
            .flex()
            .items_center()
            .justify_center()
            .child(Icon::new(icon).with_size(px(18.)).text_color(color))
            .when_some(tile_badge(count), |d, text| d.child(corner_badge(t, text, t.sidebar, 4.)))
            .on_click(click_to_show(loc, cx))
            .into_any_element()
    }

    /// The folders of `account` whose parent is `parent`, each followed by its children when
    /// open. In the All view (`in_all`) the account's parents start closed.
    fn folder_tree(
        &self,
        account: &AccountId,
        parent: Option<FolderId>,
        in_all: bool,
        unread: &HashMap<FolderId, usize>,
        active: &Option<Location>,
        cx: &Context<Self>,
    ) -> Vec<AnyElement> {
        let folders = self.mailbox.folders(account);
        folders
            .iter()
            .filter(|f| f.parent == parent)
            .map(|folder| {
                let has_kids = folders.iter().any(|c| c.parent == Some(folder.id));
                let key = folder_fold_key(folder.id, in_all);
                let open = has_kids && self.fold_open(&key, !in_all);
                let loc = Location::Folder(folder.id);
                let row = self.folder_row(
                    folder,
                    has_kids.then_some((key, open)),
                    unread.get(&folder.id).copied().unwrap_or(0),
                    *active == Some(loc),
                    cx,
                );
                if !open {
                    return row;
                }
                let kids = self.folder_tree(account, Some(folder.id), in_all, unread, active, cx);
                div()
                    .child(row)
                    .child(
                        div()
                            .ml(px(15.))
                            .pl(px(4.))
                            .border_l_1()
                            .border_color(cx.theme().sidebar_border)
                            .children(kids),
                    )
                    .into_any_element()
            })
            .collect()
    }

    /// One folder row: its icon, name and unread pill. `fold` is the key and open state of its
    /// chevron when it has subfolders. The row is draggable anywhere, and clicking shows it.
    fn folder_row(
        &self,
        folder: &Folder,
        fold: Option<(String, bool)>,
        unread: usize,
        active: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let t = cx.theme();
        let loc = Location::Folder(folder.id);
        let (icon, color) = location_icon(&loc, t);
        let chevron = fold.map(|(key, open)| {
            div()
                .id(format!("sidebar-fold-{key}"))
                .flex_none()
                .test_support()
                .ml(px(-2.))
                .mr(px(-4.))
                .child(
                    Icon::new(if open { IconName::ChevronDown } else { IconName::ChevronRight })
                        .with_size(px(12.))
                        .text_color(t.muted_foreground),
                )
                .on_click(self.toggle_fold(key, open, cx))
        });
        let drag = FolderDrag { location: loc.clone(), label: folder.name.clone() };
        let preview_label = SharedString::from(drag.label.clone());
        let menu = self.context_menu(MenuKind::Folder(folder.id), cx);
        div()
            .id(row_id(&loc))
            .test_support()
            .w_full()
            .flex()
            .items_center()
            .gap(px(8.))
            .h(px(28.))
            .px(px(8.))
            .rounded(px(6.))
            .text_size(px(13.))
            .text_color(if unread > 0 { emphasis(t) } else { t.sidebar_foreground })
            .when(unread > 0, |d| d.font_semibold())
            .when(active, |d| d.bg(t.secondary_active))
            .when(!active, |d| d.hover(|s| s.bg(t.secondary_hover.opacity(0.6))))
            .children(chevron)
            .child(Icon::new(icon).with_size(px(16.)).text_color(color).flex_none())
            // The label shrinks and truncates to one line when the pane is thin; min_w_0 is
            // what lets it shrink below its text.
            .child(div().flex_1().min_w_0().truncate().child(folder.name.clone()))
            .when(unread > 0, |d| d.child(row_pill(t, unread)))
            .on_click(click_to_show(loc, cx))
            .on_drag(DockDrag::new(drag), move |_drag: &DockDrag, _offset, _window, cx| {
                cx.new(|_| FolderDragPreview::new(preview_label.clone()))
            })
            .context_menu(menu)
            .into_any_element()
    }

    /// The All view's section for one account: its header, which folds the section, then its
    /// top-level folders with their parents closed.
    fn account_section(
        &self,
        account: &Account,
        unread: &HashMap<FolderId, usize>,
        active: &Option<Location>,
        cx: &Context<Self>,
    ) -> Vec<AnyElement> {
        let t = cx.theme();
        let key = format!("section-{}", account.id);
        let open = self.fold_open(&key, true);
        let header = div()
            .id(format!("sidebar-section-{}", account.id))
            .group("sidebar-section")
            .test_support()
            .flex()
            .items_center()
            .gap(px(8.))
            .h(px(28.))
            .px(px(8.))
            .mt(px(12.))
            .text_color(t.muted_foreground)
            .child(icons::account_icon(crate::account_style::icon_key(account), &account.color, t, 16.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(11.))
                    .font_semibold()
                    .child(crate::account_style::display_name(account).to_uppercase()),
            )
            .child(
                div()
                    .flex_none()
                    .opacity(0.)
                    .group_hover("sidebar-section", |s| s.opacity(1.))
                    .child(
                        Icon::new(if open { IconName::ChevronDown } else { IconName::ChevronRight })
                            .with_size(px(14.)),
                    ),
            )
            .on_click(self.toggle_fold(key, open, cx));
        let mut out = vec![header.into_any_element()];
        if open {
            out.extend(self.folder_tree(&account.id, None, true, unread, active, cx));
        }
        out
    }
}
