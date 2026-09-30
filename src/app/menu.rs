//! Popup menus for the quiet header and the contextual action bar.
//!
//! A menu holds no business logic: it renders a list of [`MenuItem`]s, moves a
//! selection with the keyboard, and hands the chosen action back through
//! [`MenuEvent`], so choosing a row runs exactly what its shortcut runs.

use std::rc::Rc;

use gpui_kit::{prelude::FluentBuilder as _, *};

use crate::theme::{self, Theme};

/// Key context of an open menu (arrows move, enter runs, escape dismisses).
pub const MENU_CONTEXT: &str = "MailMenu";

/// Width of a menu panel; its trigger anchors it, so the geometry needs the number.
pub const MENU_W: f32 = 224.;

gpui_kit::actions!(
    menu,
    [MenuNext, MenuPrev, MenuBack, MenuRun, MenuOpen, MenuCancel]
);

/// One row of a menu.
#[derive(Clone)]
pub enum MenuItem {
    /// Runs an action, exactly like its key binding.
    Action {
        id: SharedString,
        label: SharedString,
        /// Shortcut hint shown right-aligned, e.g. `"shift-e"`.
        key: &'static str,
        /// Builds the action on click; a closure so rows can carry data (a filter, an id).
        action: Rc<dyn Fn() -> Box<dyn Action>>,
    },
    /// Opens a nested menu.
    Submenu {
        id: SharedString,
        label: SharedString,
        items: Vec<MenuItem>,
    },
    Separator,
}

impl MenuItem {
    /// A row that runs `action`.
    pub fn action(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        key: &'static str,
        action: fn() -> Box<dyn Action>,
    ) -> Self {
        MenuItem::Action {
            id: id.into(),
            label: label.into(),
            key,
            action: Rc::new(action),
        }
    }

    /// A row whose action is built from data the row holds, e.g. a tag to toggle.
    pub fn action_fn(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        key: &'static str,
        action: impl Fn() -> Box<dyn Action> + 'static,
    ) -> Self {
        MenuItem::Action {
            id: id.into(),
            label: label.into(),
            key,
            action: Rc::new(action),
        }
    }

    /// A row that opens a nested menu.
    pub fn submenu(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        items: Vec<MenuItem>,
    ) -> Self {
        MenuItem::Submenu {
            id: id.into(),
            label: label.into(),
            items,
        }
    }

    pub fn separator() -> Self {
        MenuItem::Separator
    }

    fn is_selectable(&self) -> bool {
        matches!(self, MenuItem::Action { .. } | MenuItem::Submenu { .. })
    }
}

/// Index of the first row the keyboard can land on, so the highlight starts somewhere real.
fn first_selectable(items: &[MenuItem]) -> usize {
    items.iter().position(MenuItem::is_selectable).unwrap_or(0)
}

/// What a menu asks the app to do once it closes.
pub enum MenuEvent {
    /// Run this action on the app's own focus path, like the shortcut would.
    Run(Box<dyn Action>),
    /// Dismissed without choosing anything.
    Cancel,
}

/// One level of the menu stack.
struct Level {
    items: Vec<MenuItem>,
    /// Shown as a heading while a submenu is open.
    title: Option<SharedString>,
}

pub struct MenuPanel {
    focus: FocusHandle,
    levels: Vec<Level>,
    selected: usize,
}

impl MenuPanel {
    pub fn new(items: Vec<MenuItem>, cx: &mut Context<Self>) -> Self {
        let selected = first_selectable(&items);
        Self {
            focus: cx.focus_handle(),
            levels: vec![Level {
                items,
                title: None,
            }],
            selected,
        }
    }

    /// A submenu is open, so the panel shows a back row.
    fn nested(&self) -> bool {
        self.levels.len() > 1
    }

    fn level(&self) -> &Level {
        self.levels.last().expect("a menu always has a level")
    }

    /// Move the selection by `delta` rows, skipping separators and labels.
    fn move_selection(&mut self, delta: isize) {
        let len = self.level().items.len();
        if len == 0 {
            return;
        }
        let mut ix = self.selected as isize;
        for _ in 0..len {
            ix = (ix + delta).rem_euclid(len as isize);
            if self.level().items[ix as usize].is_selectable() {
                break;
            }
        }
        self.selected = ix as usize;
    }

