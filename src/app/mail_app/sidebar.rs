//! The accounts/folders sidebar: every location the list can show, in one column.
//!
//! The kit's [`Sidebar`] is still the container — one scrolling column with its header — but
//! the rows are ours ([`SidebarRow`]): an *All Inboxes* row, then one foldable submenu per
//! account carrying its Inbox / Snoozed / Sent / Archive / Trash and its folder tree. Rows
//! dispatch [`ShowLocation`]; Inbox and Snoozed rows (and All Inboxes) carry a count when
//! non-zero.
//!
//! Ours, because a kit row cannot drag: a folder row carries a [`FolderDrag`] on the row
//! itself, so it can be dropped on the list strip to open its view. Folding is ours too — a
//! row with children owns its caret and its open state. The app owns the pane's width and
//! visibility through the resizable panel and `cmd-b`.
use gpui_kit::component::ActiveTheme as _;

use super::drag::FolderDragPreview;
use super::*;
use crate::dnd::FolderDrag;
use crate::theme::ThemeColor;
use gpui_kit::assets::IconName;
use gpui_kit::base::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dock::AnyDrag as DockDrag;
use gpui_kit::component::{
    sidebar::{Sidebar, SidebarCollapsible, SidebarItem},
    tag::Tag,
    Collapsible, Icon, Sizable as _,
};

/// Titlebar and list-header label for a location, e.g. `"All Inboxes"`, `"Work · Archive"`
/// or `"Projects/Northwind"`.
pub(super) fn location_label(loc: &Location, mailbox: &Mailbox) -> String {
    match loc {
        Location::AllInboxes => "All Inboxes".to_owned(),
        Location::Folder(id) => mailbox.folder_path(*id),
        loc => format!(
            "{} · {}",
            account_name(mailbox, &location_account(loc)),
            location_name(loc)
        ),
    }
}

/// The label a sidebar row shows for a location: the location's own name, with the account
/// left to the submenu around the row. [`location_label`] is the account-qualified form, for
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
        .map_or_else(|| id.to_owned(), |a| crate::account_style::display_name(a).to_owned())
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

/// A pill badge with a message count: a compact capsule in the theme's foreground with its
/// number in the surface colour. The inversion is what makes the count the strongest thing on
/// the row, and it reads the same way in a light and a dark theme.
fn count_badge(t: &ThemeColor, n: usize) -> Tag {
    Tag::custom(t.foreground, t.background, t.foreground)
        .small()
        .rounded_full()
        .flex_none()
        .font_semibold()
        .child(n.to_string())
}

/// What a sidebar row does when it is clicked: show the location in the list strip. The
/// plain click takes over the preview tab, and a cmd-click or a double-click pins the view,
/// so a second location does not push it away again.
fn click_to_show(loc: Location, cx: &Context<MailApp>) -> impl Fn(&ClickEvent, &mut Window, &mut App) + use<> {
    let app = cx.weak_entity();
    move |event: &ClickEvent, _window: &mut Window, cx: &mut App| {
        let pin = event.modifiers().platform || event.click_count() >= 2;
        app.update(cx, |app, cx| app.show_location(loc.clone(), pin, cx)).ok();
    }
}

/// A row's click handler, shared so a built row can be cloned into the container's list.
type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// A sidebar row: our own element, so a folder can carry a drag as well as a click. It keeps
/// the kit row's look — icon in the location's colour, the label, a count badge on the right,
/// the active highlight, and a caret that folds a row's children away.
///
/// One row is a whole submenu: [`SidebarRow::children`] nests other rows, and the row folds
/// them with a caret, the way the kit's `SidebarMenuItem` does. Unlike the kit row, the row
/// itself owns the drag, so a folder can be grabbed anywhere on it.
#[derive(Clone)]
pub(super) struct SidebarRow {
    id: ElementId,
    label: SharedString,
    icon: Icon,
    active: bool,
    count: usize,
    default_open: bool,
    collapsed: bool,
    children: Vec<SidebarRow>,
    on_click: Option<ClickHandler>,
    drag: Option<FolderDrag>,
}

impl SidebarRow {
    fn new(id: impl Into<ElementId>, label: impl Into<SharedString>, icon: Icon) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon,
            active: false,
            count: 0,
            default_open: false,
            collapsed: false,
            children: Vec::new(),
            on_click: None,
            drag: None,
        }
    }

    fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    fn count(mut self, count: usize) -> Self {
        self.count = count;
        self
    }

    fn default_open(mut self, open: bool) -> Self {
        self.default_open = open;
        self
    }

    fn children(mut self, children: Vec<Self>) -> Self {
        self.children = children;
        self
    }

    fn on_click(mut self, handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }

    /// Make the row draggable, carrying `drag` in the dock's [`AnyDrag`].
    fn draggable(mut self, drag: FolderDrag) -> Self {
        self.drag = Some(drag);
        self
    }
}

