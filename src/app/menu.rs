//! Popup menus: what a menu holds, handed to the kit's `PopupMenu` to draw and navigate.
//!
//! A menu holds no business logic: the root view describes its rows as [`MenuItem`]s, and
//! choosing a row hands its action to a `run` callback, so a click runs exactly what the
//! row's shortcut runs. The kit supplies the popover, anchoring, keyboard navigation
//! (arrows, enter, escape, left/right for submenus) and the shortcut keycap beside each
//! row, taken from the action's key binding.
use std::rc::Rc;

use gpui_kit::component::Side;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::*;

/// What choosing a row does: the action handed to the owner to dispatch.
pub type MenuRun = Rc<dyn Fn(Box<dyn Action>, &mut Window, &mut App)>;

/// Builds a row's action each time it is chosen, so rows can carry data (a filter, an id).
type ActionFn = Rc<dyn Fn() -> Box<dyn Action>>;

/// One row of a menu.
#[derive(Clone)]
pub enum MenuItem {
    /// Runs an action, exactly like its key binding.
    Action {
        label: SharedString,
        /// Shown with a check mark: the active filter choice.
        checked: bool,
        icon: Option<gpui_kit::assets::IconName>,
        action: ActionFn,
    },
    /// A non-interactive section header.
    Label(SharedString),
    /// Opens a nested menu.
    Submenu { label: SharedString, items: Vec<MenuItem> },
    Separator,
}

impl MenuItem {
    /// A row that runs `action`.
    pub fn action(label: impl Into<SharedString>, action: fn() -> Box<dyn Action>) -> Self {
        Self::action_fn(label, action)
    }
    pub fn action_fn(
        label: impl Into<SharedString>,
        action: impl Fn() -> Box<dyn Action> + 'static,
    ) -> Self {
        MenuItem::Action { label: label.into(), checked: false, icon: None, action: Rc::new(action) }
    }
    /// A non-interactive header for the rows under it.
    pub fn header(label: impl Into<SharedString>) -> Self {
        MenuItem::Label(label.into())
    }

    /// Mark an action row as the active choice.
    pub fn checked(mut self, on: bool) -> Self {
        if let MenuItem::Action { checked, .. } = &mut self {
            *checked = on;
        }
        self
    }
    /// Show an icon alongside an action row.
    pub fn icon(mut self, icon: gpui_kit::assets::IconName) -> Self {
        if let MenuItem::Action { icon: item_icon, .. } = &mut self {
            *item_icon = Some(icon);
        }
        self
    }

    /// A row that opens a nested menu.
    pub fn submenu(label: impl Into<SharedString>, items: Vec<MenuItem>) -> Self {
        MenuItem::Submenu { label: label.into(), items }
    }

    pub fn separator() -> Self {
        MenuItem::Separator
    }

    /// The row's label, if it has one.
    pub fn label(&self) -> Option<&SharedString> {
        match self {
            MenuItem::Action { label, .. }
            | MenuItem::Label(label)
            | MenuItem::Submenu { label, .. } => Some(label),
            MenuItem::Separator => None,
        }
    }
}

/// Fill `menu` with `items`; choosing an action row calls `run` with its action.
pub fn populate(
    menu: PopupMenu,
    items: &[MenuItem],
    run: &MenuRun,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    // A checked row draws its check at the row's end: a row that carries an icon (every
    // classifier label option does) fills the left slot with the icon instead.
    let menu = menu.check_side(Side::Right);
    items.iter().fold(menu, |menu, item| match item {
        MenuItem::Separator => menu.separator(),
        MenuItem::Label(label) => menu.label(label.clone()),
        MenuItem::Action { label, checked, icon, action } => {
            let (run, build) = (run.clone(), action.clone());
            // The click handler replaces the kit's own dispatch, so the owner can scope the
            // action; the action is still attached so the row shows its key binding.
            let item = PopupMenuItem::new(label.clone())
                .action(action())
                .checked(*checked)
                .on_click(move |_, window, cx| run(build(), window, cx));
            menu.item(if let Some(icon) = icon { item.icon(*icon) } else { item })
        }
        MenuItem::Submenu { label, items } => {
            let (items, run) = (items.clone(), run.clone());
            menu.submenu(label.clone(), window, cx, move |sub, window, cx| {
                populate(sub, &items, &run, window, cx)
            })
        }
    })
}