    /// Run the selected row, or open it when it is a submenu.
    fn activate(&mut self, cx: &mut Context<Self>) {
        match self.level().items.get(self.selected).cloned() {
            Some(MenuItem::Action { action, .. }) => cx.emit(MenuEvent::Run(action())),
            Some(MenuItem::Submenu { label, items, .. }) => {
                let selected = first_selectable(&items);
                self.levels.push(Level {
                    items,
                    title: Some(label),
                });
                self.selected = selected;
                cx.notify();
            }
            Some(MenuItem::Separator) | None => {}
        }
    }

    /// `right`: open the selected submenu, as Enter does; nothing on a plain row.
    fn open_selected(&mut self, cx: &mut Context<Self>) {
        if matches!(
            self.level().items.get(self.selected),
            Some(MenuItem::Submenu { .. })
        ) {
            self.activate(cx);
        }
    }

    /// Pop one level; a no-op at the top level.
    fn back(&mut self, cx: &mut Context<Self>) {
        if self.levels.len() > 1 {
            self.levels.pop();
            self.selected = first_selectable(&self.level().items);
            cx.notify();
        }
    }
}

impl Focusable for MenuPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl EventEmitter<MenuEvent> for MenuPanel {}

/// A menu row: label left, shortcut hint right.
fn row(t: &Theme, label: SharedString, key: &str, selected: bool) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .h(px(24.))
        .px_2()
        .rounded_sm()
        .text_size(px(12.))
        .text_color(t.text)
        .when(selected, |d| d.bg(t.selection))
        .child(div().flex_1().truncate().child(label))
        .child(
            div()
                .flex_none()
                .text_size(px(11.))
                .text_color(t.text_muted)
                .child(SharedString::from(key.to_owned())),
        )
}

impl Render for MenuPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::active(cx);
        let selected = self.selected;
        let rows = self.level().items.clone();
        let title = self.level().title.clone();
        let nested = self.nested();
        let hover = t.hover;
        let muted = t.text_muted;
        let border = t.border;
        div()
            .id("menu-panel")
            .test_support()
            .key_context(MENU_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &MenuNext, _, cx| {
                this.move_selection(1);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &MenuPrev, _, cx| {
                this.move_selection(-1);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &MenuRun, _, cx| this.activate(cx)))
            .on_action(cx.listener(|this, _: &MenuBack, _, cx| this.back(cx)))
            .on_action(cx.listener(|this, _: &MenuOpen, _, cx| this.open_selected(cx)))
            .on_action(cx.listener(|_, _: &MenuCancel, _, cx| {
                cx.emit(MenuEvent::Cancel);
            }))
            .w(px(MENU_W))
            .max_h(px(360.))
            .overflow_y_scroll()
            .p_1()
            .flex()
            .flex_col()
            .bg(t.surface)
            .border_1()
            .border_color(border)
            .rounded_md()
            .when(nested, |d| {
                d.child(
                    row(&t, "‹ Back".into(), "left", false)
                        .id("menu-back")
                        .test_support()
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| this.back(cx))),
                )
            })
            .when_some(title, |d, title| {
                d.child(
                    div()
                        .px_2()
                        .pt_1()
                        .pb(px(2.))
                        .text_size(px(10.))
                        .text_color(muted)
                        .child(title),
                )
            })
            .children(rows.into_iter().enumerate().map(|(ix, item)| {
                let is_selected = selected == ix;
                match item {
                    MenuItem::Separator => div().h(px(1.)).my_1().bg(border).into_any_element(),
                    MenuItem::Action { id, label, key, action } => row(&t, label, key, is_selected)
                        .id(id)
                        .test_support()
                        .cursor_pointer()
                        .when(!is_selected, |d| d.hover(move |d| d.bg(hover)))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(MenuEvent::Run((action)()))
                        }))
                        .into_any_element(),
                    MenuItem::Submenu { id, label, .. } => row(&t, label, "right", is_selected)
                        .id(id)
                        .test_support()
                        .cursor_pointer()
                        .when(!is_selected, |d| d.hover(move |d| d.bg(hover)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.selected = ix;
                            this.activate(cx);
                        }))
                        .into_any_element(),
                }
            }))
            .into_any_element()
    }
}