impl Collapsible for SidebarRow {
    fn collapsed(mut self, collapsed: bool) -> Self {
        self.collapsed = collapsed;
        self
    }

    fn is_collapsed(&self) -> bool {
        self.collapsed
    }
}

impl SidebarItem for SidebarRow {
    fn render(self, _id: impl Into<ElementId>, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self { id, label, icon, active, count, default_open, collapsed, children, on_click, drag } = self;
        let is_submenu = !children.is_empty();
        let open_state = is_submenu.then(|| window.use_keyed_state(id.clone(), cx, |_, _| default_open));
        let is_open = open_state.as_ref().is_some_and(|state| !collapsed && *state.read(cx));
        let (accent, accent_fg, border) =
            (cx.theme().sidebar_accent, cx.theme().sidebar_accent_foreground, cx.theme().sidebar_border);
        let radius = cx.theme().radius;
        let badge = (count > 0).then(|| count_badge(cx.theme(), count).into_any_element());
        let caret = open_state.clone().map(|state| {
            let caret_id = ElementId::NamedChild(std::sync::Arc::new(id.clone()), "caret".into());
            Button::new(caret_id)
                .xsmall()
                .ghost()
                .icon(Icon::new(IconName::ChevronRight).size_4().when(is_open, |this| {
                    this.rotate(percentage(90. / 360.))
                }))
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    state.update(cx, |open: &mut bool, cx| {
                        *open = !*open;
                        cx.notify();
                    });
                })
        });

        let mut row_line = div()
            .id(id.clone())
            .test_support()
            .w_full()
            .flex()
            .items_center()
            .gap_x_2()
            .px_2()
            .h_9()
            .rounded(radius)
            .text_sm()
            .when(count > 0 || active, |this| this.font_semibold())
            .when(!active, |this| {
                this.hover(|this| this.bg(accent.opacity(0.5)).text_color(accent_fg))
            })
            .when(active, |this| this.bg(accent).text_color(accent_fg))
            .child(icon.size_4())
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .gap_x_2()
                    .justify_between()
                    .overflow_x_hidden()
                    .child(div().flex_1().overflow_x_hidden().child(label))
                    .when_some(badge, |this, badge| this.child(badge)),
            )
            .when_some(caret, |this, caret| this.child(caret));
        if let Some(handler) = on_click {
            row_line = row_line.on_click(move |event, window, cx| handler(event, window, cx));
        }
        if let Some(folder) = drag {
            let label = SharedString::from(folder.label.clone());
            row_line = row_line.on_drag(DockDrag::new(folder), move |_drag: &DockDrag, _offset, _window, cx| {
                cx.new(|_| FolderDragPreview::new(label.clone()))
            });
        }
        let mut row = div().w_full().child(row_line);
        if is_open {
            row = row.child(
                div()
                    .flex()
                    .flex_col()
                    .border_l_1()
                    .border_color(border)
                    .gap_1()
                    .ml_3p5()
                    .pl_2p5()
                    .py_0p5()
                    .children(children.into_iter().map(|child| child.render(id.clone(), window, cx).into_any_element())),
            );
        }
        row
    }
}

/// The stable, readable id of a location's sidebar row.
fn row_id(loc: &Location) -> ElementId {
    match loc {
        Location::AllInboxes => "sidebar-all-inboxes".into(),
        Location::Inbox(a) => format!("sidebar-inbox-{a}").into(),
        Location::Snoozed(a) => format!("sidebar-snoozed-{a}").into(),
        Location::Sent(a) => format!("sidebar-sent-{a}").into(),
        Location::Archive(a) => format!("sidebar-archive-{a}").into(),
        Location::Trash(a) => format!("sidebar-trash-{a}").into(),
        Location::Folder(id) => format!("sidebar-folder-{id}").into(),
    }
}

impl MailApp {
    /// Account the `g`-prefix jumps land in: the current location's, else the first.
    pub(super) fn nav_account(&self) -> Option<AccountId> {
        match self.location() {
            Some(Location::Folder(id)) => self.mailbox.folder(id).map(|f| f.account.clone()),
            None | Some(Location::AllInboxes) => {
                self.mailbox.accounts().first().map(|a| a.id.clone())
            }
            Some(loc) => Some(location_account(&loc)).filter(|a| !a.is_empty()),
        }
    }

    /// The sidebar: All Inboxes, then per account a submenu of its locations and folder tree.
    pub(super) fn render_sidebar(&self, cx: &Context<Self>) -> AnyElement {
        let active = self.location();
        let mut items: Vec<SidebarRow> = vec![self.location_row(Location::AllInboxes, &active, cx)];
        for account in self.mailbox.accounts() {
            let mut children: Vec<SidebarRow> = [
                Location::Inbox(account.id.clone()),
                Location::Snoozed(account.id.clone()),
                Location::Sent(account.id.clone()),
                Location::Archive(account.id.clone()),
                Location::Trash(account.id.clone()),
            ]
            .into_iter()
            .map(|loc| self.location_row(loc, &active, cx))
            .collect();
            for folder in self.mailbox.folders(&account.id) {
                if folder.parent.is_none() {
                    children.push(self.folder_row(folder, &account.id, &active, cx));
                }
            }
            items.push(
                SidebarRow::new(
                    format!("sidebar-account-{}", account.id),
                    crate::account_style::display_name(account).to_owned(),
                    icons::account_icon(
                        crate::account_style::icon_key(account),
                        &account.color,
                        cx.theme(),
                        16.,
                    ),
                )
                .default_open(true)
                .children(children),
            );
        }
        let header = div()
            .flex()
            .w_full()
            .items_center()
            .justify_between()
            .child(
                div()
                    .text_xs()
                    .font_semibold()
                    .text_color(cx.theme().muted_foreground)
                    .child("Mailboxes"),
            )
            .child(
                div()
                    .flex()
                    .gap_1()
                    .when(self.has_remote_accounts(), |row| {
                        row.child(
                            icon_button("sidebar-fetch-mail", IconName::RefreshCw, "Fetch mail", "cmd-shift-n", cx)
                                .loading(self.is_fetching())
                                .loading_icon(IconName::RefreshCw)
                                .on_click(run(FetchMail)),
                        )
                    })
                    .child(
                        icon_button("sidebar-add-account", IconName::Plus, "Add or manage accounts", "", cx)
                            .on_click(run(ManageAccounts)),
                    ),
            );
        Sidebar::<SidebarRow>::new("sidebar")
            .w_full()
            .collapsible(SidebarCollapsible::None)
            .header(header)
            .children(items)
            .into_any_element()
    }

    /// One location row: its icon in the location's colour, active when it is the view, a count
    /// suffix for All Inboxes / Inbox / Snoozed when non-zero. Clicking shows the location.
    fn location_row(&self, loc: Location, active: &Option<Location>, cx: &Context<Self>) -> SidebarRow {
        let (icon, color) = location_icon(&loc, cx.theme(), false);
        let is_active = *active == Some(loc.clone());
        let counted =
            matches!(loc, Location::AllInboxes | Location::Inbox(_) | Location::Snoozed(_));
        let count = if counted { self.mailbox.count_at(&loc, &self.local_now()) } else { 0 };
        SidebarRow::new(row_id(&loc), row_label(&loc, &self.mailbox), Icon::new(icon).text_color(color))
            .active(is_active)
            .count(count)
            .on_click(click_to_show(loc.clone(), cx))
    }

    /// One folder row: an item whose children are its subfolders, foldable with its caret and
    /// draggable anywhere on the row.
    fn folder_row(
        &self,
        folder: &Folder,
        account: &AccountId,
        active: &Option<Location>,
        cx: &Context<Self>,
    ) -> SidebarRow {
        let loc = Location::Folder(folder.id);
        let subfolders: Vec<&Folder> = self
            .mailbox
            .folders(account)
            .into_iter()
            .filter(|f| f.parent == Some(folder.id))
            .collect();
        let (icon, color) = location_icon(&loc, cx.theme(), !subfolders.is_empty());
        let mut row = SidebarRow::new(row_id(&loc), folder.name.clone(), Icon::new(icon).text_color(color))
            .active(*active == Some(loc.clone()))
            .draggable(FolderDrag { location: loc.clone(), label: folder.name.clone() })
            .on_click(click_to_show(loc.clone(), cx));
        if !subfolders.is_empty() {
            row = row.default_open(true).children(
                subfolders
                    .into_iter()
                    .map(|f| self.folder_row(f, account, active, cx))
                    .collect::<Vec<_>>(),
            );
        }
        row
    }
}
